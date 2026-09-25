use std::fs::{File, OpenOptions};
use std::net::{IpAddr, Ipv4Addr, UdpSocket};
use std::path::{Path, PathBuf};

use anyhow::anyhow;
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::{CUSTOM_DEVICE_NAME, CUSTOM_DOWNLOAD};

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Serialize, TS)]
#[ts(export)]
pub enum DeviceType {
    Unknown = 0,
    Phone = 1,
    Tablet = 2,
    Laptop = 3,
}

impl DeviceType {
    pub fn from_raw_value(value: u8) -> Self {
        match value {
            1 => DeviceType::Phone,
            2 => DeviceType::Tablet,
            3 => DeviceType::Laptop,
            _ => DeviceType::Unknown,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, TS)]
#[ts(export)]
pub struct RemoteDeviceInfo {
    pub name: String,
    pub device_type: DeviceType,
}

/// Endpoint info as advertised over mDNS and sent in the ConnectionRequest:
/// 1 byte Version(3 bits)|Visibility(1 bit)|Device type(3 bits)|Reserved(1 bit),
/// 16 random bytes, then the UTF-8 name prefixed with its 1-byte length.
pub fn encode_endpoint_info(device_type: DeviceType, name: &str) -> Vec<u8> {
    let name = &name[..name.floor_char_boundary(u8::MAX as usize)];

    let mut info = vec![(device_type as u8 & 0b111) << 1];
    info.extend(rand::random::<[u8; 16]>());
    info.push(name.len() as u8);
    info.extend_from_slice(name.as_bytes());
    info
}

/// Parses endpoint info. The name is absent when the peer hides it (visibility
/// bit set) or only sends the 17-byte header.
pub fn parse_endpoint_info(info: &[u8]) -> Result<(DeviceType, Option<String>), anyhow::Error> {
    if info.len() < 17 {
        return Err(anyhow!("endpoint info too short ({} bytes)", info.len()));
    }

    let device_type = DeviceType::from_raw_value((info[0] >> 1) & 0b111);
    if (info[0] >> 4) & 1 == 1 || info.len() == 17 {
        return Ok((device_type, None));
    }

    let name_len = info[17] as usize;
    let name = info
        .get(18..18 + name_len)
        .ok_or_else(|| anyhow!("endpoint name length out of range"))?;

    Ok((
        device_type,
        Some(String::from_utf8_lossy(name).into_owned()),
    ))
}

/// First 3 bytes of SHA-256("NearbySharing"), the Quick Share service id.
pub const SERVICE_ID_HASH: [u8; 3] = [0xFC, 0x9F, 0x5E];

pub fn gen_mdns_name(endpoint_id: [u8; 4]) -> String {
    let mut name_b = Vec::new();

    let pcp: [u8; 1] = [0x23];
    name_b.extend_from_slice(&pcp);

    name_b.extend_from_slice(&endpoint_id);

    name_b.extend_from_slice(&SERVICE_ID_HASH);

    let unknown_bytes: [u8; 2] = [0x00, 0x00];
    name_b.extend_from_slice(&unknown_bytes);

    URL_SAFE_NO_PAD.encode(&name_b)
}

pub fn to_four_digit_string(bytes: &[u8]) -> String {
    let k_hash_modulo = 9973;
    let k_hash_base_multiplier = 31;

    let mut hash = 0;
    let mut multiplier = 1;
    for &byte in bytes {
        let byte = byte as i8 as i32;
        hash = (hash + byte * multiplier) % k_hash_modulo;
        multiplier = (multiplier * k_hash_base_multiplier) % k_hash_modulo;
    }

    format!("{:04}", hash.abs())
}

pub fn gen_random(size: usize) -> Vec<u8> {
    let mut data = vec![0; size];
    rand::fill(&mut data[..]);

    data
}

pub fn get_download_dir() -> PathBuf {
    let cdown = CUSTOM_DOWNLOAD.read();
    match cdown {
        Ok(mg) => {
            if mg.is_some() {
                return mg.as_ref().unwrap().to_path_buf();
            }
        }
        Err(_) => {
            // TODO
        }
    }

    if let Some(user_dirs) = directories::UserDirs::new() {
        if let Some(dd) = user_dirs.download_dir() {
            return dd.to_path_buf();
        }

        return user_dirs.home_dir().to_path_buf();
    }

    Path::new("/").to_path_buf()
}

/// The address phones on our LAN reach us at, offered for Wi-Fi upgrades: the
/// source address of the default route (which skips container bridges),
/// else the first private address.
pub fn lan_ipv4() -> Option<Ipv4Addr> {
    let routed = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))
        .and_then(|socket| {
            // Only selects a route, nothing is sent. TEST-NET-1 has no route of
            // its own, so the default route is picked.
            socket.connect((Ipv4Addr::new(192, 0, 2, 1), 9))?;
            socket.local_addr()
        })
        .ok()
        .and_then(|addr| match addr.ip() {
            IpAddr::V4(ip) if !ip.is_unspecified() && !ip.is_loopback() => Some(ip),
            _ => None,
        });

    routed.or_else(|| {
        if_addrs::get_if_addrs()
            .ok()?
            .into_iter()
            .find_map(|iface| match iface.ip() {
                IpAddr::V4(ip) if ip.is_private() => Some(ip),
                _ => None,
            })
    })
}

pub fn is_not_self_ip(ip_address: &Ipv4Addr) -> bool {
    let ip = IpAddr::V4(*ip_address);
    if_addrs::get_if_addrs()
        .map(|addrs| addrs.iter().all(|a| a.ip() != ip))
        .unwrap_or(true)
}

/// A file name sent by a peer, reduced to its last path component so it can't
/// escape the download directory.
pub fn sanitize_file_name(name: &str) -> Option<String> {
    let name = Path::new(name).file_name()?.to_str()?;
    (!name.is_empty()).then(|| name.to_owned())
}

/// Creates `path` without overwriting anything, falling back to `name (1).ext`,
/// `name (2).ext`, ... when it already exists.
pub fn create_unique_file(path: &Path) -> std::io::Result<(PathBuf, File)> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }

    let stem = path.file_stem().unwrap_or_default().to_string_lossy();
    let ext = path
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()))
        .unwrap_or_default();

    let mut candidate = path.to_path_buf();
    for n in 1.. {
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&candidate)
        {
            Ok(file) => return Ok((candidate, file)),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                candidate = path.with_file_name(format!("{stem} ({n}){ext}"));
            }
            Err(e) => return Err(e),
        }
    }

    unreachable!()
}

pub fn hostname() -> String {
    gethostname::gethostname().to_string_lossy().into_owned()
}

/// Expands directories into regular files recursively.
/// Returns tuples of (file_path, parent_folder_relative_to_base).
/// Skips symlinks and special files.
/// For a directory, parent_folder is the relative path from the directory's parent.
pub fn expand_directories(paths: &[String]) -> Vec<(String, Option<String>)> {
    let mut result = vec![];

    for path_str in paths {
        let path = Path::new(path_str);

        if path.is_dir() {
            // For directories, track the base path and expand recursively
            expand_dir_recursive(path, path, &mut result);
        } else if path.is_file() {
            // Not a directory, add as-is
            result.push((path_str.clone(), None));
        }
    }

    result
}

fn expand_dir_recursive(
    base_dir: &Path,
    current_dir: &Path,
    result: &mut Vec<(String, Option<String>)>,
) {
    if let Ok(entries) = std::fs::read_dir(current_dir) {
        for entry in entries.flatten() {
            let entry_path = entry.path();

            // Skip symlinks
            if entry_path.is_symlink() {
                continue;
            }

            if entry_path.is_file() {
                if let Some(file_path_str) = entry_path.to_str() {
                    // Compute relative path from base directory's parent
                    let parent_folder = entry_path
                        .parent()
                        .and_then(|p| {
                            base_dir.parent().and_then(|base_parent| {
                                p.strip_prefix(base_parent)
                                    .ok()
                                    .and_then(|rel| rel.to_str())
                            })
                        })
                        .map(|s| s.to_string());

                    result.push((file_path_str.to_string(), parent_folder));
                }
            } else if entry_path.is_dir() {
                // Recursively expand subdirectories
                expand_dir_recursive(base_dir, &entry_path, result);
            }
        }
    }
}

/// Normalizes a device name: trims it, caps at 64 characters, and returns empty if only whitespace.
pub fn normalize_device_name(name: &str) -> String {
    name.trim().chars().take(64).collect::<String>()
}

/// Returns the effective device name: normalized custom name if set, otherwise the hostname.
pub fn effective_device_name() -> String {
    if let Ok(mg) = CUSTOM_DEVICE_NAME.read()
        && let Some(name) = mg.as_ref()
        && !name.is_empty()
    {
        return name.clone();
    }
    hostname()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn service_id_hash_matches_nearby_sharing() {
        use sha2::{Digest, Sha256};

        assert_eq!(Sha256::digest(b"NearbySharing")[..3], SERVICE_ID_HASH);
    }

    #[test]
    fn endpoint_info_roundtrip() {
        let info = encode_endpoint_info(DeviceType::Laptop, "a_device_name");
        let (device_type, name) = parse_endpoint_info(&info).unwrap();

        assert_eq!(device_type, DeviceType::Laptop);
        assert_eq!(name.as_deref(), Some("a_device_name"));
    }

    #[test]
    fn endpoint_info_without_name() {
        let mut info = encode_endpoint_info(DeviceType::Phone, "");
        info.truncate(17);
        assert_eq!(
            parse_endpoint_info(&info).unwrap(),
            (DeviceType::Phone, None)
        );

        // Visibility bit set: the name must be ignored.
        let mut info = encode_endpoint_info(DeviceType::Phone, "hidden");
        info[0] |= 1 << 4;
        assert_eq!(
            parse_endpoint_info(&info).unwrap(),
            (DeviceType::Phone, None)
        );

        assert!(parse_endpoint_info(&[0; 16]).is_err());
    }

    #[test]
    fn endpoint_name_is_truncated_on_char_boundary() {
        let name = "é".repeat(200);
        let info = encode_endpoint_info(DeviceType::Laptop, &name);
        let (_, parsed) = parse_endpoint_info(&info).unwrap();

        assert_eq!(parsed.unwrap().len(), 254);
    }

    #[test]
    fn peer_file_names_stay_inside_the_download_dir() {
        assert_eq!(
            sanitize_file_name("photo.jpg").as_deref(),
            Some("photo.jpg")
        );
        assert_eq!(
            sanitize_file_name("../../.bashrc").as_deref(),
            Some(".bashrc")
        );
        assert_eq!(sanitize_file_name("/etc/passwd").as_deref(), Some("passwd"));
        assert_eq!(sanitize_file_name(".."), None);
        assert_eq!(sanitize_file_name(""), None);
    }

    #[test]
    fn unique_file_names() {
        let dir = std::env::temp_dir().join(format!("rqs-test-{}", rand::random::<u32>()));
        std::fs::create_dir_all(&dir).unwrap();

        let (first, _) = create_unique_file(&dir.join("a.txt")).unwrap();
        let (second, _) = create_unique_file(&dir.join("a.txt")).unwrap();
        assert_eq!(first, dir.join("a.txt"));
        assert_eq!(second, dir.join("a (1).txt"));

        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn parent_folder_cannot_escape_download_dir() {
        let download_dir = Path::new("/home/user/Downloads");

        // Test normal parent_folder
        let mut path = download_dir.to_path_buf();
        if let Some(sanitized) = sanitize_file_name("folder") {
            path.push(sanitized);
        }
        path.push("file.txt");
        // Path should be /home/user/Downloads/folder/file.txt
        assert!(path.starts_with(download_dir));

        // Test escaped parent_folder with ".." - should be filtered out
        let mut path = download_dir.to_path_buf();
        for component in "..".split('/') {
            if !component.is_empty()
                && component != "."
                && component != ".."
                && let Some(sanitized) = sanitize_file_name(component)
            {
                path.push(sanitized);
            }
        }
        path.push("file.txt");
        // Path should remain /home/user/Downloads/file.txt (no ".." added)
        assert!(path.starts_with(download_dir));

        // Test deep nested folders
        let mut path = download_dir.to_path_buf();
        for component in "folder1/subfolder/deep".split('/') {
            if !component.is_empty()
                && component != "."
                && component != ".."
                && let Some(sanitized) = sanitize_file_name(component)
            {
                path.push(sanitized);
            }
        }
        path.push("file.txt");
        // Path should be /home/user/Downloads/folder1/subfolder/deep/file.txt
        assert!(path.starts_with(download_dir));
    }
}
