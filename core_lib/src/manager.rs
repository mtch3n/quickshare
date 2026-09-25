use serde::{Deserialize, Serialize};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::broadcast::Sender;
use tokio::sync::mpsc::Receiver;
use tokio_util::sync::CancellationToken;
use ts_rs::TS;

use crate::channel::{ChannelDirection, ChannelMessage, TransferType};
use crate::errors::AppError;
use crate::hdl::{InboundRequest, OutboundPayload, OutboundRequest, State, Transport};
use crate::utils::RemoteDeviceInfo;

const INNER_NAME: &str = "TcpServer";

#[derive(Debug, Deserialize, Serialize, TS)]
#[ts(export)]
pub struct SendInfo {
    pub id: String,
    pub name: String,
    pub addr: String,
    pub ob: OutboundPayload,
}

pub struct TcpServer {
    endpoint_id: [u8; 4],
    tcp_listener: TcpListener,
    sender: Sender<ChannelMessage>,
    connect_receiver: Receiver<SendInfo>,
}

impl TcpServer {
    pub fn new(
        endpoint_id: [u8; 4],
        tcp_listener: TcpListener,
        sender: Sender<ChannelMessage>,
        connect_receiver: Receiver<SendInfo>,
    ) -> Result<Self, anyhow::Error> {
        Ok(Self {
            endpoint_id,
            tcp_listener,
            sender,
            connect_receiver,
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
                            let ir = InboundRequest::new(Transport::new(socket), remote_addr.to_string(), self.sender.clone());
                            tokio::spawn(ir.run());
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
