//! Commands invoked by the frontend.

use std::path::PathBuf;

use rqs_lib::channel::{ChannelAction, ChannelDirection, ChannelMessage};
use rqs_lib::{SendInfo, Visibility};
use serde::Serialize;
use tauri::{AppHandle, State};

use crate::{AppState, PendingFiles, integrations, store};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    device_name: String,
    visibility: Visibility,
    download_path: String,
    keep_running: bool,
    file_manager_integration: bool,
}

#[tauri::command]
pub fn get_settings(app: AppHandle) -> Settings {
    Settings {
        device_name: rqs_lib::hostname(),
        visibility: store::visibility(&app),
        download_path: rqs_lib::get_download_dir().to_string_lossy().into_owned(),
        keep_running: store::keep_running(&app),
        file_manager_integration: integrations::installed(&app),
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

pub fn send_action(state: &AppState, id: String, action: ChannelAction) {
    let _ = state.message_sender.send(ChannelMessage {
        id,
        direction: ChannelDirection::FrontToLib,
        action: Some(action),
        ..Default::default()
    });
}
