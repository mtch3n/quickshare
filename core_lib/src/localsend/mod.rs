//! LocalSend protocol v2 (<https://github.com/localsend/protocol>), served next
//! to Quick Share so the LocalSend apps can send to us and receive from us.
//!
//! 1. Discovery: a JSON [`DeviceInfo`] announcement on multicast
//!    224.0.0.167:53317. Peers answer by registering with our HTTPS server on
//!    the same port, or with a multicast reply (`announce: false`).
//! 2. Receiving: the sender posts its file list to `prepare-upload`, which we
//!    hold until the user decides, answering with a session id and a token per
//!    file. It then posts each file's bytes to `upload`. A text message is a
//!    single `text/plain` file whose content is its preview, so nothing is
//!    uploaded.
//! 3. Sending: the same, as the client.
//!
//! Both directions report through the same [`ChannelMessage`]s as Quick Share,
//! so the frontend, notifications and trusted devices work unchanged.

mod client;
mod discovery;
mod identity;
mod server;

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rand::RngExt;
use rand::distr::Alphanumeric;
use serde::{Deserialize, Serialize};
use tokio::sync::{Notify, broadcast, watch};
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use crate::channel::{ChannelAction, ChannelDirection, ChannelMessage, TransferType};
use crate::hdl::info::TransferMetadata;
use crate::hdl::{EndpointInfo, Protocol, State, TextPayloadType, Visibility};
use crate::manager::SendInfo;
use crate::utils::{DeviceType, is_web_url};

pub const PORT: u16 = 53317;
const MULTICAST: Ipv4Addr = Ipv4Addr::new(224, 0, 0, 167);
const API: &str = "/api/localsend/v2";
const VERSION: &str = "2.1";
/// How often transfer progress is reported to the frontend.
const PROGRESS_INTERVAL: Duration = Duration::from_millis(200);

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DeviceInfo {
    alias: String,
    version: String,
    #[serde(default)]
    device_model: Option<String>,
    #[serde(default)]
    device_type: Option<String>,
    fingerprint: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    port: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    protocol: Option<String>,
    #[serde(default)]
    download: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    announce: Option<bool>,
}

impl DeviceInfo {
    fn device_type(&self) -> DeviceType {
        match self.device_type.as_deref() {
            Some("mobile") => DeviceType::Phone,
            Some("desktop") => DeviceType::Laptop,
            _ => DeviceType::Unknown,
        }
    }

    fn https(&self) -> bool {
        self.protocol.as_deref() != Some("http")
    }

    fn endpoint(&self, ip: IpAddr) -> EndpointInfo {
        EndpointInfo {
            id: format!("ls-{}", self.fingerprint),
            name: Some(self.alias.clone()),
            ip: Some(ip.to_string()),
            port: Some(self.port.unwrap_or(PORT).to_string()),
            rtype: Some(self.device_type()),
            present: Some(true),
            protocol: Protocol::LocalSend {
                https: self.https(),
            },
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FileInfo {
    id: String,
    file_name: String,
    size: u64,
    file_type: String,
    #[serde(default)]
    sha256: Option<String>,
    #[serde(default)]
    preview: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct PrepareUpload {
    info: DeviceInfo,
    files: HashMap<String, FileInfo>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PrepareUploadResponse {
    session_id: String,
    files: HashMap<String, String>,
}

/// The text of a message transfer: one `text/plain` file carried in its preview.
fn message_text(files: &HashMap<String, FileInfo>) -> Option<String> {
    let mut files = files.values();
    match (files.next(), files.next()) {
        (Some(file), None) if file.file_type.starts_with("text/plain") => file.preview.clone(),
        _ => None,
    }
}

fn text_type(text: &str) -> TextPayloadType {
    if is_web_url(text) {
        TextPayloadType::Url
    } else {
        TextPayloadType::Text
    }
}

fn random_id() -> String {
    rand::rng()
        .sample_iter(Alphanumeric)
        .take(16)
        .map(char::from)
        .collect()
}

/// State shared by the server, discovery and outgoing transfers.
struct Shared {
    fingerprint: String,
    /// Our HTTPS port; always [`PORT`] outside tests.
    port: u16,
    device_name: watch::Receiver<String>,
    visibility: watch::Receiver<Visibility>,
    sender: broadcast::Sender<ChannelMessage>,
    /// Where found peers go while the frontend is looking for devices.
    discovery: Mutex<Option<broadcast::Sender<EndpointInfo>>>,
    announce: Notify,
    sessions: Mutex<HashMap<String, Arc<server::Session>>>,
    http: reqwest::Client,
}

impl Shared {
    fn info(&self, announce: Option<bool>) -> DeviceInfo {
        DeviceInfo {
            alias: self.device_name.borrow().clone(),
            version: VERSION.into(),
            device_model: Some("Linux".into()),
            device_type: Some("desktop".into()),
            fingerprint: self.fingerprint.clone(),
            port: Some(self.port),
            protocol: Some("https".into()),
            download: Some(false),
            announce,
        }
    }

    fn visible(&self) -> bool {
        *self.visibility.borrow() != Visibility::Invisible
    }

    /// Reports a peer to the frontend, if it is looking for devices.
    fn found(&self, info: &DeviceInfo, ip: IpAddr) {
        if info.fingerprint == self.fingerprint {
            return;
        }
        if let Some(sender) = self.discovery.lock().unwrap().as_ref() {
            let _ = sender.send(info.endpoint(ip));
        }
    }

    fn emit(&self, id: &str, rtype: TransferType, state: State, meta: &TransferMetadata) {
        let _ = self.sender.send(ChannelMessage {
            id: id.to_owned(),
            direction: ChannelDirection::LibToFront,
            rtype: Some(rtype),
            state: Some(state),
            meta: Some(meta.clone()),
            ..Default::default()
        });
    }
}

/// Waits for the frontend's next action on transfer `id`.
async fn next_action(
    receiver: &mut broadcast::Receiver<ChannelMessage>,
    id: &str,
) -> Option<ChannelAction> {
    loop {
        match receiver.recv().await {
            Ok(msg) if msg.direction == ChannelDirection::FrontToLib && msg.id == id => {
                if let Some(action) = msg.action {
                    return Some(action);
                }
            }
            Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => {}
            Err(broadcast::error::RecvError::Closed) => return None,
        }
    }
}

/// Cancels `token` once the frontend cancels transfer `id`, or when `done` is.
fn watch_cancel(
    mut receiver: broadcast::Receiver<ChannelMessage>,
    id: String,
    token: CancellationToken,
    done: CancellationToken,
) {
    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = done.cancelled() => return,
                action = next_action(&mut receiver, &id) => match action {
                    Some(ChannelAction::CancelTransfer) | None => {
                        token.cancel();
                        return;
                    }
                    Some(_) => {}
                },
            }
        }
    });
}

#[derive(Clone)]
pub struct LocalSend {
    shared: Arc<Shared>,
}

impl std::fmt::Debug for LocalSend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LocalSend").finish_non_exhaustive()
    }
}

impl LocalSend {
    /// Starts the server and discovery on `tracker`. `data_dir` keeps the TLS
    /// certificate, whose hash is our fingerprint, across restarts.
    pub fn start(
        data_dir: Option<PathBuf>,
        device_name: watch::Receiver<String>,
        visibility: watch::Receiver<Visibility>,
        sender: broadcast::Sender<ChannelMessage>,
        tracker: &TaskTracker,
        ctk: CancellationToken,
    ) -> Result<Self, anyhow::Error> {
        // reqwest and the server both use rustls' process-wide provider.
        let _ = rustls::crypto::ring::default_provider().install_default();

        let identity = identity::load_or_create(data_dir.as_deref())?;
        let http = reqwest::Client::builder()
            .tls_danger_accept_invalid_certs(true)
            .connect_timeout(Duration::from_secs(5))
            .build()?;

        let shared = Arc::new(Shared {
            fingerprint: identity.fingerprint,
            port: PORT,
            device_name,
            visibility,
            sender,
            discovery: Mutex::new(None),
            announce: Notify::new(),
            sessions: Mutex::new(HashMap::new()),
            http,
        });

        let (s, c) = (shared.clone(), ctk.clone());
        tracker.spawn(async move {
            if let Err(e) = server::serve(s, identity.tls, c).await {
                warn!("LocalSend: server stopped: {e}");
            }
        });
        let s = shared.clone();
        tracker.spawn(async move {
            if let Err(e) = discovery::run(s, ctk).await {
                warn!("LocalSend: discovery stopped: {e}");
            }
        });

        Ok(Self { shared })
    }

    /// Announces ourselves and reports the peers that answer to `sender`.
    pub fn start_discovery(&self, sender: broadcast::Sender<EndpointInfo>) {
        *self.shared.discovery.lock().unwrap() = Some(sender);
        self.shared.announce.notify_one();
    }

    pub fn stop_discovery(&self) {
        *self.shared.discovery.lock().unwrap() = None;
    }

    pub fn send(&self, info: SendInfo, https: bool) {
        tokio::spawn(client::send(self.shared.clone(), info, https));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hdl::OutboundPayload;

    /// A LocalSend server on `port`, which we then send to: it is its own peer.
    async fn start(port: u16) -> Arc<Shared> {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let identity = identity::load_or_create(None).unwrap();
        let shared = Arc::new(Shared {
            fingerprint: identity.fingerprint,
            port,
            device_name: watch::channel("Desk".to_owned()).1,
            visibility: watch::channel(Visibility::Visible).1,
            sender: broadcast::channel(256).0,
            discovery: Mutex::new(None),
            announce: Notify::new(),
            sessions: Mutex::new(HashMap::new()),
            http: reqwest::Client::builder()
                .tls_danger_accept_invalid_certs(true)
                .build()
                .unwrap(),
        });
        tokio::spawn(server::serve(
            shared.clone(),
            identity.tls,
            CancellationToken::new(),
        ));
        tokio::time::sleep(Duration::from_millis(200)).await;
        shared
    }

    /// Sends `payload` to ourselves, answering the consent request with
    /// `answer`, and returns the final outbound and inbound messages.
    async fn send_to_self(
        port: u16,
        payload: OutboundPayload,
        answer: ChannelAction,
    ) -> (ChannelMessage, ChannelMessage) {
        let shared = start(port).await;
        let mut messages = shared.sender.subscribe();
        tokio::spawn(client::send(
            shared.clone(),
            SendInfo {
                id: "peer".into(),
                name: "Desk".into(),
                addr: format!("127.0.0.1:{port}"),
                protocol: Protocol::LocalSend { https: true },
                ob: payload,
            },
            true,
        ));

        let terminal = |state: &Option<State>| {
            matches!(
                state,
                Some(State::Finished | State::Rejected | State::Cancelled | State::Disconnected)
            )
        };
        let (mut outbound, mut inbound) = (None, None);
        tokio::time::timeout(Duration::from_secs(10), async {
            while outbound.is_none() || inbound.is_none() {
                let msg = messages.recv().await.unwrap();
                if msg.direction != ChannelDirection::LibToFront {
                    continue;
                }
                match msg.rtype {
                    Some(TransferType::Inbound)
                        if msg.state == Some(State::WaitingForUserConsent) =>
                    {
                        let _ = shared.sender.send(ChannelMessage {
                            id: msg.id.clone(),
                            direction: ChannelDirection::FrontToLib,
                            action: Some(answer.clone()),
                            ..Default::default()
                        });
                    }
                    Some(TransferType::Inbound) if terminal(&msg.state) => inbound = Some(msg),
                    Some(TransferType::Outbound) if terminal(&msg.state) => outbound = Some(msg),
                    _ => {}
                }
            }
        })
        .await
        .expect("transfer didn't end");
        (outbound.unwrap(), inbound.unwrap())
    }

    #[tokio::test]
    async fn sends_and_receives_files() {
        let _download_dir = crate::DOWNLOAD_DIR_TEST_LOCK.lock().await;
        let dir = std::env::temp_dir().join(format!("rqs-localsend-{}", random_id()));
        let source = dir.join("out/photo.jpg");
        std::fs::create_dir_all(source.parent().unwrap()).unwrap();
        let content = vec![7u8; 3 * 1024 * 1024 + 5];
        std::fs::write(&source, &content).unwrap();
        *crate::CUSTOM_DOWNLOAD.write().unwrap() = Some(dir.join("in"));

        let (outbound, inbound) = send_to_self(
            53391,
            OutboundPayload::Files(vec![source.to_string_lossy().into_owned()]),
            ChannelAction::AcceptTransfer,
        )
        .await;

        assert_eq!(outbound.state, Some(State::Finished));
        assert_eq!(inbound.state, Some(State::Finished));
        assert_eq!(inbound.meta.unwrap().ack_bytes, content.len() as u64);
        assert_eq!(std::fs::read(dir.join("in/photo.jpg")).unwrap(), content);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn sends_and_receives_text() {
        let (outbound, inbound) = send_to_self(
            53392,
            OutboundPayload::Text("https://localsend.org".into()),
            ChannelAction::AcceptTransfer,
        )
        .await;

        assert_eq!(outbound.state, Some(State::Finished));
        let meta = inbound.meta.unwrap();
        assert_eq!(inbound.state, Some(State::Finished));
        assert_eq!(meta.text_payload.as_deref(), Some("https://localsend.org"));
        assert!(matches!(meta.text_type, Some(TextPayloadType::Url)));
    }

    #[tokio::test]
    async fn reports_declined_transfers() {
        let (outbound, inbound) = send_to_self(
            53393,
            OutboundPayload::Text("hi".into()),
            ChannelAction::RejectTransfer,
        )
        .await;

        assert_eq!(outbound.state, Some(State::Rejected));
        assert_eq!(inbound.state, Some(State::Rejected));
    }
}
