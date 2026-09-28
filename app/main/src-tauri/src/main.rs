#[macro_use]
extern crate log;

use std::path::Path;
use std::sync::{Arc, Mutex};

use rqs_lib::channel::{ChannelAction, ChannelDirection, ChannelMessage};
use rqs_lib::{EndpointInfo, RQS, SendInfo, State, TextPayloadType, Visibility};
use tauri::{AppHandle, Emitter, Manager, Window, WindowEvent};
use tauri_plugin_autostart::MacosLauncher;
use tauri_plugin_clipboard_manager::ClipboardExt;
use tauri_plugin_opener::OpenerExt;
use tokio::sync::{broadcast, mpsc, watch};

use crate::logger::set_up_logging;
use crate::notification::{send_request_notification, send_temporarily_notification};
use crate::tray::TrayHandle;

mod commands;
mod integrations;
mod logger;
mod notification;
mod store;
mod tray;
mod wifi;

/// Passed by the autostart entry so the app starts in the tray.
const HIDDEN_ARG: &str = "--hidden";

pub struct AppState {
    pub message_sender: broadcast::Sender<ChannelMessage>,
    pub dch_sender: broadcast::Sender<EndpointInfo>,
    pub visibility_sender: Arc<Mutex<watch::Sender<Visibility>>>,
    pub device_name_sender: Arc<Mutex<watch::Sender<String>>>,
    pub sender_file: mpsc::Sender<SendInfo>,
    pub ble_receiver: broadcast::Receiver<()>,
    pub rqs: Mutex<RQS>,
}

/// Files handed to us on the command line (file manager "Send with Quick Share")
/// before the frontend was ready to receive them.
#[derive(Default)]
pub struct PendingFiles(pub Mutex<Vec<String>>);

fn main() -> Result<(), anyhow::Error> {
    // WebKitGTK's DMA-BUF renderer shows a blank window on NVIDIA and some Mesa
    // setups. This has to happen before any other thread exists.
    // SAFETY: we are still single-threaded, nothing else reads the environment.
    unsafe {
        if std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").is_none() {
            std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
        }
        if Path::new("/sys/module/nvidia").exists()
            && std::env::var_os("__NV_DISABLE_EXPLICIT_SYNC").is_none()
        {
            std::env::set_var("__NV_DISABLE_EXPLICIT_SYNC", "1");
        }
    }

    // Tauri drives its async work on our runtime, but the app itself must be
    // built on the main thread outside of it: plugins block on the runtime
    // while they initialize.
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    tauri::async_runtime::set(runtime.handle().clone());

    run()
}

fn run() -> Result<(), anyhow::Error> {
    let args: Vec<String> = std::env::args().collect();
    let start_hidden = args.iter().any(|a| a == HIDDEN_ARG);
    let cwd = std::env::current_dir().unwrap_or_default();
    let initial_files = files_from_args(&args, &cwd);

    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, argv, cwd| {
            trace!("tauri_plugin_single_instance: instance already running");
            let files = files_from_args(&argv, Path::new(&cwd));
            if argv.iter().any(|a| a == HIDDEN_ARG) && files.is_empty() {
                return;
            }

            open_main_window(app);
            if !files.is_empty() {
                let _ = app.emit("send_files", files);
            }
        }))
        .plugin(tauri_plugin_store::Builder::new().build())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            Some(vec![HIDDEN_ARG]),
        ))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .manage(PendingFiles(Mutex::new(initial_files)))
        .invoke_handler(tauri::generate_handler![
            commands::get_settings,
            commands::set_visibility,
            commands::set_download_path,
            commands::set_device_name,
            commands::set_keep_running,
            commands::set_file_manager_integration,
            commands::start_discovery,
            commands::stop_discovery,
            commands::send_payload,
            commands::transfer_action,
            commands::take_pending_files,
            commands::connect_wifi,
            commands::trust_device,
            commands::untrust_device,
            commands::set_auto_open_links,
            commands::set_auto_copy_text,
        ])
        .setup(move |app| {
            set_up_logging(app.app_handle())?;
            debug!("Starting setup of RQuickShare app");

            // Keep the file manager entries pointing at the current executable,
            // which moves when an AppImage is updated.
            if integrations::installed(app.app_handle())
                && let Err(e) = integrations::install(app.app_handle())
            {
                warn!("Couldn't refresh file manager integration: {e}");
            }

            let visibility = store::visibility(app.app_handle());
            let port_number = store::port(app.app_handle());
            let download_path = store::download_path(app.app_handle());
            let device_name = store::device_name(app.app_handle());

            let app_handle = app.app_handle().clone();
            // Block until the service is up so the logger is already in place
            // and every command can rely on AppState being managed.
            tauri::async_runtime::block_on(async move {
                let mut rqs = RQS::new(visibility, port_number, download_path, device_name);
                let (sender_file, ble_receiver) = rqs.run().await?;

                app_handle.manage(AppState {
                    message_sender: rqs.message_sender.clone(),
                    dch_sender: broadcast::channel(10).0,
                    visibility_sender: rqs.visibility_sender.clone(),
                    device_name_sender: rqs.device_name_sender.clone(),
                    sender_file,
                    ble_receiver,
                    rqs: Mutex::new(rqs),
                });
                app_handle.manage(tray::spawn(&app_handle, visibility).await);

                Ok::<_, anyhow::Error>(())
            })?;

            if let Some(window) = app.get_webview_window("main") {
                fix_wayland_titlebar(&window);
                if start_hidden {
                    let _ = window.hide();
                }
            }

            spawn_receiver_tasks(app.app_handle());
            Ok(())
        })
        .on_window_event(handle_window_event)
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app_handle, event| {
            if let tauri::RunEvent::ExitRequested { code, .. } = event {
                trace!("RunEvent::ExitRequested");
                if code != Some(-1) {
                    kill_app(app_handle);
                }
            }
        });

    info!("Application stopped");
    Ok(())
}

/// Regular files among the arguments, resolved against `cwd`. Accepts both
/// `rquickshare --send FILE...` and plain `rquickshare FILE...` (desktop %F).
fn files_from_args(args: &[String], cwd: &Path) -> Vec<String> {
    args.iter()
        .skip(1)
        .filter(|a| !a.starts_with("--"))
        .map(|a| cwd.join(a))
        .filter(|p| p.is_file() || p.is_dir())
        .map(|p| p.to_string_lossy().into_owned())
        .collect()
}

fn spawn_receiver_tasks(app_handle: &AppHandle) {
    let capp_handle = app_handle.clone();
    tauri::async_runtime::spawn(async move {
        let state: tauri::State<'_, AppState> = capp_handle.state();
        let tray: tauri::State<'_, TrayHandle> = capp_handle.state();
        let mut receiver = state.message_sender.subscribe();
        let mut handled_finished: std::collections::HashSet<String> =
            std::collections::HashSet::new();
        let mut notified_requests: std::collections::HashSet<String> =
            std::collections::HashSet::new();

        loop {
            match receiver.recv().await {
                Ok(info) => {
                    if info.direction == ChannelDirection::FrontToLib {
                        continue;
                    }

                    let waiting = info.state == Some(State::WaitingForUserConsent);
                    let finished = info.state == Some(State::Finished);

                    // Auto-accept from trusted devices
                    if waiting {
                        let name = info
                            .meta
                            .as_ref()
                            .and_then(|meta| meta.source.as_ref())
                            .map(|source| source.name.clone())
                            .unwrap_or_else(|| "Unknown".to_string());

                        let trusted_devices = store::trusted_devices(&capp_handle);
                        if trusted_devices.contains(&name) {
                            trace!("Auto-accepting from trusted device: {}", name);
                            commands::send_action(
                                &state,
                                info.id.clone(),
                                ChannelAction::AcceptTransfer,
                            );
                            if info.state.is_some() {
                                tray.set_waiting(info.id.clone(), false).await;
                            }
                            continue;
                        }

                        // The app always asks; the notification covers the
                        // times nobody is looking at it.
                        if notified_requests.insert(info.id.clone())
                            && !main_window_focused(&capp_handle)
                        {
                            let meta = info.meta.as_ref();
                            send_request_notification(
                                name,
                                meta.and_then(|m| m.pin_code.clone()),
                                meta.and_then(|m| m.files.clone()),
                                meta.and_then(|m| m.text_type.as_ref().map(|t| format!("{t:?}"))),
                                info.id.clone(),
                                &capp_handle,
                            );
                        }
                    }

                    // Auto-open links, auto-copy text, and show received notification on Finished
                    if finished && !handled_finished.contains(&info.id) {
                        handled_finished.insert(info.id.clone());

                        if let Some(meta) = &info.meta {
                            // Auto-open links
                            if store::auto_open_links(&capp_handle)
                                && matches!(meta.text_type, Some(TextPayloadType::Url))
                                && let Some(url) = &meta.text_payload
                                && is_web_url(url)
                                && let Err(e) = capp_handle.opener().open_url(url, None::<&str>)
                            {
                                warn!("Couldn't auto-open URL: {e}");
                            }

                            // Auto-copy text
                            if store::auto_copy_text(&capp_handle)
                                && matches!(meta.text_type, Some(TextPayloadType::Text))
                                && let Some(text) = &meta.text_payload
                                && let Err(e) = capp_handle.clipboard().write_text(text.clone())
                            {
                                warn!("Couldn't auto-copy text: {e}");
                            }

                            // The app shows a toast; notify when nobody is looking at it.
                            if !main_window_focused(&capp_handle) {
                                let source_name = meta
                                    .source
                                    .as_ref()
                                    .map(|source| source.name.clone())
                                    .unwrap_or_else(|| "A nearby device".to_string());
                                let text_type_str =
                                    meta.text_type.as_ref().map(|t| format!("{t:?}"));
                                notification::send_received_notification(
                                    source_name,
                                    meta.files.clone(),
                                    meta.destination.clone(),
                                    text_type_str,
                                    meta.text_payload.clone(),
                                    &capp_handle,
                                );
                            }
                        }
                    }

                    if info.state.is_some() {
                        tray.set_waiting(info.id.clone(), waiting).await;
                    }

                    trace!("rs2js_channelmessage: {info:?}");
                    let _ = capp_handle.emit("rs2js_channelmessage", &info);
                }
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    warn!("message_sender: skipped {n} messages");
                }
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    });

    let capp_handle = app_handle.clone();
    tauri::async_runtime::spawn(async move {
        let state: tauri::State<'_, AppState> = capp_handle.state();
        let mut dch_receiver = state.dch_sender.subscribe();

        loop {
            match dch_receiver.recv().await {
                Ok(info) => {
                    trace!("rs2js_endpointinfo: {info:?}");
                    let _ = capp_handle.emit("rs2js_endpointinfo", &info);
                }
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    warn!("dch_sender: skipped {n} messages");
                }
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    });

    let capp_handle = app_handle.clone();
    tauri::async_runtime::spawn(async move {
        let state: tauri::State<'_, AppState> = capp_handle.state();
        let tray: tauri::State<'_, TrayHandle> = capp_handle.state();
        let mut visibility_receiver = state.visibility_sender.lock().unwrap().subscribe();

        while visibility_receiver.changed().await.is_ok() {
            let v = *visibility_receiver.borrow_and_update();
            store::set_visibility(&capp_handle, v);
            let _ = capp_handle.emit("visibility_updated", v);
            tray.set_visibility(v).await;
        }
    });

    let capp_handle = app_handle.clone();
    tauri::async_runtime::spawn(async move {
        let state: tauri::State<'_, AppState> = capp_handle.state();
        let mut device_name_receiver = state.device_name_sender.lock().unwrap().subscribe();

        while device_name_receiver.changed().await.is_ok() {
            let name = device_name_receiver.borrow_and_update().clone();
            store::set_device_name(&capp_handle, Some(&name));
            let _ = capp_handle.emit("device_name_updated", &name);
        }
    });

    let capp_handle = app_handle.clone();
    tauri::async_runtime::spawn(async move {
        let state: tauri::State<'_, AppState> = capp_handle.state();
        let mut ble_receiver = state.ble_receiver.resubscribe();
        let mut last_sent: Option<std::time::Instant> = None;

        loop {
            match ble_receiver.recv().await {
                Ok(_) => {
                    let v = store::visibility(&capp_handle);
                    trace!("Tauri: ble received: {:?}", v);

                    if v == Visibility::Invisible
                        && last_sent.is_none_or(|t| t.elapsed().as_secs() >= 120)
                    {
                        send_temporarily_notification(&capp_handle);
                        last_sent = Some(std::time::Instant::now());
                    }
                }
                Err(broadcast::error::RecvError::Lagged(_)) => {}
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    });
}

fn handle_window_event(w: &Window, event: &WindowEvent) {
    if let WindowEvent::CloseRequested { api, .. } = event {
        if !store::keep_running(w.app_handle()) {
            trace!("handle_window_event: real close");
            return;
        }

        trace!("handle_window_event: prevent close");
        let _ = w.hide();
        api.prevent_close();
    }
}

pub fn open_main_window(app_handle: &AppHandle) {
    match app_handle.get_webview_window("main") {
        Some(window) => {
            let _ = window.unminimize();
            let _ = window.show();
            let _ = window.set_focus();
        }
        None => warn!("open_main_window: no main window found"),
    }
}

fn main_window_focused(app_handle: &AppHandle) -> bool {
    app_handle.get_webview_window("main").is_some_and(|window| {
        window.is_visible().unwrap_or(false) && window.is_focused().unwrap_or(false)
    })
}

pub fn kill_app(app_handle: &AppHandle) {
    let state: tauri::State<'_, AppState> = app_handle.state();

    tokio::task::block_in_place(|| {
        #[allow(clippy::await_holding_lock)]
        tauri::async_runtime::block_on(async move {
            state.rqs.lock().unwrap().stop().await;
        });
    });

    app_handle.exit(-1);
}

/// Makes the titlebar buttons clickable under Wayland.
///
/// On Wayland tao (up to 0.35) wraps its client-side header bar in a GtkEventBox
/// with `above-child` set, so the box swallows every click and the close button
/// does nothing once the window has been hidden and shown again (#422).
/// Remove this and the `gtk` dependency once Tauri ships tao 0.36+.
fn fix_wayland_titlebar(window: &tauri::WebviewWindow) {
    use gtk::prelude::*;

    let Ok(gtk_window) = window.gtk_window() else {
        return;
    };

    if let Some(titlebar) = gtk_window.titlebar()
        && let Ok(event_box) = titlebar.downcast::<gtk::EventBox>()
    {
        event_box.set_above_child(false);
    }
}

/// Links from peers are only opened if they are web links: a `file:` or custom
/// scheme URL would hand attacker-controlled input to arbitrary handlers.
pub fn is_web_url(url: &str) -> bool {
    let url = url.trim().to_ascii_lowercase();
    (url.starts_with("https://") || url.starts_with("http://"))
        && !url.contains(char::is_whitespace)
}
