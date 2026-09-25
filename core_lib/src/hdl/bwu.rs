//! Nearby Connections bandwidth upgrade to Wi-Fi LAN, with us as the initiator.
//!
//! Used to move BLE sessions (tens of KB/s) onto TCP once they are encrypted:
//!
//! 1. Over the BLE channel, encrypted: UPGRADE_PATH_AVAILABLE with our LAN
//!    address and the TCP listener's port.
//! 2. The phone connects and sends a plaintext CLIENT_INTRODUCTION with its
//!    endpoint id; the TCP server hands that connection to the waiting session
//!    through [`UpgradeRegistry`], which answers CLIENT_INTRODUCTION_ACK.
//! 3. Both sides send LAST_WRITE_TO_PRIOR_CHANNEL over BLE and answer the
//!    peer's with SAFE_TO_CLOSE_PRIOR_CHANNEL. Frames the phone sent before its
//!    LAST_WRITE are still processed.
//! 4. On the peer's SAFE_TO_CLOSE, each side sends a plaintext DISCONNECTION on
//!    BLE and reads the peer's; without ours the phone stalls ~10s.
//! 5. The session continues over TCP with the same keys and sequence numbers.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use prost::Message;
use tokio::sync::oneshot;

use super::Transport;
use crate::location_nearby_connections::bandwidth_upgrade_negotiation_frame::upgrade_path_info::{
    Medium, WifiLanSocket,
};
use crate::location_nearby_connections::bandwidth_upgrade_negotiation_frame::{
    ClientIntroductionAck, EventType, UpgradePathInfo,
};
use crate::location_nearby_connections::{
    BandwidthUpgradeNegotiationFrame, DisconnectionFrame, OfflineFrame, V1Frame, offline_frame,
    v1_frame,
};

/// Where BLE sessions invite phones to continue over Wi-Fi.
#[derive(Debug, Clone)]
pub struct WifiLanUpgrade {
    /// The TCP listener's port.
    pub port: u16,
    pub registry: UpgradeRegistry,
}

/// Sessions waiting for their phone to connect over Wi-Fi, by the phone's endpoint id.
#[derive(Debug, Clone, Default)]
pub struct UpgradeRegistry(Arc<Mutex<HashMap<String, oneshot::Sender<Transport>>>>);

impl UpgradeRegistry {
    /// Starts waiting for `endpoint_id` to connect. Dropping the result stops waiting.
    pub fn expect(&self, endpoint_id: String) -> PendingUpgrade {
        let (sender, receiver) = oneshot::channel();
        self.0.lock().unwrap().insert(endpoint_id, sender);

        PendingUpgrade {
            registry: self.clone(),
            receiver,
        }
    }

    /// Hands an upgraded connection to its session. False when none is waiting.
    pub fn deliver(&self, endpoint_id: &str, transport: Transport) -> bool {
        let sender = self.0.lock().unwrap().remove(endpoint_id);
        sender.is_some_and(|s| s.send(transport).is_ok())
    }
}

pub struct PendingUpgrade {
    registry: UpgradeRegistry,
    receiver: oneshot::Receiver<Transport>,
}

impl PendingUpgrade {
    /// Resolves once the phone's connection arrives. Cancel-safe.
    pub async fn connection(&mut self) -> Option<Transport> {
        (&mut self.receiver).await.ok()
    }
}

impl Drop for PendingUpgrade {
    fn drop(&mut self) {
        self.receiver.close();
        self.registry
            .0
            .lock()
            .unwrap()
            .retain(|_, sender| !sender.is_closed());
    }
}

/// Offers our TCP listener, sent encrypted over the current channel.
pub fn upgrade_path_available(ip: std::net::Ipv4Addr, port: u16) -> OfflineFrame {
    negotiation(BandwidthUpgradeNegotiationFrame {
        event_type: Some(EventType::UpgradePathAvailable.into()),
        upgrade_path_info: Some(UpgradePathInfo {
            medium: Some(Medium::WifiLan.into()),
            wifi_lan_socket: Some(WifiLanSocket {
                ip_address: Some(ip.octets().to_vec()),
                wifi_port: Some(port.into()),
            }),
            supports_client_introduction_ack: Some(true),
            ..Default::default()
        }),
        ..Default::default()
    })
}

/// Sent plaintext on the new connection, in reply to its CLIENT_INTRODUCTION.
pub fn client_introduction_ack() -> OfflineFrame {
    negotiation(BandwidthUpgradeNegotiationFrame {
        event_type: Some(EventType::ClientIntroductionAck.into()),
        client_introduction_ack: Some(ClientIntroductionAck {}),
        ..Default::default()
    })
}

/// LAST_WRITE_TO_PRIOR_CHANNEL or SAFE_TO_CLOSE_PRIOR_CHANNEL.
pub fn event_frame(event: EventType) -> OfflineFrame {
    negotiation(BandwidthUpgradeNegotiationFrame {
        event_type: Some(event.into()),
        ..Default::default()
    })
}

/// The plaintext DISCONNECTION that ends the prior channel.
pub fn prior_channel_disconnection() -> OfflineFrame {
    OfflineFrame {
        version: Some(offline_frame::Version::V1.into()),
        v1: Some(V1Frame {
            r#type: Some(v1_frame::FrameType::Disconnection.into()),
            disconnection: Some(DisconnectionFrame {
                request_safe_to_disconnect: Some(false),
                ack_safe_to_disconnect: Some(false),
            }),
            ..Default::default()
        }),
    }
}

/// The bandwidth upgrade event `frame` carries, if it is one.
pub fn event(frame: &OfflineFrame) -> Option<EventType> {
    let v1 = frame.v1.as_ref()?;
    if v1.r#type() != v1_frame::FrameType::BandwidthUpgradeNegotiation {
        return None;
    }
    Some(v1.bandwidth_upgrade_negotiation.as_ref()?.event_type())
}

/// The phone's endpoint id when `frame` (plaintext) is a CLIENT_INTRODUCTION,
/// i.e. the connection is a session moving over from BLE.
pub fn client_introduction(frame: &[u8]) -> Option<String> {
    let frame = OfflineFrame::decode(frame).ok()?;
    if event(&frame)? != EventType::ClientIntroduction {
        return None;
    }
    frame
        .v1?
        .bandwidth_upgrade_negotiation?
        .client_introduction?
        .endpoint_id
}

fn negotiation(frame: BandwidthUpgradeNegotiationFrame) -> OfflineFrame {
    OfflineFrame {
        version: Some(offline_frame::Version::V1.into()),
        v1: Some(V1Frame {
            r#type: Some(v1_frame::FrameType::BandwidthUpgradeNegotiation.into()),
            bandwidth_upgrade_negotiation: Some(frame),
            ..Default::default()
        }),
    }
}

#[cfg(test)]
mod tests {
    use std::net::Ipv4Addr;

    use super::*;
    use crate::location_nearby_connections::ConnectionRequestFrame;
    use crate::location_nearby_connections::bandwidth_upgrade_negotiation_frame::ClientIntroduction;

    fn transport() -> Transport {
        Transport::new(tokio::io::duplex(16).0)
    }

    fn roundtrip(frame: &OfflineFrame) -> OfflineFrame {
        OfflineFrame::decode(&*frame.encode_to_vec()).unwrap()
    }

    #[test]
    fn upgrade_path_available_offers_wifi_lan() {
        let frame = roundtrip(&upgrade_path_available(Ipv4Addr::new(192, 168, 1, 7), 4242));
        assert_eq!(event(&frame), Some(EventType::UpgradePathAvailable));

        let info = frame
            .v1
            .unwrap()
            .bandwidth_upgrade_negotiation
            .unwrap()
            .upgrade_path_info
            .unwrap();
        assert_eq!(info.medium(), Medium::WifiLan);
        assert!(info.supports_client_introduction_ack());
        assert!(!info.supports_disabling_encryption());
        let socket = info.wifi_lan_socket.unwrap();
        assert_eq!(socket.ip_address(), [192, 168, 1, 7]);
        assert_eq!(socket.wifi_port(), 4242);
    }

    #[test]
    fn control_frames() {
        assert_eq!(
            event(&roundtrip(&client_introduction_ack())),
            Some(EventType::ClientIntroductionAck)
        );
        assert_eq!(
            event(&roundtrip(&event_frame(EventType::LastWriteToPriorChannel))),
            Some(EventType::LastWriteToPriorChannel)
        );

        let disconnection = roundtrip(&prior_channel_disconnection());
        assert_eq!(event(&disconnection), None);
        let v1 = disconnection.v1.unwrap();
        assert_eq!(v1.r#type(), v1_frame::FrameType::Disconnection);
        assert!(!v1.disconnection.unwrap().request_safe_to_disconnect());
    }

    #[test]
    fn recognizes_client_introductions() {
        let intro = negotiation(BandwidthUpgradeNegotiationFrame {
            event_type: Some(EventType::ClientIntroduction.into()),
            client_introduction: Some(ClientIntroduction {
                endpoint_id: Some("ABCD".into()),
                supports_disabling_encryption: Some(false),
            }),
            ..Default::default()
        });
        assert_eq!(
            client_introduction(&intro.encode_to_vec()).as_deref(),
            Some("ABCD")
        );

        let request = OfflineFrame {
            version: Some(offline_frame::Version::V1.into()),
            v1: Some(V1Frame {
                r#type: Some(v1_frame::FrameType::ConnectionRequest.into()),
                connection_request: Some(ConnectionRequestFrame {
                    endpoint_id: Some("ABCD".into()),
                    ..Default::default()
                }),
                ..Default::default()
            }),
        };
        assert_eq!(client_introduction(&request.encode_to_vec()), None);
        assert_eq!(
            client_introduction(&client_introduction_ack().encode_to_vec()),
            None
        );
        assert_eq!(client_introduction(&[0xFF, 0xFF]), None);
    }

    #[tokio::test]
    async fn registry_hands_over_connections() {
        let registry = UpgradeRegistry::default();
        assert!(!registry.deliver("ABCD", transport()));

        let mut pending = registry.expect("ABCD".into());
        assert!(!registry.deliver("WXYZ", transport()));
        assert!(registry.deliver("ABCD", transport()));
        assert!(pending.connection().await.is_some());
        // Delivered once only.
        assert!(!registry.deliver("ABCD", transport()));
    }

    #[test]
    fn dropped_waits_are_forgotten() {
        let registry = UpgradeRegistry::default();
        drop(registry.expect("ABCD".into()));

        assert!(registry.0.lock().unwrap().is_empty());
        assert!(!registry.deliver("ABCD", transport()));
    }
}
