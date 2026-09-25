//! Persistent settings, stored in `.settings.json` in the app data directory.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use rqs_lib::Visibility;
use tauri::{AppHandle, Wry};
use tauri_plugin_store::{Store, StoreExt};

const VISIBILITY: &str = "visibility";
const DOWNLOAD_PATH: &str = "download_path";
const KEEP_RUNNING: &str = "keep_running";
/// Fixed TCP port, for firewalls. Only settable by editing the file.
const PORT: &str = "port";
/// Log level override. Only settable by editing the file.
const LOG_LEVEL: &str = "debug_level";

fn store(app: &AppHandle) -> Arc<Store<Wry>> {
    app.store_builder(".settings.json")
        .auto_save(Duration::from_millis(100))
        .build()
        .expect("settings store can be opened")
}

pub fn visibility(app: &AppHandle) -> Visibility {
    store(app)
        .get(VISIBILITY)
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or(Visibility::Visible)
}

pub fn set_visibility(app: &AppHandle, visibility: Visibility) {
    store(app).set(VISIBILITY, serde_json::json!(visibility));
}

pub fn download_path(app: &AppHandle) -> Option<PathBuf> {
    store(app)
        .get(DOWNLOAD_PATH)
        .and_then(|v| v.as_str().map(PathBuf::from))
}

pub fn set_download_path(app: &AppHandle, path: Option<&PathBuf>) {
    match path {
        Some(path) => store(app).set(DOWNLOAD_PATH, path.to_string_lossy().as_ref()),
        None => {
            store(app).delete(DOWNLOAD_PATH);
        }
    }
}

/// Whether closing the window keeps the app running in the background.
pub fn keep_running(app: &AppHandle) -> bool {
    store(app)
        .get(KEEP_RUNNING)
        .and_then(|v| v.as_bool())
        .unwrap_or(true)
}

pub fn set_keep_running(app: &AppHandle, enabled: bool) {
    store(app).set(KEEP_RUNNING, enabled);
}

pub fn port(app: &AppHandle) -> Option<u32> {
    store(app)
        .get(PORT)
        .and_then(|v| v.as_u64())
        .and_then(|v| u32::try_from(v).ok())
}

pub fn log_level(app: &AppHandle) -> Option<String> {
    store(app)
        .get(LOG_LEVEL)
        .and_then(|v| v.as_str().map(String::from))
}
