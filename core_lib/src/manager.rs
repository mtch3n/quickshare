use serde::{Deserialize, Serialize};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::broadcast::Sender;
use tokio::sync::mpsc::Receiver;
use tokio_util::sync::CancellationToken;
use ts_rs::TS;

use crate::channel::{ChannelDirection, ChannelMessage, TransferType};
use crate::errors::AppError;
use crate::hdl::{
    InboundRequest, OutboundPayload, OutboundRequest, Protocol, State, Transport, UpgradeRegistry,
    client_introduction,
};
use crate::utils::RemoteDeviceInfo;

const INNER_NAME: &str = "TcpServer";

#[derive(Debug, Deserialize, Serialize, TS)]
#[ts(export)]
pub struct SendInfo {
    pub id: String,
    pub name: String,
    pub addr: String,
    pub protocol: Protocol,
    pub ob: OutboundPayload,
}

pub struct TcpServer {
    endpoint_id: [u8; 4],
    tcp_listener: TcpListener,
    sender: Sender<ChannelMessage>,
    connect_receiver: Receiver<SendInfo>,
    upgrades: UpgradeRegistry,
}

impl TcpServer {
    pub fn new(
        endpoint_id: [u8; 4],
        tcp_listener: TcpListener,
        sender: Sender<ChannelMessage>,
        connect_receiver: Receiver<SendInfo>,
        upgrades: UpgradeRegistry,
    ) -> Result<Self, anyhow::Error> {
        Ok(Self {
            endpoint_id,
            tcp_listener,
            sender,
            connect_receiver,
            upgrades,
        })
    }

    pub async fn run(&mut self, ctk: CancellationToken) -> Result<(), anyhow::Error> {
        info!("{INNER_NAME}: service starting");

        loop {
            let cctk = ctk.clone();

            tokio::select! {
                _ = ctk.cancelled() => {
                    info!("{INNER_NAME}: tracker cancelled, breaking");
                    break;
                }
                Some(i) = self.connect_receiver.recv() => {
                    info!("{INNER_NAME}: connect_receiver: got {:?}", i);
                    // Sending runs on its own so we keep accepting transfers meanwhile.
                    let endpoint_id = self.endpoint_id;
                    let sender = self.sender.clone();
                    tokio::spawn(async move {
                        if let Err(e) = connect(endpoint_id, sender, cctk, i).await {
                            error!("{INNER_NAME}: error sending: {e}");
                        }
                    });
                }
                r = self.tcp_listener.accept() => {
                    match r {
                        Ok((socket, remote_addr)) => {
                            trace!("{INNER_NAME}: new client: {remote_addr}");
                            tokio::spawn(accept(
                                Transport::new(socket),
                                remote_addr.to_string(),
                                self.sender.clone(),
                                self.upgrades.clone(),
                            ));
                        },
                        Err(err) => {
                            error!("{INNER_NAME}: error accepting: {}", err);
                            break;
                        }
                    }
                }
            }
        }

        Ok(())
    }
}

/// Serves an incoming connection: either a new transfer, or a BLE session
/// continuing over Wi-Fi, told apart by the first frame.
async fn accept(
    mut transport: Transport,
    id: String,
    sender: Sender<ChannelMessage>,
    upgrades: UpgradeRegistry,
) {
    let first_frame = match transport.read_frame().await {
        Ok(frame) => frame,
        Err(e) => {
            debug!("{INNER_NAME}: {id} closed before its first frame: {e}");
            return;
        }
    };

    if let Some(endpoint_id) = client_introduction(&first_frame) {
        if upgrades.deliver(&endpoint_id, transport) {
            info!("{INNER_NAME}: {id} continues the BLE session of {endpoint_id}");
        } else {
            warn!(
                "{INNER_NAME}: {id} introduced itself as {endpoint_id}, but no session awaits it"
            );
        }
        return;
    }

    InboundRequest::new(transport, id, sender)
        .run(Some(first_frame))
        .await;
}

async fn connect(
    endpoint_id: [u8; 4],
    sender: Sender<ChannelMessage>,
    ctk: CancellationToken,
    si: SendInfo,
) -> Result<(), anyhow::Error> {
    debug!("{INNER_NAME}: Connecting to: {}", si.addr);
    let socket = match TcpStream::connect(&si.addr).await {
        Ok(socket) => socket,
        Err(e) => {
            let _ = sender.send(ChannelMessage {
                id: si.id,
                direction: ChannelDirection::LibToFront,
                rtype: Some(TransferType::Outbound),
                state: Some(State::Disconnected),
                ..Default::default()
            });
            return Err(e.into());
        }
    };

    let mut or = OutboundRequest::new(
        endpoint_id,
        Transport::new(socket),
        si.id,
        sender.clone(),
        si.ob,
        RemoteDeviceInfo {
            device_type: crate::DeviceType::Unknown,
            name: si.name,
        },
    );

    // Send connection request
    or.send_connection_request().await?;
    // Send UKEY init
    or.send_ukey2_client_init().await?;

    loop {
        tokio::select! {
            _ = ctk.cancelled() => {
                info!("{INNER_NAME}: tracker cancelled, breaking");
                break;
            },
            r = or.handle() => {
                if let Err(e) = r {
                    match e.downcast_ref() {
                        Some(AppError::NotAnError) => break,
                        None => {
                            if or.state.state == State::Initial {
                                break;
                            }

                            if or.state.state != State::Finished && or.state.state != State::Cancelled {
                                let _ = sender.clone().send(ChannelMessage {
                                    id: si.addr,
                                    direction: ChannelDirection::LibToFront,
                                    state: Some(State::Disconnected),
                                    ..Default::default()
                                });
                            }
                            error!("{INNER_NAME}: error while handling client: {e} ({:?})", or.state.state);
                            break;
                        }
                    }
                }
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use prost::Message;
    use tokio::sync::broadcast;

    use super::*;
    use crate::location_nearby_connections::bandwidth_upgrade_negotiation_frame::{
        ClientIntroduction, EventType,
    };
    use crate::location_nearby_connections::{
        BandwidthUpgradeNegotiationFrame, ConnectionRequestFrame, OfflineFrame, V1Frame,
        offline_frame, v1_frame,
    };
    use crate::securegcm::{Ukey2Message, ukey2_message};
    use crate::utils::{DeviceType, encode_endpoint_info};

    fn v1(frame: V1Frame) -> Vec<u8> {
        OfflineFrame {
            version: Some(offline_frame::Version::V1.into()),
            v1: Some(frame),
        }
        .encode_to_vec()
    }

    /// Spawns `accept` on one end of an in-memory connection, returning the other.
    fn accept_peer(upgrades: UpgradeRegistry) -> Transport {
        let (ours, peer) = tokio::io::duplex(4096);
        let (sender, _) = broadcast::channel(8);
        tokio::spawn(accept(
            Transport::new(ours),
            "peer".into(),
            sender,
            upgrades,
        ));
        Transport::new(peer)
    }

    #[tokio::test]
    async fn serves_new_transfers() {
        let mut peer = accept_peer(UpgradeRegistry::default());

        peer.write_frame(&v1(V1Frame {
            r#type: Some(v1_frame::FrameType::ConnectionRequest.into()),
            connection_request: Some(ConnectionRequestFrame {
                endpoint_info: Some(encode_endpoint_info(DeviceType::Phone, "Pixel")),
                ..Default::default()
            }),
            ..Default::default()
        }))
        .await
        .unwrap();
        // Out of order, to get an answer: the first frame was handled.
        peer.write_frame(
            &Ukey2Message {
                message_type: Some(ukey2_message::Type::ClientFinish.into()),
                message_data: None,
            }
            .encode_to_vec(),
        )
        .await
        .unwrap();

        let reply = Ukey2Message::decode(&*peer.read_frame().await.unwrap()).unwrap();
        assert_eq!(reply.message_type(), ukey2_message::Type::Alert);
    }

    #[tokio::test]
    async fn hands_upgraded_connections_to_their_session() {
        let upgrades = UpgradeRegistry::default();
        let mut pending = upgrades.expect("PHNE".into());
        let mut peer = accept_peer(upgrades);

        peer.write_frame(&v1(V1Frame {
            r#type: Some(v1_frame::FrameType::BandwidthUpgradeNegotiation.into()),
            bandwidth_upgrade_negotiation: Some(BandwidthUpgradeNegotiationFrame {
                event_type: Some(EventType::ClientIntroduction.into()),
                client_introduction: Some(ClientIntroduction {
                    endpoint_id: Some("PHNE".into()),
                    supports_disabling_encryption: Some(false),
                }),
                ..Default::default()
            }),
            ..Default::default()
        }))
        .await
        .unwrap();
        peer.write_frame(b"next").await.unwrap();

        // Whatever followed the introduction is still there.
        let mut upgraded = pending.connection().await.unwrap();
        assert_eq!(upgraded.read_frame().await.unwrap(), b"next");
    }
}
