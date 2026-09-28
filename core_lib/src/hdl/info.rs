use std::fs::File;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use super::TextPayloadType;
use crate::utils::RemoteDeviceInfo;

#[derive(Debug, Clone, Deserialize, Serialize, TS)]
#[ts(export)]
pub struct WifiNetwork {
    pub ssid: String,
    pub password: String,
    pub security: WifiSecurity,
    pub hidden: bool,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "UPPERCASE")]
pub enum WifiSecurity {
    Open,
    WpaPsk,
    Wep,
    Sae,
}

#[derive(Debug)]
pub struct InternalFileInfo {
    pub payload_id: i64,
    pub file_url: PathBuf,
    pub bytes_transferred: i64,
    pub total_size: i64,
    pub file: Option<File>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, TS)]
#[ts(export)]
pub struct TransferMetadata {
    pub source: Option<RemoteDeviceInfo>,
    pub pin_code: Option<String>,

    pub destination: Option<String>,
    pub files: Option<Vec<String>>,

    pub text_type: Option<TextPayloadType>,
    pub text_description: Option<String>,
    pub text_payload: Option<String>,
    pub wifi: Option<WifiNetwork>,

    #[ts(type = "number")]
    pub total_bytes: u64,
    #[ts(type = "number")]
    pub ack_bytes: u64,

    /// Why a transfer failed, when we know something more useful than that
    /// the connection was lost.
    pub reason: Option<String>,
}
