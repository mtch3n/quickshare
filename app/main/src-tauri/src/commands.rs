//! Commands invoked by the frontend.

use std::path::PathBuf;

use rqs_lib::channel::{ChannelAction, ChannelDirection, ChannelMessage};
use rqs_lib::{SendInfo, Visibility, WifiNetwork};
use serde::Serialize;
use tauri::{AppHandle, State};

use crate::{AppState, PendingFiles, integrations, store, wifi};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    device_name: String,
    visibility: Visibility,
    download_path: String,
    keep_running: bool,
    file_manager_integration: bool,
    trusted_devices: Vec<String>,
    auto_open_links: bool,
    auto_copy_text: bool,
}

#[tauri::command]
pub fn get_settings(app: AppHandle) -> Settings {
    let effective_name = store::device_name(&app)
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(rqs_lib::hostname);
    Settings {
        device_name: effective_name,
        visibility: store::visibility(&app),
        download_path: rqs_lib::get_download_dir().to_string_lossy().into_owned(),
        keep_running: store::keep_running(&app),
        file_manager_integration: integrations::installed(&app),
        trusted_devices: store::trusted_devices(&app),
        auto_open_links: store::auto_open_links(&app),
        auto_copy_text: store::auto_copy_text(&app),
    }
}

/// Persisted by the visibility watcher in `main.rs`, which also sees changes
/// made from the tray, notifications and the temporary-visibility timeout.
#[tauri::command]
pub fn set_visibility(visibility: Visibility, state: State<'_, AppState>) {
    state.rqs.lock().unwrap().change_visibility(visibility);
}

/// `None` goes back to the user's Downloads folder.
#[tauri::command]
pub fn set_download_path(path: Option<String>, app: AppHandle, state: State<'_, AppState>) {
    let path = path.map(PathBuf::from);
    store::set_download_path(&app, path.as_ref());
    state.rqs.lock().unwrap().set_download_path(path);
}

/// `None` resets to the computer's hostname.
#[tauri::command]
pub fn set_device_name(name: Option<String>, app: AppHandle, state: State<'_, AppState>) {
    let normalized = name.map(|n| rqs_lib::normalize_device_name(&n));
    let effective_name = normalized
        .as_ref()
        .and_then(|s| if s.is_empty() { None } else { Some(s.as_str()) });

    store::set_device_name(&app, effective_name);
    if let Some(effective) = effective_name {
        state
            .rqs
            .lock()
            .unwrap()
            .set_device_name(effective.to_string());
    } else {
        state.rqs.lock().unwrap().set_device_name(String::new());
    }
}

#[tauri::command]
pub fn set_keep_running(enabled: bool, app: AppHandle) {
    store::set_keep_running(&app, enabled);
}

#[tauri::command]
pub fn set_file_manager_integration(enabled: bool, app: AppHandle) -> Result<(), String> {
    let result = if enabled {
        integrations::install(&app)
    } else {
        integrations::uninstall(&app)
    };

    result.map_err(|e| format!("Couldn't update the file manager integration: {e}"))
}

#[tauri::command]
pub async fn start_discovery(state: State<'_, AppState>) -> Result<(), String> {
    state
        .rqs
        .lock()
        .unwrap()
        .discovery(state.dch_sender.clone())
        .map_err(|e| format!("Couldn't start discovery: {e}"))
}

#[tauri::command]
pub fn stop_discovery(state: State<'_, AppState>) {
    state.rqs.lock().unwrap().stop_discovery();
}

#[tauri::command]
pub async fn send_payload(info: SendInfo, state: State<'_, AppState>) -> Result<(), String> {
    state
        .sender_file
        .send(info)
        .await
        .map_err(|e| format!("Couldn't send: {e}"))
}

#[tauri::command]
pub fn transfer_action(id: String, action: ChannelAction, state: State<'_, AppState>) {
    send_action(&state, id, action);
}

/// Files passed on the command line at startup, handed out once.
#[tauri::command]
pub fn take_pending_files(state: State<'_, PendingFiles>) -> Vec<String> {
    std::mem::take(&mut *state.0.lock().unwrap())
}

/// Connect to a Wi-Fi network using NetworkManager.
#[tauri::command]
pub async fn connect_wifi(network: WifiNetwork) -> Result<(), String> {
    wifi::connect_wifi(network).await
}

pub fn send_action(state: &AppState, id: String, action: ChannelAction) {
    let _ = state.message_sender.send(ChannelMessage {
        id,
        direction: ChannelDirection::FrontToLib,
        action: Some(action),
        ..Default::default()
    });
}

#[tauri::command]
pub fn trust_device(name: String, app: AppHandle) {
    let mut devices = store::trusted_devices(&app);
    if !devices.contains(&name) {
        devices.push(name);
        store::set_trusted_devices(&app, &devices);
    }
}

#[tauri::command]
pub fn untrust_device(name: String, app: AppHandle) {
    let mut devices = store::trusted_devices(&app);
    devices.retain(|d| d != &name);
    store::set_trusted_devices(&app, &devices);
}

#[tauri::command]
pub fn set_auto_open_links(enabled: bool, app: AppHandle) {
    store::set_auto_open_links(&app, enabled);
}

#[tauri::command]
pub fn set_auto_copy_text(enabled: bool, app: AppHandle) {
    store::set_auto_copy_text(&app, enabled);
}
