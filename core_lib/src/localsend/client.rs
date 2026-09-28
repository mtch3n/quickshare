//! Sending to a LocalSend peer.

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use anyhow::anyhow;
use futures::StreamExt;
use reqwest::StatusCode;
use tokio_util::io::ReaderStream;
use tokio_util::sync::CancellationToken;

use super::{
    API, FileInfo, PROGRESS_INTERVAL, PrepareUpload, PrepareUploadResponse, Shared, random_id,
    text_type, watch_cancel,
};
use crate::channel::TransferType;
use crate::hdl::info::TransferMetadata;
use crate::hdl::{OutboundPayload, State};
use crate::manager::SendInfo;
use crate::utils::{DeviceType, RemoteDeviceInfo, expand_directories};

/// A file to upload, by its id in the file list.
struct Upload {
    path: String,
    size: u64,
}

/// The file list announced to the receiver, and what to upload for each id.
#[derive(Default)]
struct FileList {
    files: HashMap<String, FileInfo>,
    uploads: HashMap<String, Upload>,
}

pub async fn send(shared: Arc<Shared>, info: SendInfo, https: bool) {
    let id = info.id.clone();
    let FileList { files, uploads } = match file_list(&info.ob) {
        Ok(list) => list,
        Err(e) => {
            warn!("LocalSend: can't send to {}: {e}", info.name);
            let meta = TransferMetadata {
                reason: Some(e.to_string()),
                ..Default::default()
            };
            shared.emit(&id, TransferType::Outbound, State::Disconnected, &meta);
            return;
        }
    };

    let text = match &info.ob {
        OutboundPayload::Text(text) => Some(text.clone()),
        OutboundPayload::Files(_) => None,
    };
    let mut meta = TransferMetadata {
        source: Some(RemoteDeviceInfo {
            name: info.name.clone(),
            device_type: DeviceType::Unknown,
            fingerprint: None,
        }),
        files: text
            .is_none()
            .then(|| files.values().map(|f| f.file_name.clone()).collect()),
        text_type: text.as_deref().map(text_type),
        text_payload: text,
        total_bytes: uploads.values().map(|u| u.size).sum(),
        ..Default::default()
    };

    let cancelled = CancellationToken::new();
    let done = CancellationToken::new();
    watch_cancel(
        shared.sender.subscribe(),
        id.clone(),
        cancelled.clone(),
        done.clone(),
    );

    let scheme = if https { "https" } else { "http" };
    // Discovery names LocalSend devices after their fingerprint.
    let fingerprint = id.strip_prefix("ls-").unwrap_or_default();
    let http = match shared.client(fingerprint, https) {
        Ok(http) => http,
        Err(e) => {
            warn!("LocalSend: can't send to {}: {e:#}", info.name);
            done.cancel();
            shared.emit(&id, TransferType::Outbound, State::Disconnected, &meta);
            return;
        }
    };
    let peer = Peer {
        http,
        base: format!("{scheme}://{}{API}", info.addr),
        ip: info.addr.parse::<SocketAddr>().ok().map(|a| a.ip()),
        pin: info.pin.as_deref(),
    };
    let state = match transfer(&shared, &id, &peer, files, uploads, &mut meta, &cancelled).await {
        Ok(state) => state,
        Err(e) => {
            warn!("LocalSend: sending to {} failed: {e:#}", info.name);
            State::Disconnected
        }
    };
    done.cancel();
    shared.emit(&id, TransferType::Outbound, state, &meta);
}

fn file_list(payload: &OutboundPayload) -> Result<FileList, anyhow::Error> {
    let mut list = FileList::default();

    match payload {
        OutboundPayload::Text(text) => {
            let id = random_id();
            list.files.insert(
                id.clone(),
                FileInfo {
                    file_name: format!("{id}.txt"),
                    id,
                    size: text.len() as u64,
                    file_type: "text/plain".into(),
                    sha256: None,
                    preview: Some(text.clone()),
                },
            );
        }
        OutboundPayload::Files(paths) => {
            let expanded = expand_directories(paths);
            if expanded.is_empty() {
                return Err(anyhow!("there are no files to send, only empty folders"));
            }
            for (path, folder) in expanded {
                let name = Path::new(&path)
                    .file_name()
                    .ok_or_else(|| anyhow!("{path} has no file name"))?
                    .to_string_lossy()
                    .into_owned();
                let size = std::fs::metadata(&path)?.len();
                let id = random_id();
                list.files.insert(
                    id.clone(),
                    FileInfo {
                        id: id.clone(),
                        file_name: match folder {
                            Some(folder) => format!("{folder}/{name}"),
                            None => name,
                        },
                        size,
                        file_type: mime_guess::from_path(&path)
                            .first_or_octet_stream()
                            .to_string(),
                        sha256: None,
                        preview: None,
                    },
                );
                list.uploads.insert(id, Upload { path, size });
            }
        }
    }

    Ok(list)
}

/// Where a transfer goes.
struct Peer<'a> {
    /// Only talks to this device, over HTTPS.
    http: reqwest::Client,
    /// `https://ip:port/api/localsend/v2`
    base: String,
    /// Whose `/cancel` requests may cancel it.
    ip: Option<IpAddr>,
    pin: Option<&'a str>,
}

/// Lists a send in [`Shared::outbound`] while it runs.
struct Outbound<'a> {
    shared: &'a Shared,
    session_id: String,
}

impl Drop for Outbound<'_> {
    fn drop(&mut self) {
        self.shared
            .outbound
            .lock()
            .unwrap()
            .remove(&self.session_id);
    }
}

async fn transfer(
    shared: &Shared,
    id: &str,
    peer: &Peer<'_>,
    files: HashMap<String, FileInfo>,
    uploads: HashMap<String, Upload>,
    meta: &mut TransferMetadata,
    cancelled: &CancellationToken,
) -> Result<State, anyhow::Error> {
    let base = peer.base.as_str();
    shared.emit(id, TransferType::Outbound, State::SentIntroduction, meta);

    // Held open by the receiver until its user decides.
    let mut request = peer.http.post(format!("{base}/prepare-upload"));
    if let Some(pin) = peer.pin {
        request = request.query(&[("pin", pin)]);
    }
    let request = request
        .json(&PrepareUpload {
            info: shared.info(None),
            files,
        })
        .send();
    let response = tokio::select! {
        _ = cancelled.cancelled() => return Ok(State::Cancelled),
        response = request => response?,
    };
    let session: PrepareUploadResponse = match response.status() {
        StatusCode::OK => response.json().await?,
        StatusCode::NO_CONTENT => return Ok(State::Finished),
        StatusCode::FORBIDDEN => return Ok(State::Rejected),
        // Missing or wrong: the frontend asks for it and sends again.
        StatusCode::UNAUTHORIZED => return Ok(State::PinRequired),
        StatusCode::CONFLICT => {
            meta.reason = Some("Busy with another transfer".into());
            return Ok(State::Disconnected);
        }
        StatusCode::TOO_MANY_REQUESTS => {
            meta.reason = Some("Too many wrong PINs".into());
            return Ok(State::Disconnected);
        }
        status => return Err(anyhow!("prepare-upload answered {status}")),
    };

    let peer_cancelled = CancellationToken::new();
    let _outbound = peer.ip.map(|ip| {
        shared
            .outbound
            .lock()
            .unwrap()
            .insert(session.session_id.clone(), (ip, peer_cancelled.clone()));
        Outbound {
            shared,
            session_id: session.session_id.clone(),
        }
    });

    shared.emit(id, TransferType::Outbound, State::SendingFiles, meta);
    let sent = Arc::new(AtomicU64::new(0));
    let uploading = upload_all(peer, &session, &uploads, sent.clone());
    tokio::pin!(uploading);
    let mut ticker = tokio::time::interval(PROGRESS_INTERVAL);

    loop {
        tokio::select! {
            _ = peer_cancelled.cancelled() => return Ok(State::Cancelled),
            _ = cancelled.cancelled() => {
                let _ = peer
                    .http
                    .post(format!("{base}/cancel?sessionId={}", session.session_id))
                    .timeout(Duration::from_secs(3))
                    .send()
                    .await;
                return Ok(State::Cancelled);
            }
            result = &mut uploading => {
                result?;
                meta.ack_bytes = meta.total_bytes;
                return Ok(State::Finished);
            }
            _ = ticker.tick() => {
                meta.ack_bytes = sent.load(Ordering::Relaxed);
                shared.emit(id, TransferType::Outbound, State::SendingFiles, meta);
            }
        }
    }
}

/// Uploads the files the receiver asked for (it gets a token), one at a time.
async fn upload_all(
    peer: &Peer<'_>,
    session: &PrepareUploadResponse,
    uploads: &HashMap<String, Upload>,
    sent: Arc<AtomicU64>,
) -> Result<(), anyhow::Error> {
    for (file_id, token) in &session.files {
        let Some(upload) = uploads.get(file_id) else {
            continue;
        };
        let counter = sent.clone();
        let stream =
            ReaderStream::new(tokio::fs::File::open(&upload.path).await?).inspect(move |chunk| {
                if let Ok(chunk) = chunk {
                    counter.fetch_add(chunk.len() as u64, Ordering::Relaxed);
                }
            });
        let response = peer
            .http
            .post(format!("{}/upload", peer.base))
            .query(&[
                ("sessionId", session.session_id.as_str()),
                ("fileId", file_id.as_str()),
                ("token", token.as_str()),
            ])
            .header(reqwest::header::CONTENT_LENGTH, upload.size)
            .body(reqwest::Body::wrap_stream(stream))
            .send()
            .await?;
        if !response.status().is_success() {
            return Err(anyhow!("upload answered {}", response.status()));
        }
    }
    Ok(())
}
