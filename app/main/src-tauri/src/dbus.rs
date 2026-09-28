//! The session D-Bus service through which the CLI, the browser extension's
//! native host and the GNOME Shell extension drive the running app.

use std::collections::HashMap;
use std::time::Duration;

use rqs_lib::channel::{ChannelAction, ChannelDirection, TransferType};
use rqs_lib::{EndpointInfo, OutboundPayload, Protocol, SendInfo, State, Visibility};
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::broadcast;
use zbus::object_server::SignalEmitter;
use zbus::{connection, fdo, interface};

use crate::{AppState, commands, open_main_window};

pub const BUS_NAME: &str = "dev.mandre.RQuickShare";
pub const OBJECT_PATH: &str = "/dev/mandre/RQuickShare";

/// How long a send by device name looks for that device.
const FIND_TIMEOUT: Duration = Duration::from_secs(10);

struct Service {
    app: AppHandle,
}

#[interface(name = "dev.mandre.RQuickShare1")]
impl Service {
    /// Shows the window.
    fn show(&self) {
        open_main_window(&self.app);
    }

    /// Opens the send screen for files or folders, to pick a device there.
    fn pick_device_for_files(&self, paths: Vec<String>) {
        open_main_window(&self.app);
        let _ = self.app.emit("send_files", paths);
    }

    /// Opens the send screen for a text or link, to pick a device there.
    fn pick_device_for_text(&self, text: String) {
        open_main_window(&self.app);
        let _ = self.app.emit("send_text", text);
    }

    /// Devices found nearby within `seconds`: name, protocol and device type.
    async fn devices(&self, seconds: u32) -> Vec<(String, String, String)> {
        let found = discover(&self.app, Duration::from_secs(seconds.into()), |_| false).await;
        found
            .values()
            .map(|e| {
                (
                    e.name.clone().unwrap_or_default(),
                    protocol_name(&e.protocol).into(),
                    e.rtype.map(|t| format!("{t:?}")).unwrap_or_default(),
                )
            })
            .collect()
    }

    /// Sends files or folders to the nearby device called `device`. Returns the
    /// transfer id that `TransferChanged` reports on.
    async fn send_files(&self, device: String, paths: Vec<String>) -> fdo::Result<String> {
        self.send(&device, OutboundPayload::Files(paths)).await
    }

    /// Sends a text or link to the nearby device called `device`.
    async fn send_text(&self, device: String, text: String) -> fdo::Result<String> {
        self.send(&device, OutboundPayload::Text(text)).await
    }

    fn cancel(&self, id: String) {
        let state: tauri::State<'_, AppState> = self.app.state();
        commands::send_action(&state, id, ChannelAction::CancelTransfer);
    }

    /// `visible`, `hidden` or `temporary` (visible to everyone for a minute).
    #[zbus(property)]
    fn visibility(&self) -> String {
        visibility_name(crate::store::visibility(&self.app)).into()
    }

    #[zbus(property)]
    fn set_visibility(&mut self, visibility: String) -> fdo::Result<()> {
        let visibility = match visibility.as_str() {
            "visible" => Visibility::Visible,
            "hidden" => Visibility::Invisible,
            "temporary" => Visibility::Temporarily,
            other => {
                return Err(fdo::Error::InvalidArgs(format!(
                    "unknown visibility {other}"
                )));
            }
        };
        let state: tauri::State<'_, AppState> = self.app.state();
        state.rqs.lock().unwrap().change_visibility(visibility);
        Ok(())
    }

    #[zbus(property)]
    fn device_name(&self) -> String {
        crate::store::device_name(&self.app)
            .filter(|name| !name.trim().is_empty())
            .unwrap_or_else(rqs_lib::hostname)
    }

    /// A transfer's progress: its id, `inbound` or `outbound`, its state (as in
    /// the app, e.g. `SendingFiles`, `Finished`), the peer's name, and bytes.
    #[zbus(signal)]
    async fn transfer_changed(
        emitter: &SignalEmitter<'_>,
        id: &str,
        direction: &str,
        state: &str,
        peer: &str,
        done: u64,
        total: u64,
    ) -> zbus::Result<()>;
}

impl Service {
    async fn send(&self, device: &str, payload: OutboundPayload) -> fdo::Result<String> {
        let wanted = device.to_lowercase();
        let found = discover(&self.app, FIND_TIMEOUT, |e| {
            e.name
                .as_deref()
                .is_some_and(|n| n.to_lowercase() == wanted)
        })
        .await;
        let endpoint = pick(&found, &wanted)
            .ok_or_else(|| fdo::Error::Failed(format!("no nearby device called {device}")))?;

        let (Some(ip), Some(port)) = (&endpoint.ip, &endpoint.port) else {
            return Err(fdo::Error::Failed(format!("{device} has no address")));
        };
        let state: tauri::State<'_, AppState> = self.app.state();
        state
            .sender_file
            .send(SendInfo {
                id: endpoint.id.clone(),
                name: endpoint.name.clone().unwrap_or_default(),
                addr: format!("{ip}:{port}"),
                protocol: endpoint.protocol.clone(),
                ob: payload,
                pin: None,
            })
            .await
            .map_err(|e| fdo::Error::Failed(e.to_string()))?;
        Ok(endpoint.id.clone())
    }
}

/// The device named exactly `wanted`, else the only one whose name starts with it.
fn pick<'a>(found: &'a HashMap<String, EndpointInfo>, wanted: &str) -> Option<&'a EndpointInfo> {
    let named = |f: &dyn Fn(&str) -> bool| {
        found
            .values()
            .filter(|e| e.name.as_deref().is_some_and(|n| f(&n.to_lowercase())))
            .collect::<Vec<_>>()
    };
    if let Some(exact) = named(&|n| n == wanted).first() {
        return Some(exact);
    }
    match named(&|n| n.starts_with(wanted)).as_slice() {
        [only] => Some(only),
        _ => None,
    }
}

/// Looks for devices for up to `timeout`, or until `done` accepts one.
async fn discover(
    app: &AppHandle,
    timeout: Duration,
    done: impl Fn(&EndpointInfo) -> bool,
) -> HashMap<String, EndpointInfo> {
    let state: tauri::State<'_, AppState> = app.state();
    let mut receiver = state.dch_sender.subscribe();
    if let Err(e) = state
        .rqs
        .lock()
        .unwrap()
        .discovery(state.dch_sender.clone())
    {
        warn!("D-Bus: couldn't start discovery: {e}");
        return HashMap::new();
    }

    let mut found = HashMap::new();
    let _ = tokio::time::timeout(timeout, async {
        loop {
            match receiver.recv().await {
                Ok(endpoint) if endpoint.present == Some(true) => {
                    let hit = done(&endpoint);
                    found.insert(endpoint.id.clone(), endpoint);
                    if hit {
                        return;
                    }
                }
                Ok(endpoint) => {
                    found.remove(&endpoint.id);
                }
                Err(broadcast::error::RecvError::Lagged(_)) => {}
                Err(broadcast::error::RecvError::Closed) => return,
            }
        }
    })
    .await;

    // The send screen runs its own discovery; leave it alone while it shows.
    let window_open = app
        .get_webview_window("main")
        .is_some_and(|w| w.is_visible().unwrap_or(false));
    if !window_open {
        state.rqs.lock().unwrap().stop_discovery();
    }
    found
}

fn protocol_name(protocol: &Protocol) -> &'static str {
    match protocol {
        Protocol::QuickShare => "quickshare",
        Protocol::LocalSend { .. } => "localsend",
    }
}

fn visibility_name(visibility: Visibility) -> &'static str {
    match visibility {
        Visibility::Visible => "visible",
        Visibility::Invisible => "hidden",
        Visibility::Temporarily => "temporary",
    }
}

/// Claims the bus name and serves until the app exits, forwarding visibility
/// changes and transfer progress as D-Bus notifications.
pub async fn serve(app: AppHandle) -> Result<(), anyhow::Error> {
    let connection = connection::Builder::session()?
        .name(BUS_NAME)?
        .serve_at(OBJECT_PATH, Service { app: app.clone() })?
        .build()
        .await?;
    info!("D-Bus: serving {BUS_NAME}");

    let iface = connection
        .object_server()
        .interface::<_, Service>(OBJECT_PATH)
        .await?;
    let state: tauri::State<'_, AppState> = app.state();
    let mut visibility = state.visibility_sender.lock().unwrap().subscribe();
    let mut messages = state.message_sender.subscribe();

    loop {
        tokio::select! {
            changed = visibility.changed() => {
                if changed.is_err() {
                    break;
                }
                visibility.borrow_and_update();
                let service = iface.get().await;
                let _ = service.visibility_changed(iface.signal_emitter()).await;
            }
            message = messages.recv() => match message {
                Ok(msg) if msg.direction == ChannelDirection::LibToFront => {
                    let (Some(state), Some(rtype)) = (&msg.state, &msg.rtype) else {
                        continue;
                    };
                    let meta = msg.meta.unwrap_or_default();
                    let peer = meta.source.map(|s| s.name).unwrap_or_default();
                    let direction = match rtype {
                        TransferType::Inbound => "inbound",
                        TransferType::Outbound => "outbound",
                    };
                    let _ = Service::transfer_changed(
                        iface.signal_emitter(),
                        &msg.id,
                        direction,
                        &format!("{state:?}"),
                        &peer,
                        meta.ack_bytes,
                        meta.total_bytes,
                    )
                    .await;
                }
                Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => {}
                Err(broadcast::error::RecvError::Closed) => break,
            },
        }
    }
    Ok(())
}

/// Whether `state` ends a transfer.
pub fn is_terminal(state: &str) -> bool {
    [
        State::Finished,
        State::Rejected,
        State::Cancelled,
        State::Disconnected,
        State::PinRequired,
    ]
    .iter()
    .any(|s| format!("{s:?}") == state)
}
