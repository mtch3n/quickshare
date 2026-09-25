use std::sync::Arc;
use std::time::Duration;

use bluer::adv::Advertisement;
use bluer::{Uuid, UuidExt};
use bytes::Bytes;
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

use super::Visibility;
use super::gatt::SERVICE_UUID;
use crate::utils::{DeviceType, SERVICE_ID_HASH, encode_endpoint_info};

const SERVICE_DATA: Bytes = Bytes::from_static(&[
    252, 18, 142, 1, 66, 0, 0, 0, 0, 0, 0, 0, 0, 0, 191, 45, 91, 160, 225, 216, 117, 36, 202, 0,
]);

const INNER_NAME: &str = "BleAdvertiser";

#[derive(Debug, Clone)]
pub struct BleAdvertiser {
    adapter: Arc<bluer::Adapter>,
}

impl BleAdvertiser {
    pub async fn new() -> Result<Self, anyhow::Error> {
        let session = bluer::Session::new().await?;
        let adapter = session.default_adapter().await?;
        adapter.set_powered(true).await?;

        Ok(Self {
            adapter: Arc::new(adapter),
        })
    }

    pub async fn run(&self, ctk: CancellationToken) -> Result<(), anyhow::Error> {
        info!(
            "{INNER_NAME}: advertising on Bluetooth adapter {} with address {}",
            self.adapter.name(),
            self.adapter.address().await?
        );

        let service_uuid = Uuid::from_u16(0xFE2C);
        let handle = self
            .adapter
            .advertise(self.get_advertisment(service_uuid, SERVICE_DATA))
            .await?;
        ctk.cancelled().await;
        info!("{INNER_NAME}: tracker cancelled, returning");
        drop(handle);

        Ok(())
    }

    fn get_advertisment(&self, service_uuid: Uuid, adv_data: Bytes) -> Advertisement {
        Advertisement {
            advertisement_type: bluer::adv::Type::Broadcast,
            service_data: [(service_uuid, adv_data.into())].into(),
            ..Default::default()
        }
    }
}

const RX_INNER_NAME: &str = "ReceiverAdvertiser";

/// Mediums advertisement header: version 2, socket version 2, not a fast advertisement.
const MEDIUMS_HEADER: u8 = 0x48;
/// Connections advertisement header: version 1, PCP 3 (point to point).
const CONNECTIONS_HEADER: u8 = 0x23;
/// Longest endpoint info a (non-fast) connections advertisement may carry.
const MAX_ENDPOINT_INFO_LEN: usize = 131;
/// Header byte, 16 identity bytes and the name length byte.
const ENDPOINT_INFO_OVERHEAD: usize = 18;

/// A connectable advertising set is consumed when a phone connects, so it is
/// registered again periodically to stay discoverable for the next transfer.
const READVERTISE_INTERVAL: Duration = Duration::from_secs(30);
/// Phones connect noticeably faster than with BlueZ's default ~1s interval.
const MIN_INTERVAL: Duration = Duration::from_millis(100);
const MAX_INTERVAL: Duration = Duration::from_millis(150);

/// The 0xFEF3 service data announcing us as a Quick Share receiver: a Nearby
/// mediums advertisement wrapping the connections advertisement with our
/// endpoint id (the one mDNS uses) and endpoint info. Also served on the GATT
/// advertisement slot, see `gatt.rs`.
pub fn receiver_advertisement(endpoint_id: [u8; 4], name: &str) -> Vec<u8> {
    let name = &name[..name.floor_char_boundary(MAX_ENDPOINT_INFO_LEN - ENDPOINT_INFO_OVERHEAD)];
    let endpoint_info = encode_endpoint_info(DeviceType::Laptop, name);

    let mut connections = vec![CONNECTIONS_HEADER];
    connections.extend_from_slice(&SERVICE_ID_HASH);
    connections.extend_from_slice(&endpoint_id);
    connections.push(endpoint_info.len() as u8);
    connections.extend_from_slice(&endpoint_info);
    // No Bluetooth Classic MAC (all zeros, as Nearby writes it): a real one makes
    // the phone try an RFCOMM connection we don't serve before falling back to BLE.
    connections.extend_from_slice(&[0; 6]);
    // UWB address length, then the extra field (not WebRTC connectable).
    connections.extend_from_slice(&[0, 0]);

    let mut advertisement = vec![MEDIUMS_HEADER];
    advertisement.extend_from_slice(&SERVICE_ID_HASH);
    advertisement.extend_from_slice(&(connections.len() as u32).to_be_bytes());
    advertisement.extend(connections);
    // Device token. No extra fields follow: an L2CAP PSM there would make the
    // phone try an L2CAP channel before the GATT socket.
    advertisement.extend(rand::random::<[u8; 2]>());
    advertisement
}

/// Advertises the receiver over BLE (service data under 0xFEF3) while we are
/// visible, so phones that left Wi-Fi to share can still list us.
pub struct ReceiverAdvertiser {
    adapter: bluer::Adapter,
    advertisement: Vec<u8>,
    visibility: watch::Receiver<Visibility>,
}

impl ReceiverAdvertiser {
    pub fn new(
        adapter: bluer::Adapter,
        advertisement: Vec<u8>,
        visibility: watch::Receiver<Visibility>,
    ) -> Self {
        Self {
            adapter,
            advertisement,
            visibility,
        }
    }

    pub async fn run(mut self, ctk: CancellationToken) {
        info!(
            "{RX_INNER_NAME}: service starting on {} ({} bytes)",
            self.adapter.name(),
            self.advertisement.len()
        );

        let mut failing = false;
        loop {
            let visible = *self.visibility.borrow_and_update() != Visibility::Invisible;

            let handle = if visible {
                match self.adapter.advertise(self.build()).await {
                    Ok(handle) => {
                        if failing {
                            info!("{RX_INNER_NAME}: advertising again");
                        }
                        failing = false;
                        Some(handle)
                    }
                    Err(e) => {
                        // Typically the adapter is off; retry quietly until it's back.
                        if !failing {
                            warn!("{RX_INNER_NAME}: can't advertise, retrying: {e}");
                        }
                        failing = true;
                        None
                    }
                }
            } else {
                None
            };

            tokio::select! {
                _ = ctk.cancelled() => {
                    info!("{RX_INNER_NAME}: tracker cancelled, returning");
                    return;
                }
                r = self.visibility.changed() => {
                    if r.is_err() {
                        return;
                    }
                }
                _ = tokio::time::sleep(READVERTISE_INTERVAL), if visible => {}
            }

            drop(handle);
        }
    }

    fn build(&self) -> Advertisement {
        Advertisement {
            advertisement_type: bluer::adv::Type::Peripheral,
            service_data: [(Uuid::from_u16(SERVICE_UUID), self.advertisement.clone())].into(),
            discoverable: Some(true),
            min_interval: Some(MIN_INTERVAL),
            max_interval: Some(MAX_INTERVAL),
            ..Default::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::parse_endpoint_info;

    /// Splits a receiver advertisement into (endpoint id, endpoint info, bytes
    /// after the endpoint info, device token), checking the fixed fields.
    fn parse(adv: &[u8]) -> ([u8; 4], Vec<u8>, Vec<u8>, Vec<u8>) {
        assert_eq!(adv[0], MEDIUMS_HEADER);
        assert_eq!(adv[1..4], SERVICE_ID_HASH);
        let size = u32::from_be_bytes(adv[4..8].try_into().unwrap()) as usize;
        let (connections, token) = adv[8..].split_at(size);

        assert_eq!(connections[0], CONNECTIONS_HEADER);
        assert_eq!(connections[1..4], SERVICE_ID_HASH);
        let endpoint_id = connections[4..8].try_into().unwrap();
        let info_len = connections[8] as usize;
        let info = connections[9..9 + info_len].to_vec();
        let rest = connections[9 + info_len..].to_vec();

        (endpoint_id, info, rest, token.to_vec())
    }

    #[test]
    fn receiver_advertisement_layout() {
        let adv = receiver_advertisement(*b"ABCD", "my-laptop");
        let (endpoint_id, info, rest, token) = parse(&adv);

        assert_eq!(&endpoint_id, b"ABCD");
        assert_eq!(
            parse_endpoint_info(&info).unwrap(),
            (DeviceType::Laptop, Some("my-laptop".to_string()))
        );
        // Zero MAC, no UWB address, empty extra field.
        assert_eq!(rest, [0; 8]);
        assert_eq!(token.len(), 2);
    }

    #[test]
    fn long_names_fit_the_endpoint_info_limit() {
        let adv = receiver_advertisement(*b"ABCD", &"é".repeat(200));
        let (_, info, rest, token) = parse(&adv);

        assert!(info.len() <= MAX_ENDPOINT_INFO_LEN);
        let (_, name) = parse_endpoint_info(&info).unwrap();
        assert!(name.unwrap().chars().all(|c| c == 'é'));
        assert_eq!(rest.len(), 8);
        assert_eq!(token.len(), 2);
    }
}
