//! Persistent settings, stored in `.settings.json` in the app data directory.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use rqs_lib::{RemoteDeviceInfo, Visibility};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Wry};
use tauri_plugin_store::{Store, StoreExt};

const VISIBILITY: &str = "visibility";
const DOWNLOAD_PATH: &str = "download_path";
const DEVICE_NAME: &str = "device_name";
const KEEP_RUNNING: &str = "keep_running";
/// Fixed ports, for firewalls. Read at startup.
const PORT: &str = "port";
const LOCALSEND_PORT: &str = "localsend_port";
/// Log level override. Only settable by editing the file.
const LOG_LEVEL: &str = "debug_level";
const TRUSTED_DEVICES: &str = "trusted_devices";
const AUTO_OPEN_LINKS: &str = "auto_open_links";
const AUTO_COPY_TEXT: &str = "auto_copy_text";

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

pub fn device_name(app: &AppHandle) -> Option<String> {
    store(app)
        .get(DEVICE_NAME)
        .and_then(|v| v.as_str().map(String::from))
}

pub fn set_device_name(app: &AppHandle, name: Option<&str>) {
    match name {
        Some(name) => store(app).set(DEVICE_NAME, name),
        None => {
            store(app).delete(DEVICE_NAME);
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

/// Quick Share's TCP port; any free one when unset.
pub fn port(app: &AppHandle) -> Option<u16> {
    get_port(app, PORT)
}

pub fn set_port(app: &AppHandle, port: Option<u16>) {
    set_port_value(app, PORT, port);
}

/// LocalSend's HTTPS port; the protocol's default when unset.
pub fn localsend_port(app: &AppHandle) -> Option<u16> {
    get_port(app, LOCALSEND_PORT)
}

pub fn set_localsend_port(app: &AppHandle, port: Option<u16>) {
    set_port_value(app, LOCALSEND_PORT, port);
}

fn get_port(app: &AppHandle, key: &str) -> Option<u16> {
    store(app)
        .get(key)
        .and_then(|v| v.as_u64())
        .and_then(|v| u16::try_from(v).ok())
        .filter(|&port| port != 0)
}

fn set_port_value(app: &AppHandle, key: &str, port: Option<u16>) {
    match port {
        Some(port) => store(app).set(key, port),
        None => {
            store(app).delete(key);
        }
    }
}

pub fn log_level(app: &AppHandle) -> Option<String> {
    store(app)
        .get(LOG_LEVEL)
        .and_then(|v| v.as_str().map(String::from))
}

/// A device whose transfers are accepted without asking.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrustedDevice {
    pub name: String,
    /// LocalSend devices are trusted by their certificate. Quick Share ones,
    /// without it, by name.
    pub fingerprint: Option<String>,
}

impl TrustedDevice {
    /// Whether `source` is this device.
    pub fn matches(&self, source: &RemoteDeviceInfo, localsend: bool) -> bool {
        match (&self.fingerprint, localsend) {
            (Some(trusted), true) => source.fingerprint.as_ref() == Some(trusted),
            (None, false) => source.name == self.name,
            // A LocalSend sender can't borrow a Quick Share device's trust by
            // taking its name, nor the other way round.
            _ => false,
        }
    }
}

pub fn trusted_devices(app: &AppHandle) -> Vec<TrustedDevice> {
    store(app)
        .get(TRUSTED_DEVICES)
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default()
}

pub fn set_trusted_devices(app: &AppHandle, devices: &[TrustedDevice]) {
    store(app).set(TRUSTED_DEVICES, serde_json::json!(devices));
}

pub fn auto_open_links(app: &AppHandle) -> bool {
    store(app)
        .get(AUTO_OPEN_LINKS)
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}

pub fn set_auto_open_links(app: &AppHandle, enabled: bool) {
    store(app).set(AUTO_OPEN_LINKS, enabled);
}

pub fn auto_copy_text(app: &AppHandle) -> bool {
    store(app)
        .get(AUTO_COPY_TEXT)
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}

pub fn set_auto_copy_text(app: &AppHandle, enabled: bool) {
    store(app).set(AUTO_COPY_TEXT, enabled);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sender(name: &str, fingerprint: Option<&str>) -> RemoteDeviceInfo {
        RemoteDeviceInfo {
            name: name.into(),
            device_type: rqs_lib::DeviceType::Phone,
            fingerprint: fingerprint.map(Into::into),
        }
    }

    #[test]
    fn trusts_localsend_devices_by_certificate_only() {
        let phone = TrustedDevice {
            name: "Phone".into(),
            fingerprint: Some("AA".into()),
        };
        assert!(phone.matches(&sender("Renamed", Some("AA")), true));
        assert!(!phone.matches(&sender("Phone", Some("BB")), true));
        assert!(!phone.matches(&sender("Phone", None), true));
        assert!(!phone.matches(&sender("Phone", None), false));
    }

    #[test]
    fn trusts_quick_share_devices_by_name_only() {
        let pixel = TrustedDevice {
            name: "Pixel".into(),
            fingerprint: None,
        };
        assert!(pixel.matches(&sender("Pixel", None), false));
        assert!(!pixel.matches(&sender("Other", None), false));
        // A LocalSend sender calling itself "Pixel" isn't the Pixel.
        assert!(!pixel.matches(&sender("Pixel", Some("AA")), true));
        assert!(!pixel.matches(&sender("Pixel", None), true));
    }
}
