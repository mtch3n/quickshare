//! The HTTPS server LocalSend peers register with and send files to.

use std::collections::{HashMap, HashSet};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::anyhow;
use axum::body::Body;
use axum::extract::{ConnectInfo, Query, State as AxumState};
use axum::http::StatusCode;
use axum::middleware::AddExtension;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Extension, Json, Router};
use axum_server::Handle;
use axum_server::accept::Accept;
use axum_server::tls_rustls::{RustlsAcceptor, RustlsConfig};
use futures::StreamExt;
use futures::future::BoxFuture;
use rustls::ServerConfig;
use serde::Deserialize;
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};
use tokio_rustls::server::TlsStream;
use tokio_util::sync::CancellationToken;
use tower_layer::Layer;

use super::{
    API, DeviceInfo, PROGRESS_INTERVAL, PrepareUpload, PrepareUploadResponse, Shared, message_text,
    next_action, random_id, text_type, tls, watch_cancel,
};
use crate::channel::{ChannelAction, TransferType};
use crate::hdl::State;
use crate::hdl::info::TransferMetadata;
use crate::utils::{
    RemoteDeviceInfo, create_unique_file, get_download_dir, sanitize_relative_path,
};

type SharedState = AxumState<Arc<Shared>>;

/// The fingerprint of the certificate the sender presented on this
/// connection, if any. The handshake proved it holds the certificate's key.
#[derive(Clone)]
struct SenderCert(Option<String>);

/// Completes the TLS handshake, then hands [`SenderCert`] to the handlers.
#[derive(Clone)]
struct SenderCertAcceptor(RustlsAcceptor);

impl<I, S> Accept<I, S> for SenderCertAcceptor
where
    I: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    S: Send + 'static,
{
    type Stream = TlsStream<I>;
    type Service = AddExtension<S, SenderCert>;
    type Future = BoxFuture<'static, std::io::Result<(Self::Stream, Self::Service)>>;

    fn accept(&self, stream: I, service: S) -> Self::Future {
        let acceptor = self.0.clone();
        Box::pin(async move {
            let (stream, service) = acceptor.accept(stream, service).await?;
            let cert = stream
                .get_ref()
                .1
                .peer_certificates()
                .and_then(|certs| certs.first())
                .map(|cert| tls::fingerprint(cert));
            Ok((stream, Extension(SenderCert(cert)).layer(service)))
        })
    }
}

/// How long a sender waits for the user to decide before we give up.
const CONSENT_TIMEOUT: Duration = Duration::from_secs(120);
/// How long an accepted session may go without receiving a byte.
const IDLE_TIMEOUT: Duration = Duration::from_secs(60);

/// An accepted incoming transfer, until its last file arrives.
pub struct Session {
    /// The transfer's id towards the frontend.
    id: String,
    remote: IpAddr,
    dir: PathBuf,
    files: HashMap<String, SessionFile>,
    remaining: Mutex<HashSet<String>>,
    meta: Mutex<TransferMetadata>,
    last_report: Mutex<Instant>,
    last_activity: Mutex<Instant>,
    /// Cancelled when the transfer ends, for whatever reason.
    done: CancellationToken,
}

struct SessionFile {
    token: String,
    path: PathBuf,
    size: u64,
}

pub async fn serve(
    shared: Arc<Shared>,
    tls: Arc<ServerConfig>,
    ctk: CancellationToken,
) -> Result<(), anyhow::Error> {
    let port = shared.port;
    let app = Router::new()
        .route(&format!("{API}/info"), get(info))
        .route(&format!("{API}/register"), post(register))
        .route(&format!("{API}/prepare-upload"), post(prepare_upload))
        .route(&format!("{API}/upload"), post(upload))
        .route(&format!("{API}/cancel"), post(cancel))
        .with_state(shared);

    let handle = Handle::new();
    let shutdown = handle.clone();
    tokio::spawn(async move {
        ctk.cancelled().await;
        shutdown.shutdown();
    });

    info!("LocalSend: serving on port {port}");
    axum_server::bind(SocketAddr::from((Ipv4Addr::UNSPECIFIED, port)))
        .acceptor(SenderCertAcceptor(RustlsAcceptor::new(
            RustlsConfig::from_config(tls),
        )))
        .handle(handle)
        .serve(app.into_make_service_with_connect_info::<SocketAddr>())
        .await?;
    Ok(())
}

async fn info(AxumState(shared): SharedState) -> Json<DeviceInfo> {
    Json(shared.info(None))
}

async fn register(
    AxumState(shared): SharedState,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Json(peer): Json<DeviceInfo>,
) -> Json<DeviceInfo> {
    shared.found(&peer, addr.ip());
    Json(shared.info(None))
}

/// Reports the request as cancelled if the sender hangs up before the user decides.
struct PendingRequest<'a> {
    shared: &'a Shared,
    id: &'a str,
    meta: &'a TransferMetadata,
    decided: bool,
}

impl Drop for PendingRequest<'_> {
    fn drop(&mut self) {
        if !self.decided {
            self.shared
                .emit(self.id, TransferType::Inbound, State::Cancelled, self.meta);
        }
    }
}

async fn prepare_upload(
    AxumState(shared): SharedState,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Extension(sender): Extension<SenderCert>,
    Json(request): Json<PrepareUpload>,
) -> Response {
    if !shared.visible() {
        return StatusCode::FORBIDDEN.into_response();
    }

    let id = format!("ls-{}", random_id());
    let dir = get_download_dir();
    let text = message_text(&request.files);

    let mut files = HashMap::new();
    let mut names = vec![];
    let mut total_bytes = 0;
    if text.is_none() {
        for file in request.files.values() {
            let Some(path) = sanitize_relative_path(&file.file_name) else {
                return StatusCode::BAD_REQUEST.into_response();
            };
            names.push(path.to_string_lossy().into_owned());
            total_bytes += file.size;
            files.insert(
                file.id.clone(),
                SessionFile {
                    token: random_id(),
                    path,
                    size: file.size,
                },
            );
        }
    }

    let meta = TransferMetadata {
        source: Some(RemoteDeviceInfo {
            name: request.info.alias.clone(),
            device_type: request.info.device_type(),
            fingerprint: sender.0,
        }),
        destination: text.is_none().then(|| dir.to_string_lossy().into_owned()),
        files: text.is_none().then_some(names),
        text_type: text.as_deref().map(text_type),
        text_payload: text.clone(),
        total_bytes,
        ..Default::default()
    };

    info!(
        "LocalSend: {} wants to send, asking as {id}",
        request.info.alias
    );
    let mut actions = shared.sender.subscribe();
    shared.emit(
        &id,
        TransferType::Inbound,
        State::WaitingForUserConsent,
        &meta,
    );
    let mut pending = PendingRequest {
        shared: &shared,
        id: &id,
        meta: &meta,
        decided: false,
    };
    let Ok(action) = tokio::time::timeout(CONSENT_TIMEOUT, next_action(&mut actions, &id)).await
    else {
        info!("LocalSend: nobody answered {id}");
        return StatusCode::FORBIDDEN.into_response();
    };
    pending.decided = true;
    drop(pending);

    if action != Some(ChannelAction::AcceptTransfer) {
        shared.emit(&id, TransferType::Inbound, State::Rejected, &meta);
        return StatusCode::FORBIDDEN.into_response();
    }

    if files.is_empty() {
        shared.emit(&id, TransferType::Inbound, State::Finished, &meta);
        return StatusCode::NO_CONTENT.into_response();
    }

    let session_id = random_id();
    let tokens = files
        .iter()
        .map(|(file_id, file)| (file_id.clone(), file.token.clone()))
        .collect();
    let session = Arc::new(Session {
        id: id.clone(),
        remote: addr.ip(),
        dir,
        remaining: Mutex::new(files.keys().cloned().collect()),
        files,
        meta: Mutex::new(meta.clone()),
        last_report: Mutex::new(Instant::now()),
        last_activity: Mutex::new(Instant::now()),
        done: CancellationToken::new(),
    });
    shared
        .sessions
        .lock()
        .unwrap()
        .insert(session_id.clone(), session.clone());
    shared.emit(&id, TransferType::Inbound, State::ReceivingFiles, &meta);

    // Cancelling from our side, or a sender gone quiet: drop the session, so
    // the uploads fail.
    let cancelled = CancellationToken::new();
    watch_cancel(actions, id, cancelled.clone(), session.done.clone());
    let (s, sid) = (shared.clone(), session_id.clone());
    tokio::spawn(async move {
        let mut check = tokio::time::interval(Duration::from_secs(5));
        loop {
            tokio::select! {
                _ = session.done.cancelled() => return,
                _ = cancelled.cancelled() => return end(&s, &sid, State::Cancelled),
                _ = check.tick() => {
                    if session.last_activity.lock().unwrap().elapsed() > IDLE_TIMEOUT {
                        warn!("LocalSend: {} went quiet", session.id);
                        return end(&s, &sid, State::Disconnected);
                    }
                }
            }
        }
    });

    Json(PrepareUploadResponse {
        session_id,
        files: tokens,
    })
    .into_response()
}

/// Ends session `session_id` in `state`, if it is still running.
fn end(shared: &Shared, session_id: &str, state: State) {
    let Some(session) = shared.sessions.lock().unwrap().remove(session_id) else {
        return;
    };
    session.done.cancel();
    let meta = session.meta.lock().unwrap().clone();
    shared.emit(&session.id, TransferType::Inbound, state, &meta);
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct UploadQuery {
    session_id: String,
    file_id: String,
    token: String,
}

async fn upload(
    AxumState(shared): SharedState,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Query(query): Query<UploadQuery>,
    body: Body,
) -> StatusCode {
    let session = shared
        .sessions
        .lock()
        .unwrap()
        .get(&query.session_id)
        .cloned();
    let Some(session) = session else {
        return StatusCode::FORBIDDEN;
    };
    let Some(file) = session.files.get(&query.file_id) else {
        return StatusCode::FORBIDDEN;
    };
    if file.token != query.token || session.remote != addr.ip() {
        return StatusCode::FORBIDDEN;
    }
    if !session.remaining.lock().unwrap().contains(&query.file_id) {
        return StatusCode::CONFLICT;
    }

    if let Err(e) = session.receive(&shared, file, body).await {
        warn!("LocalSend: receiving into {}: {e}", session.id);
        end(&shared, &query.session_id, State::Disconnected);
        return StatusCode::INTERNAL_SERVER_ERROR;
    }

    let finished = {
        let mut remaining = session.remaining.lock().unwrap();
        remaining.remove(&query.file_id);
        remaining.is_empty()
    };
    if finished {
        info!("LocalSend: {} finished", session.id);
        end(&shared, &query.session_id, State::Finished);
    }
    StatusCode::OK
}

impl Session {
    async fn receive(
        &self,
        shared: &Shared,
        file: &SessionFile,
        body: Body,
    ) -> Result<(), anyhow::Error> {
        *self.last_activity.lock().unwrap() = Instant::now();
        let (path, std_file) = create_unique_file(&self.dir.join(&file.path))?;
        info!("LocalSend: receiving into {}", path.display());
        let result = self.write(shared, file, body, std_file).await;
        if result.is_err() {
            let _ = std::fs::remove_file(&path);
        }
        result
    }

    async fn write(
        &self,
        shared: &Shared,
        file: &SessionFile,
        body: Body,
        std_file: std::fs::File,
    ) -> Result<(), anyhow::Error> {
        let mut out = tokio::fs::File::from_std(std_file);
        let mut stream = body.into_data_stream();
        let mut received = 0u64;

        loop {
            let chunk = tokio::select! {
                _ = self.done.cancelled() => return Err(anyhow!("cancelled")),
                chunk = stream.next() => chunk,
            };
            let Some(chunk) = chunk else { break };
            let chunk = chunk?;
            received += chunk.len() as u64;
            if received > file.size {
                return Err(anyhow!("more bytes than the announced {}", file.size));
            }
            out.write_all(&chunk).await?;
            self.progress(shared, chunk.len() as u64);
        }

        if received != file.size {
            return Err(anyhow!("got {received} of {} bytes", file.size));
        }
        out.flush().await?;
        Ok(())
    }

    fn progress(&self, shared: &Shared, bytes: u64) {
        *self.last_activity.lock().unwrap() = Instant::now();
        let mut meta = self.meta.lock().unwrap();
        meta.ack_bytes += bytes;
        let mut last = self.last_report.lock().unwrap();
        if last.elapsed() >= PROGRESS_INTERVAL {
            *last = Instant::now();
            shared.emit(
                &self.id,
                TransferType::Inbound,
                State::ReceivingFiles,
                &meta,
            );
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CancelQuery {
    session_id: String,
}

async fn cancel(
    AxumState(shared): SharedState,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Query(query): Query<CancelQuery>,
) -> StatusCode {
    let remote = shared
        .sessions
        .lock()
        .unwrap()
        .get(&query.session_id)
        .map(|session| session.remote);
    if remote == Some(addr.ip()) {
        info!("LocalSend: sender cancelled session {}", query.session_id);
        end(&shared, &query.session_id, State::Cancelled);
    } else if let Some((peer, token)) = shared.outbound.lock().unwrap().get(&query.session_id)
        && *peer == addr.ip()
    {
        info!("LocalSend: receiver cancelled session {}", query.session_id);
        token.cancel();
    }
    StatusCode::OK
}
