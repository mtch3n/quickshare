use std::os::unix::fs::FileExt;
use std::time::Duration;

use anyhow::anyhow;
use prost::Message;
use sha2::{Digest, Sha512};
use tokio::sync::broadcast::{Receiver, Sender};

use super::bwu::{self, PendingUpgrade, WifiLanUpgrade};
use super::crypto::{self, Role};
use super::transport::Transport;
use super::{InnerState, State};
use crate::channel::{ChannelAction, ChannelDirection, ChannelMessage};
use crate::errors::AppError;
use crate::hdl::info::{InternalFileInfo, TransferMetadata, WifiNetwork, WifiSecurity};
use crate::hdl::{TextPayloadInfo, TextPayloadType};
use crate::location_nearby_connections::bandwidth_upgrade_negotiation_frame::EventType as UpgradeEvent;
use crate::location_nearby_connections::payload_transfer_frame::control_message::EventType as ControlEvent;
use crate::location_nearby_connections::payload_transfer_frame::{
    PacketType, PayloadChunk, PayloadHeader, payload_header,
};
use crate::location_nearby_connections::{KeepAliveFrame, OfflineFrame, PayloadTransferFrame};
use crate::securegcm::ukey2_alert::AlertType;
use crate::securegcm::{
    DeviceToDeviceMessage, GcmMetadata, Type, Ukey2Alert, Ukey2ClientFinished, Ukey2ClientInit,
    Ukey2HandshakeCipher, Ukey2Message, Ukey2ServerInit, ukey2_message,
};
use crate::securemessage::{
    EcP256PublicKey, EncScheme, GenericPublicKey, Header, HeaderAndBody, PublicKeyType,
    SecureMessage, SigScheme,
};
use crate::sharing_nearby::{
    WifiCredentials, paired_key_result_frame, text_metadata, wifi_credentials_metadata,
};
use crate::utils::{
    RemoteDeviceInfo, create_unique_file, gen_random, get_download_dir, lan_ipv4,
    parse_endpoint_info, sanitize_file_name,
};
use crate::{location_nearby_connections, sharing_nearby};

const SANE_FRAME_LENGTH: i32 = 5 * 1024 * 1024;
const SANITY_DURATION: Duration = Duration::from_micros(10);

/// How long the phone gets to come over Wi-Fi before we stay on BLE.
const UPGRADE_TIMEOUT: Duration = Duration::from_secs(15);
/// Longest pause in the prior channel while draining it.
const DRAIN_TIMEOUT: Duration = Duration::from_secs(5);
/// How long to wait for the phone's DISCONNECTION on the prior channel.
const PRIOR_DISCONNECTION_TIMEOUT: Duration = Duration::from_secs(1);

#[derive(Debug)]
pub struct InboundRequest {
    transport: Transport,
    pub state: InnerState,
    sender: Sender<ChannelMessage>,
    receiver: Receiver<ChannelMessage>,
    /// Set for BLE sessions, which move to Wi-Fi once encrypted.
    upgrade: Option<WifiLanUpgrade>,
    /// The phone's Nearby endpoint id, from its connection request.
    peer_endpoint_id: Option<String>,
}

impl InboundRequest {
    pub fn new(transport: Transport, id: String, sender: Sender<ChannelMessage>) -> Self {
        let receiver = sender.subscribe();

        Self {
            transport,
            state: InnerState {
                id,
                server_seq: 0,
                client_seq: 0,
                state: State::Initial,
                encryption_done: false,
                ..Default::default()
            },
            sender,
            receiver,
            upgrade: None,
            peer_endpoint_id: None,
        }
    }

    /// Offers the phone to continue over Wi-Fi LAN once the connection is encrypted.
    pub fn with_wifi_lan_upgrade(mut self, upgrade: WifiLanUpgrade) -> Self {
        self.upgrade = Some(upgrade);
        self
    }

    /// Drives the connection until it ends, reporting an unexpected
    /// disconnection to the frontend. `first_frame` was already read off the
    /// transport.
    pub async fn run(mut self, mut first_frame: Option<Vec<u8>>) {
        loop {
            let result = match first_frame.take() {
                Some(frame) => self.handle_frame(frame).await,
                None => self.handle().await,
            };
            let Err(e) = result else {
                continue;
            };

            if matches!(e.downcast_ref(), Some(AppError::NotAnError))
                || self.state.state == State::Initial
            {
                break;
            }

            if self.state.state != State::Finished {
                let _ = self.sender.send(ChannelMessage {
                    id: self.state.id.clone(),
                    direction: ChannelDirection::LibToFront,
                    state: Some(State::Disconnected),
                    ..Default::default()
                });
            }
            error!(
                "inbound: error while handling {}: {e} ({:?})",
                self.state.id, self.state.state
            );
            break;
        }
    }

    pub async fn handle(&mut self) -> Result<(), anyhow::Error> {
        tokio::select! {
            i = self.receiver.recv() => {
                match i {
                    Ok(channel_msg) => {
                        if channel_msg.direction == ChannelDirection::LibToFront {
                            return Ok(());
                        }

                        if channel_msg.id != self.state.id {
                            return Ok(());
                        }

                        debug!("inbound: got: {:?}", channel_msg);
                        match channel_msg.action {
                            Some(ChannelAction::AcceptTransfer) => {
                                self.accept_transfer().await?;
                            },
                            Some(ChannelAction::RejectTransfer) => {
                                self.update_state(
                                    |e| {
                                        e.state = State::Rejected;
                                    },
                                    true,
                                ).await;

                                self.reject_transfer(Some(
                                    sharing_nearby::connection_response_frame::Status::Reject
                                )).await?;
                                return Err(anyhow!(crate::errors::AppError::NotAnError));
                            },
                            Some(ChannelAction::CancelTransfer) => {
                                return self.cancel(true).await;
                            },
                            None => {
                                trace!("inbound: nothing to do")
                            },
                        }
                    }
                    Err(e) => {
                        error!("inbound: channel error: {}", e);
                    }
                }
            },
            frame = self.transport.read_frame() => {
                self.handle_frame(frame?).await?
            }
        }

        Ok(())
    }

    async fn handle_frame(&mut self, frame_data: Vec<u8>) -> Result<(), anyhow::Error> {
        let current_state = &self.state;
        // Now determine what will be the request type based on current state
        match current_state.state {
            State::Initial => {
                debug!("Handling State::Initial frame");
                let frame = location_nearby_connections::OfflineFrame::decode(&*frame_data)?;
                let rdi = self.process_connection_request(&frame)?;
                info!("RemoteDeviceInfo: {:?}", rdi);

                // Advance current state
                self.update_state(
                    |e: &mut InnerState| {
                        e.state = State::ReceivedConnectionRequest;
                        e.remote_device_info = Some(rdi);
                    },
                    false,
                )
                .await;
            }
            State::ReceivedConnectionRequest => {
                debug!("Handling State::ReceivedConnectionRequest frame");
                let msg = Ukey2Message::decode(&*frame_data)?;
                self.process_ukey2_client_init(&msg).await?;

                self.update_state(
                    |e: &mut InnerState| {
                        e.state = State::SentUkeyServerInit;
                        e.client_init_msg_data = Some(frame_data);
                    },
                    false,
                )
                .await;
            }
            State::SentUkeyServerInit => {
                debug!("Handling State::SentUkeyServerInit frame");
                let msg = Ukey2Message::decode(&*frame_data)?;
                self.process_ukey2_client_finish(&msg, &frame_data).await?;

                self.update_state(
                    |e: &mut InnerState| {
                        e.state = State::ReceivedUkeyClientFinish;
                    },
                    false,
                )
                .await;
            }
            State::ReceivedUkeyClientFinish => {
                debug!("Handling State::ReceivedUkeyClientFinish frame");
                let frame = location_nearby_connections::OfflineFrame::decode(&*frame_data)?;
                self.process_connection_response(&frame).await?;

                self.update_state(
                    |e: &mut InnerState| {
                        e.state = State::SentConnectionResponse;
                    },
                    false,
                )
                .await;

                // Encrypted now, so a BLE session can move to Wi-Fi.
                if let Some(upgrade) = self.upgrade.take() {
                    self.upgrade_to_wifi_lan(upgrade).await?;
                }
            }
            _ => {
                debug!("Handling SecureMessage frame");
                let offline = self.decrypt_frame(&frame_data).await?;
                self.process_offline_frame(offline).await?;
            }
        }

        Ok(())
    }

    fn process_connection_request(
        &mut self,
        frame: &location_nearby_connections::OfflineFrame,
    ) -> Result<RemoteDeviceInfo, anyhow::Error> {
        let v1_frame = frame
            .v1
            .as_ref()
            .ok_or_else(|| anyhow!("Missing required fields"))?;

        if v1_frame.r#type() != location_nearby_connections::v1_frame::FrameType::ConnectionRequest
        {
            return Err(anyhow!(format!(
                "Unexpected frame type: {:?}",
                v1_frame.r#type()
            )));
        }

        let connection_request = v1_frame
            .connection_request
            .as_ref()
            .ok_or_else(|| anyhow!("Missing required fields"))?;
        self.peer_endpoint_id = connection_request.endpoint_id.clone();

        let endpoint_info = connection_request
            .endpoint_info
            .as_ref()
            .ok_or_else(|| anyhow!("Missing endpoint info"))?;

        let (device_type, name) = parse_endpoint_info(endpoint_info)?;
        let name = name
            .or_else(|| {
                connection_request
                    .endpoint_name
                    .as_ref()
                    .map(|n| String::from_utf8_lossy(n).into_owned())
            })
            .unwrap_or_else(|| "Unknown".to_string());

        Ok(RemoteDeviceInfo { name, device_type })
    }

    async fn process_ukey2_client_init(&mut self, msg: &Ukey2Message) -> Result<(), anyhow::Error> {
        if msg.message_type() != ukey2_message::Type::ClientInit {
            self.send_ukey2_alert(AlertType::BadMessageType).await?;
            return Err(anyhow!(
                "UKey2: message_type({:?}) != ClientInit",
                msg.message_type
            ));
        }

        let client_init = match Ukey2ClientInit::decode(msg.message_data()) {
            Ok(uk2ci) => uk2ci,
            Err(e) => {
                self.send_ukey2_alert(AlertType::BadMessageData).await?;
                return Err(anyhow!("UKey2: Ukey2ClientInit::decode: {}", e));
            }
        };

        if client_init.version() != 1 {
            self.send_ukey2_alert(AlertType::BadVersion).await?;
            return Err(anyhow!("UKey2: client_init.version != 1"));
        }

        if client_init.random().len() != 32 {
            self.send_ukey2_alert(AlertType::BadRandom).await?;
            return Err(anyhow!("UKey2: client_init.random.len != 32"));
        }

        // Searching for preferred cipher commitment
        let mut found = false;
        for commitment in &client_init.cipher_commitments {
            trace!("CipherCommitment: {:?}", commitment.handshake_cipher());
            if Ukey2HandshakeCipher::P256Sha512 == commitment.handshake_cipher() {
                found = true;
                self.update_state(
                    |e| {
                        e.cipher_commitment = Some(commitment.clone());
                    },
                    false,
                )
                .await;
                break;
            }
        }

        if !found {
            self.send_ukey2_alert(AlertType::BadHandshakeCipher).await?;
            return Err(anyhow!("UKey2: badHandshakeCipher"));
        }

        if client_init.next_protocol() != "AES_256_CBC-HMAC_SHA256" {
            self.send_ukey2_alert(AlertType::BadNextProtocol).await?;
            return Err(anyhow!(
                "UKey2: badNextProtocol: {}",
                client_init.next_protocol()
            ));
        }

        let (secret_key, public_key) = crypto::gen_keypair();
        let (x, y) = crypto::encode_public_key(&public_key);

        let pkey = GenericPublicKey {
            r#type: PublicKeyType::EcP256.into(),
            ec_p256_public_key: Some(EcP256PublicKey { x, y }),
            ..Default::default()
        };

        let server_init = Ukey2ServerInit {
            version: Some(1),
            random: Some(rand::random::<[u8; 32]>().to_vec()),
            handshake_cipher: Some(Ukey2HandshakeCipher::P256Sha512.into()),
            public_key: Some(pkey.encode_to_vec()),
        };

        let server_init_msg = Ukey2Message {
            message_type: Some(ukey2_message::Type::ServerInit.into()),
            message_data: Some(server_init.encode_to_vec()),
        };

        let server_init_data = server_init_msg.encode_to_vec();
        self.update_state(
            |e| {
                e.private_key = Some(secret_key);
                e.server_init_data = Some(server_init_data.clone());
            },
            false,
        )
        .await;

        self.send_frame(server_init_data).await?;

        Ok(())
    }

    async fn process_ukey2_client_finish(
        &mut self,
        msg: &Ukey2Message,
        frame_data: &Vec<u8>,
    ) -> Result<(), anyhow::Error> {
        if msg.message_type() != ukey2_message::Type::ClientFinish {
            self.send_ukey2_alert(AlertType::BadMessageType).await?;
            return Err(anyhow!(
                "UKey2: message_type({:?}) != ClientFinish",
                msg.message_type
            ));
        }

        let sha512 = Sha512::digest(frame_data);
        if self.state.cipher_commitment.as_ref().unwrap().commitment() != sha512.as_slice() {
            error!("cipher_commitment isn't equals to sha512(frame_data)");
            return Err(anyhow!("UKey2: cipher_commitment != sha512"));
        }

        let client_finish = match Ukey2ClientFinished::decode(msg.message_data()) {
            Ok(uk2cf) => uk2cf,
            Err(e) => {
                return Err(anyhow!("UKey2: Ukey2ClientFinished::decode: {}", e));
            }
        };

        if client_finish.public_key.is_none() {
            return Err(anyhow!("UKey2: client_finish.public_key None"));
        }

        let client_public_key = match GenericPublicKey::decode(client_finish.public_key()) {
            Ok(cpk) => cpk,
            Err(e) => {
                return Err(anyhow!("UKey2: GenericPublicKey::decode: {}", e));
            }
        };

        self.finalize_key_exchange(client_public_key).await?;

        Ok(())
    }

    async fn process_connection_response(
        &mut self,
        frame: &location_nearby_connections::OfflineFrame,
    ) -> Result<(), anyhow::Error> {
        let v1_frame = frame
            .v1
            .as_ref()
            .ok_or_else(|| anyhow!("Missing required fields"))?;

        if v1_frame.r#type() != location_nearby_connections::v1_frame::FrameType::ConnectionResponse
        {
            return Err(anyhow!(format!(
                "Unexpected frame type: {:?}",
                v1_frame.r#type()
            )));
        }

        let response = location_nearby_connections::OfflineFrame {
			version: Some(location_nearby_connections::offline_frame::Version::V1.into()),
			v1: Some(location_nearby_connections::V1Frame {
				r#type: Some(location_nearby_connections::v1_frame::FrameType::ConnectionResponse.into()),
				connection_response: Some(location_nearby_connections::ConnectionResponseFrame {
					response: Some(location_nearby_connections::connection_response_frame::ResponseStatus::Accept.into()),
					os_info: Some(location_nearby_connections::OsInfo {
						r#type: Some(location_nearby_connections::os_info::OsType::Linux.into())
					}),
					..Default::default()
				}),
				..Default::default()
			})
		};

        self.send_frame(response.encode_to_vec()).await?;

        let paired_encryption = sharing_nearby::Frame {
            version: Some(sharing_nearby::frame::Version::V1.into()),
            v1: Some(sharing_nearby::V1Frame {
                r#type: Some(sharing_nearby::v1_frame::FrameType::PairedKeyEncryption.into()),
                paired_key_encryption: Some(sharing_nearby::PairedKeyEncryptionFrame {
                    secret_id_hash: Some(gen_random(6)),
                    signed_data: Some(gen_random(72)),
                    ..Default::default()
                }),
                ..Default::default()
            }),
        };

        self.send_encrypted_frame(&paired_encryption).await?;

        Ok(())
    }

    /// Verifies and decrypts a SecureMessage frame, checking its sequence number.
    async fn decrypt_frame(&mut self, frame_data: &[u8]) -> Result<OfflineFrame, anyhow::Error> {
        let smsg = SecureMessage::decode(frame_data)?;
        crypto::verify(
            self.state.recv_hmac_key.as_ref().unwrap(),
            &smsg.header_and_body,
            &smsg.signature,
        )?;

        let header_and_body = HeaderAndBody::decode(&*smsg.header_and_body)?;
        let decrypted = crypto::decrypt(
            self.state.decrypt_key.as_ref().unwrap(),
            header_and_body.header.iv(),
            &header_and_body.body,
        )?;

        let d2d_msg = DeviceToDeviceMessage::decode(&*decrypted)?;

        let seq = self.get_client_seq_inc().await;
        if d2d_msg.sequence_number() != seq {
            return Err(anyhow!(
                "Error d2d_msg.sequence_number invalid ({} vs {})",
                d2d_msg.sequence_number(),
                seq
            ));
        }

        Ok(OfflineFrame::decode(d2d_msg.message())?)
    }

    async fn process_offline_frame(&mut self, offline: OfflineFrame) -> Result<(), anyhow::Error> {
        let v1_frame = offline
            .v1
            .as_ref()
            .ok_or_else(|| anyhow!("Missing required fields"))?;
        match v1_frame.r#type() {
            location_nearby_connections::v1_frame::FrameType::PayloadTransfer => {
                trace!("Received FrameType::PayloadTransfer");
                let payload_transfer = v1_frame
                    .payload_transfer
                    .as_ref()
                    .ok_or_else(|| anyhow!("Missing required fields"))?;

                let header = payload_transfer
                    .payload_header
                    .as_ref()
                    .ok_or_else(|| anyhow!("Missing required fields"))?;

                if payload_transfer.packet_type() == PacketType::Control {
                    let event = payload_transfer.control_message.as_ref().map(|c| c.event());
                    if event == Some(ControlEvent::PayloadCanceled) {
                        info!("Sender cancelled payload {}", header.id());
                        return self.cancel(false).await;
                    }
                    return Ok(());
                }

                let chunk = payload_transfer
                    .payload_chunk
                    .as_ref()
                    .ok_or_else(|| anyhow!("Missing required fields"))?;

                match header.r#type() {
                    payload_header::PayloadType::Bytes => {
                        info!("Processing PayloadType::Bytes");
                        let payload_id = header.id();

                        if header.total_size() > SANE_FRAME_LENGTH.into() {
                            self.state.payload_buffers.remove(&payload_id);
                            return Err(anyhow!(
                                "Payload too large: {} bytes",
                                header.total_size()
                            ));
                        }

                        self.state
                            .payload_buffers
                            .entry(payload_id)
                            .or_insert_with(|| Vec::with_capacity(header.total_size() as usize));

                        // Get the current length of the buffer, if it exists, without holding a mutable borrow.
                        let buffer_len = self.state.payload_buffers.get(&payload_id).unwrap().len();
                        if chunk.offset() != buffer_len as i64 {
                            self.state.payload_buffers.remove(&payload_id);
                            return Err(anyhow!(
                                "Unexpected chunk offset: {}, expected: {}",
                                chunk.offset(),
                                buffer_len
                            ));
                        }

                        let buffer = self.state.payload_buffers.get_mut(&payload_id).unwrap();
                        if let Some(body) = &chunk.body {
                            buffer.extend(body);
                        }

                        if (chunk.flags() & 1) == 1 {
                            debug!("Chunk flags & 1 == 1 ?? End of data ??");

                            if self.state.text_payload.is_some()
                                && self.state.text_payload.as_ref().unwrap().get_i64_value()
                                    == payload_id
                            {
                                info!("Transfer finished");
                                let payload = String::from_utf8_lossy(buffer).into_owned();

                                match self.state.text_payload.clone().unwrap() {
                                    TextPayloadInfo::Url(_) => {
                                        self.update_state(
                                            |e| {
                                                if let Some(tmd) = e.transfer_metadata.as_mut() {
                                                    tmd.text_payload = Some(payload);
                                                    tmd.text_type = Some(TextPayloadType::Url);
                                                }
                                            },
                                            false,
                                        )
                                        .await;
                                    }
                                    TextPayloadInfo::Text(_) => {
                                        self.update_state(
                                            |e| {
                                                if let Some(tmd) = e.transfer_metadata.as_mut() {
                                                    tmd.text_payload = Some(payload);
                                                    tmd.text_type = Some(TextPayloadType::Text);
                                                }
                                            },
                                            false,
                                        )
                                        .await;
                                    }
                                    TextPayloadInfo::Wifi((_, ssid, security_type)) => {
                                        let wifi_network = WifiCredentials::decode(buffer.as_slice())
                                            .ok()
                                            .and_then(|creds| {
                                                let security = match security_type {
                                                    wifi_credentials_metadata::SecurityType::Open => {
                                                        WifiSecurity::Open
                                                    }
                                                    wifi_credentials_metadata::SecurityType::WpaPsk => {
                                                        WifiSecurity::WpaPsk
                                                    }
                                                    wifi_credentials_metadata::SecurityType::Wep => {
                                                        WifiSecurity::Wep
                                                    }
                                                    wifi_credentials_metadata::SecurityType::Sae => {
                                                        WifiSecurity::Sae
                                                    }
                                                    wifi_credentials_metadata::SecurityType::UnknownSecurityType => {
                                                        return None;
                                                    }
                                                };

                                                Some(WifiNetwork {
                                                    ssid: ssid.clone(),
                                                    password: creds.password.unwrap_or_default(),
                                                    security,
                                                    hidden: creds.hidden_ssid.unwrap_or(false),
                                                })
                                            });

                                        self.update_state(
                                            |e| {
                                                if let Some(tmd) = e.transfer_metadata.as_mut() {
                                                    tmd.text_payload = Some(ssid.clone());
                                                    tmd.text_type = Some(TextPayloadType::Wifi);
                                                    tmd.wifi = wifi_network;
                                                }
                                            },
                                            false,
                                        )
                                        .await;
                                    }
                                }

                                self.update_state(
                                    |e| {
                                        e.state = State::Finished;
                                    },
                                    true,
                                )
                                .await;
                                self.disconnection().await?;
                                return Err(anyhow!(crate::errors::AppError::NotAnError));
                            } else {
                                let innner_frame =
                                    sharing_nearby::Frame::decode(buffer.as_slice())?;
                                self.process_transfer_setup(&innner_frame).await?;
                            }
                        }
                    }
                    payload_header::PayloadType::File => {
                        info!("Processing PayloadType::File");
                        let payload_id = header.id();

                        let file_internal = self
                            .state
                            .transferred_files
                            .get_mut(&payload_id)
                            .ok_or_else(|| {
                                anyhow!("File payload ID ({}) is not known", payload_id)
                            })?;

                        let current_offset = file_internal.bytes_transferred;
                        if chunk.offset() != current_offset {
                            return Err(anyhow!(
                                "Invalid offset into file {}, expected {}",
                                chunk.offset(),
                                current_offset
                            ));
                        }

                        let chunk_size = chunk.body().len();
                        if current_offset + chunk_size as i64 > file_internal.total_size {
                            return Err(anyhow!(
                                "Transferred file size exceeds previously specified value: {} vs {}",
                                current_offset + chunk_size as i64,
                                file_internal.total_size
                            ));
                        }

                        if !chunk.body().is_empty() {
                            file_internal
                                .file
                                .as_ref()
                                .unwrap()
                                .write_all_at(chunk.body(), current_offset as u64)?;
                            file_internal.bytes_transferred += chunk_size as i64;

                            self.update_state(
                                |e| {
                                    if let Some(tmd) = e.transfer_metadata.as_mut() {
                                        tmd.ack_bytes += chunk_size as u64;
                                    }
                                },
                                true,
                            )
                            .await;
                        } else if (chunk.flags() & 1) == 1 {
                            self.state.transferred_files.remove(&payload_id);
                            if self.state.transferred_files.is_empty() {
                                info!("Transfer finished");
                                self.update_state(
                                    |e| {
                                        e.state = State::Finished;
                                    },
                                    true,
                                )
                                .await;
                                self.disconnection().await?;
                                return Err(anyhow!(crate::errors::AppError::NotAnError));
                            }
                        }
                    }
                    payload_header::PayloadType::Stream => {
                        error!("Unhandled PayloadType::Stream: {:?}", header.r#type())
                    }
                    payload_header::PayloadType::UnknownPayloadType => {
                        error!(
                            "Invalid PayloadType::UnknownPayloadType: {:?}",
                            header.r#type()
                        )
                    }
                }
            }
            location_nearby_connections::v1_frame::FrameType::KeepAlive => {
                trace!("Sending keepalive");
                self.send_keepalive(true).await?;
            }
            location_nearby_connections::v1_frame::FrameType::BandwidthUpgradeNegotiation => {
                info!(
                    "Ignoring bandwidth upgrade frame: {:?}",
                    bwu::event(&offline)
                );
            }
            _ => {
                error!("Unhandled offline frame encrypted: {:?}", offline);
            }
        }

        Ok(())
    }

    async fn process_transfer_setup(
        &mut self,
        frame: &sharing_nearby::Frame,
    ) -> Result<(), anyhow::Error> {
        let v1_frame = frame
            .v1
            .as_ref()
            .ok_or_else(|| anyhow!("Missing required fields"))?;

        if v1_frame.r#type() == sharing_nearby::v1_frame::FrameType::Cancel {
            info!("Transfer canceled by the sender");
            return self.cancel(false).await;
        }

        match self.state.state {
            State::SentConnectionResponse => {
                debug!("Processing State::SentConnectionResponse");
                self.process_paired_key_encryption_frame(v1_frame).await?;
                self.update_state(
                    |e| {
                        e.state = State::SentPairedKeyResult;
                    },
                    false,
                )
                .await;
            }
            State::SentPairedKeyResult => {
                debug!("Processing State::SentPairedKeyResult");
                self.process_paired_key_result(v1_frame).await?;
                self.update_state(
                    |e| {
                        e.state = State::ReceivedPairedKeyResult;
                    },
                    false,
                )
                .await;
            }
            State::ReceivedPairedKeyResult => {
                debug!("Processing State::ReceivedPairedKeyResult");
                // Newer Pixels send other frames before the introduction.
                if v1_frame.introduction.is_some() {
                    self.process_introduction(v1_frame).await?;
                } else {
                    debug!(
                        "Awaiting introduction, ignoring {:?} frame",
                        v1_frame.r#type()
                    );
                }
            }
            _ => {
                info!(
                    "Unhandled connection state in process_transfer_setup: {:?}",
                    self.state.state
                );
            }
        }

        Ok(())
    }

    async fn process_paired_key_encryption_frame(
        &mut self,
        v1_frame: &sharing_nearby::V1Frame,
    ) -> Result<(), anyhow::Error> {
        if v1_frame.paired_key_encryption.is_none() {
            return Err(anyhow!("Missing required fields"));
        }

        let paired_result = sharing_nearby::Frame {
            version: Some(sharing_nearby::frame::Version::V1.into()),
            v1: Some(sharing_nearby::V1Frame {
                r#type: Some(sharing_nearby::v1_frame::FrameType::PairedKeyResult.into()),
                paired_key_result: Some(sharing_nearby::PairedKeyResultFrame {
                    status: Some(paired_key_result_frame::Status::Unable.into()),
                }),
                ..Default::default()
            }),
        };

        self.send_encrypted_frame(&paired_result).await?;

        Ok(())
    }

    async fn process_paired_key_result(
        &self,
        v1_frame: &sharing_nearby::V1Frame,
    ) -> Result<(), anyhow::Error> {
        if v1_frame.paired_key_result.is_none() {
            return Err(anyhow!("Missing required fields"));
        }

        Ok(())
    }

    async fn process_introduction(
        &mut self,
        v1_frame: &sharing_nearby::V1Frame,
    ) -> Result<(), anyhow::Error> {
        let introduction = v1_frame
            .introduction
            .as_ref()
            .ok_or_else(|| anyhow!("Missing required fields"))?;

        // No need to inform the channel here, we'll do it anyway with files info
        self.update_state(
            |e| {
                e.state = State::WaitingForUserConsent;
            },
            false,
        )
        .await;

        if !introduction.file_metadata.is_empty() && introduction.text_metadata.is_empty() {
            trace!("process_introduction: handling file_metadata");
            let mut files_name = Vec::with_capacity(introduction.file_metadata.len());
            let mut total_bytes: u64 = 0;

            for file in &introduction.file_metadata {
                let name = sanitize_file_name(file.name())
                    .unwrap_or_else(|| format!("file_{}", file.payload_id()));

                let mut dest = get_download_dir();

                // Handle parent_folder if present
                let parent_folder = file.parent_folder();
                if !parent_folder.is_empty() {
                    // Split by '/' and sanitize each component
                    for component in parent_folder.split('/') {
                        if !component.is_empty()
                            && component != "."
                            && component != ".."
                            && let Some(sanitized) = sanitize_file_name(component)
                        {
                            dest.push(sanitized);
                        }
                    }
                }

                dest.push(&name);

                let info = InternalFileInfo {
                    payload_id: file.payload_id(),
                    file_url: dest,
                    bytes_transferred: 0,
                    total_size: file.size(),
                    file: None,
                };
                total_bytes += info.total_size as u64;
                self.state.transferred_files.insert(file.payload_id(), info);
                files_name.push(name);
            }

            let metadata = TransferMetadata {
                destination: Some(
                    get_download_dir()
                        .into_os_string()
                        .into_string()
                        .map_err(|_| anyhow!("failed to convert PathBuf to String"))?,
                ),
                source: self.state.remote_device_info.clone(),
                files: Some(files_name),
                pin_code: self.state.pin_code.clone(),
                text_description: None,
                total_bytes,
                ..Default::default()
            };

            info!("Asking for user consent: {:?}", metadata);
            self.update_state(
                |e| {
                    e.transfer_metadata = Some(metadata);
                },
                true,
            )
            .await;
        } else if introduction.text_metadata.len() == 1 {
            trace!("process_introduction: handling text_metadata");
            let meta = introduction.text_metadata.first().unwrap();

            match meta.r#type() {
                text_metadata::Type::Url => {
                    let metadata = TransferMetadata {
                        destination: None,
                        source: self.state.remote_device_info.clone(),
                        files: None,
                        pin_code: self.state.pin_code.clone(),
                        text_description: meta.text_title.clone(),
                        ..Default::default()
                    };

                    info!("Asking for user consent: {:?}", metadata);
                    self.update_state(
                        |e| {
                            e.text_payload = Some(TextPayloadInfo::Url(meta.payload_id()));
                            e.transfer_metadata = Some(metadata);
                        },
                        true,
                    )
                    .await;
                }
                text_metadata::Type::PhoneNumber
                | text_metadata::Type::Address
                | text_metadata::Type::Text => {
                    let metadata = TransferMetadata {
                        destination: None,
                        source: self.state.remote_device_info.clone(),
                        files: None,
                        pin_code: self.state.pin_code.clone(),
                        text_description: meta.text_title.clone(),
                        ..Default::default()
                    };

                    info!("Asking for user consent: {:?}", metadata);
                    self.update_state(
                        |e| {
                            e.text_payload = Some(TextPayloadInfo::Text(meta.payload_id()));
                            e.transfer_metadata = Some(metadata);
                        },
                        true,
                    )
                    .await;
                }
                text_metadata::Type::Unknown => {
                    // Reject transfer
                    self.reject_transfer(Some(
						sharing_nearby::connection_response_frame::Status::UnsupportedAttachmentType,
					))
					.await?;
                }
            }
        } else if introduction.wifi_credentials_metadata.len() == 1 {
            trace!("process_introduction: handling wifi_credentials_metadata");
            let meta = introduction.wifi_credentials_metadata.first().unwrap();

            let metadata = TransferMetadata {
                destination: None,
                source: self.state.remote_device_info.clone(),
                files: None,
                pin_code: self.state.pin_code.clone(),
                text_description: meta.ssid.clone(),
                ..Default::default()
            };

            self.update_state(
                |e| {
                    e.text_payload = Some(TextPayloadInfo::Wifi((
                        meta.payload_id(),
                        meta.ssid().to_owned(),
                        meta.security_type(),
                    )));
                    e.transfer_metadata = Some(metadata);
                },
                true,
            )
            .await;
        } else {
            // Reject transfer
            self.reject_transfer(Some(
                sharing_nearby::connection_response_frame::Status::UnsupportedAttachmentType,
            ))
            .await?;
        }

        Ok(())
    }

    /// Nearby's bandwidth upgrade to Wi-Fi LAN with us as the initiator, see
    /// `bwu.rs`. The session stays on BLE until the phone has come over; once
    /// we've acknowledged its connection there is no way back.
    async fn upgrade_to_wifi_lan(&mut self, upgrade: WifiLanUpgrade) -> Result<(), anyhow::Error> {
        let (Some(ip), Some(endpoint_id)) = (lan_ipv4(), self.peer_endpoint_id.clone()) else {
            warn!("BWU: no LAN address or phone endpoint id, staying on BLE");
            return Ok(());
        };

        let mut pending = upgrade.registry.expect(endpoint_id);
        info!("BWU: offering Wi-Fi LAN at {ip}:{}", upgrade.port);
        self.encrypt_and_send(&bwu::upgrade_path_available(ip, upgrade.port))
            .await?;

        let Some(mut upgraded) = self.wait_for_upgrade(&mut pending).await? else {
            return Ok(());
        };
        drop(pending);

        upgraded
            .write_frame(&bwu::client_introduction_ack().encode_to_vec())
            .await?;
        self.encrypt_and_send(&bwu::event_frame(UpgradeEvent::LastWriteToPriorChannel))
            .await?;
        self.drain_prior_channel().await?;

        // Plaintext, so it doesn't take a sequence number. The phone keeps the
        // new channel paused until it reads this, and sends its own.
        if let Err(e) = self
            .send_frame(bwu::prior_channel_disconnection().encode_to_vec())
            .await
        {
            debug!("BWU: couldn't close the prior channel: {e}");
        }
        match tokio::time::timeout(PRIOR_DISCONNECTION_TIMEOUT, self.transport.read_frame()).await {
            Ok(Ok(frame)) => debug!(
                "BWU: phone's last frame on the prior channel: {:?}",
                OfflineFrame::decode(&*frame)
                    .ok()
                    .and_then(|f| f.v1)
                    .map(|v| v.r#type())
            ),
            Ok(Err(e)) => debug!("BWU: prior channel closed: {e}"),
            Err(_) => debug!("BWU: no DISCONNECTION from the phone on the prior channel"),
        }

        let prior = std::mem::replace(&mut self.transport, upgraded);
        if prior.buffered() > 0 {
            warn!(
                "BWU: dropping {} unexpected bytes left on the prior channel",
                prior.buffered()
            );
        }
        info!("BWU: upgraded to Wi-Fi LAN, continuing over TCP");

        Ok(())
    }

    /// Keeps serving the BLE channel until the phone's Wi-Fi connection
    /// arrives. `None` when it won't: the phone gave up or took too long.
    async fn wait_for_upgrade(
        &mut self,
        pending: &mut PendingUpgrade,
    ) -> Result<Option<Transport>, anyhow::Error> {
        let deadline = tokio::time::sleep(UPGRADE_TIMEOUT);
        tokio::pin!(deadline);

        loop {
            tokio::select! {
                connection = pending.connection() => {
                    if connection.is_some() {
                        info!("BWU: phone connected over Wi-Fi");
                    }
                    return Ok(connection);
                }
                _ = &mut deadline => {
                    warn!(
                        "BWU: phone didn't connect over Wi-Fi within {}s, staying on BLE",
                        UPGRADE_TIMEOUT.as_secs()
                    );
                    return Ok(None);
                }
                frame = self.transport.read_frame() => {
                    let offline = self.decrypt_frame(&frame?).await?;
                    if bwu::event(&offline) == Some(UpgradeEvent::UpgradeFailure) {
                        warn!("BWU: phone couldn't connect over Wi-Fi, staying on BLE");
                        return Ok(None);
                    }
                    self.process_offline_frame(offline).await?;
                }
            }
        }
    }

    /// Reads the prior channel up to the phone's SAFE_TO_CLOSE. Frames it sent
    /// before its LAST_WRITE are processed as usual, and its LAST_WRITE is
    /// answered with our SAFE_TO_CLOSE.
    async fn drain_prior_channel(&mut self) -> Result<(), anyhow::Error> {
        loop {
            let frame = match tokio::time::timeout(DRAIN_TIMEOUT, self.transport.read_frame()).await
            {
                Ok(Ok(frame)) => frame,
                Ok(Err(e)) => {
                    warn!("BWU: prior channel closed before SAFE_TO_CLOSE: {e}");
                    return Ok(());
                }
                Err(_) => {
                    warn!("BWU: no SAFE_TO_CLOSE from the phone, switching anyway");
                    return Ok(());
                }
            };

            // We're committed to TCP by now, so don't let the old channel end the session.
            let offline = match self.decrypt_frame(&frame).await {
                Ok(offline) => offline,
                Err(e) => {
                    warn!("BWU: unreadable frame on the prior channel, switching: {e}");
                    return Ok(());
                }
            };
            match bwu::event(&offline) {
                Some(UpgradeEvent::LastWriteToPriorChannel) => {
                    debug!("BWU: phone's LAST_WRITE, answering SAFE_TO_CLOSE");
                    if let Err(e) = self
                        .encrypt_and_send(&bwu::event_frame(UpgradeEvent::SafeToClosePriorChannel))
                        .await
                    {
                        debug!("BWU: couldn't send SAFE_TO_CLOSE: {e}");
                    }
                }
                Some(UpgradeEvent::SafeToClosePriorChannel) => {
                    debug!("BWU: phone's SAFE_TO_CLOSE");
                    return Ok(());
                }
                Some(event) => debug!("BWU: ignoring {event:?} on the prior channel"),
                None => self.process_offline_frame(offline).await?,
            }
        }
    }

    async fn disconnection(&mut self) -> Result<(), anyhow::Error> {
        let frame = location_nearby_connections::OfflineFrame {
            version: Some(location_nearby_connections::offline_frame::Version::V1.into()),
            v1: Some(location_nearby_connections::V1Frame {
                r#type: Some(
                    location_nearby_connections::v1_frame::FrameType::Disconnection.into(),
                ),
                disconnection: Some(location_nearby_connections::DisconnectionFrame {
                    ..Default::default()
                }),
                ..Default::default()
            }),
        };

        if self.state.encryption_done {
            self.encrypt_and_send(&frame).await
        } else {
            self.send_frame(frame.encode_to_vec()).await
        }
    }

    /// Stops the transfer and deletes partially received files. `notify_peer`
    /// sends a Cancel frame first, for cancellations coming from our side.
    async fn cancel(&mut self, notify_peer: bool) -> Result<(), anyhow::Error> {
        if notify_peer && self.state.encryption_done {
            let frame = sharing_nearby::Frame {
                version: Some(sharing_nearby::frame::Version::V1.into()),
                v1: Some(sharing_nearby::V1Frame {
                    r#type: Some(sharing_nearby::v1_frame::FrameType::Cancel.into()),
                    ..Default::default()
                }),
            };
            let _ = self.send_encrypted_frame(&frame).await;
        }

        for (_, info) in self.state.transferred_files.drain() {
            if info.file.is_some() {
                let _ = std::fs::remove_file(&info.file_url);
            }
        }

        self.update_state(|e| e.state = State::Cancelled, true)
            .await;
        self.disconnection().await?;
        Err(anyhow!(crate::errors::AppError::NotAnError))
    }

    async fn accept_transfer(&mut self) -> Result<(), anyhow::Error> {
        let ids: Vec<i64> = self.state.transferred_files.keys().cloned().collect();

        for id in ids {
            let mfi = self.state.transferred_files.get_mut(&id).unwrap();

            let (path, file) = create_unique_file(&mfi.file_url)?;
            info!("Receiving into {}", path.display());
            mfi.file_url = path;
            mfi.file = Some(file);
        }

        let frame = sharing_nearby::Frame {
            version: Some(sharing_nearby::frame::Version::V1.into()),
            v1: Some(sharing_nearby::V1Frame {
                r#type: Some(sharing_nearby::v1_frame::FrameType::Response.into()),
                connection_response: Some(sharing_nearby::ConnectionResponseFrame {
                    status: Some(sharing_nearby::connection_response_frame::Status::Accept.into()),
                }),
                ..Default::default()
            }),
        };

        self.send_encrypted_frame(&frame).await?;

        self.update_state(
            |e| {
                e.state = State::ReceivingFiles;
            },
            true,
        )
        .await;

        Ok(())
    }

    async fn reject_transfer(
        &mut self,
        reason: Option<sharing_nearby::connection_response_frame::Status>,
    ) -> Result<(), anyhow::Error> {
        let sreason = if let Some(r) = reason {
            r
        } else {
            sharing_nearby::connection_response_frame::Status::Reject
        };

        let frame = sharing_nearby::Frame {
            version: Some(sharing_nearby::frame::Version::V1.into()),
            v1: Some(sharing_nearby::V1Frame {
                r#type: Some(sharing_nearby::v1_frame::FrameType::Response.into()),
                connection_response: Some(sharing_nearby::ConnectionResponseFrame {
                    status: Some(sreason.into()),
                }),
                ..Default::default()
            }),
        };

        self.send_encrypted_frame(&frame).await?;

        Ok(())
    }

    async fn finalize_key_exchange(
        &mut self,
        raw_peer_key: GenericPublicKey,
    ) -> Result<(), anyhow::Error> {
        let peer_p256_key = raw_peer_key
            .ec_p256_public_key
            .ok_or_else(|| anyhow!("Missing required fields"))?;
        let peer_key = crypto::decode_public_key(&peer_p256_key.x, &peer_p256_key.y)?;

        let keys = crypto::derive_session_keys(
            self.state.private_key.as_ref().unwrap(),
            &peer_key,
            self.state.client_init_msg_data.as_ref().unwrap(),
            self.state.server_init_data.as_ref().unwrap(),
            Role::Server,
        )?;

        self.update_state(
            |e| {
                e.decrypt_key = Some(keys.decrypt_key);
                e.recv_hmac_key = Some(keys.recv_hmac_key);
                e.encrypt_key = Some(keys.encrypt_key);
                e.send_hmac_key = Some(keys.send_hmac_key);
                e.pin_code = Some(keys.pin_code);
                e.encryption_done = true;
            },
            false,
        )
        .await;

        info!("Pin code: {:?}", self.state.pin_code);

        Ok(())
    }

    async fn send_ukey2_alert(&mut self, atype: AlertType) -> Result<(), anyhow::Error> {
        let alert = Ukey2Alert {
            r#type: Some(atype.into()),
            error_message: None,
        };

        let data = Ukey2Message {
            message_type: Some(ukey2_message::Type::Alert.into()),
            message_data: Some(alert.encode_to_vec()),
        };

        self.send_frame(data.encode_to_vec()).await
    }

    async fn send_encrypted_frame(
        &mut self,
        frame: &sharing_nearby::Frame,
    ) -> Result<(), anyhow::Error> {
        let frame_data = frame.encode_to_vec();
        let body_size = frame_data.len();

        let payload_header = PayloadHeader {
            id: Some(rand::random_range(i64::MIN..i64::MAX)),
            r#type: Some(payload_header::PayloadType::Bytes.into()),
            total_size: Some(body_size as i64),
            is_sensitive: Some(false),
            ..Default::default()
        };

        let transfer = PayloadTransferFrame {
            packet_type: Some(PacketType::Data.into()),
            payload_chunk: Some(PayloadChunk {
                offset: Some(0),
                flags: Some(0),
                body: Some(frame_data),
            }),
            payload_header: Some(payload_header.clone()),
            ..Default::default()
        };

        let wrapper = location_nearby_connections::OfflineFrame {
            version: Some(location_nearby_connections::offline_frame::Version::V1.into()),
            v1: Some(location_nearby_connections::V1Frame {
                r#type: Some(
                    location_nearby_connections::v1_frame::FrameType::PayloadTransfer.into(),
                ),
                payload_transfer: Some(transfer),
                ..Default::default()
            }),
        };

        // Encrypt and send offline
        self.encrypt_and_send(&wrapper).await?;

        // Send lastChunk
        let transfer = PayloadTransferFrame {
            packet_type: Some(PacketType::Data.into()),
            payload_chunk: Some(PayloadChunk {
                offset: Some(body_size as i64),
                flags: Some(1), // lastChunk
                body: Some(vec![]),
            }),
            payload_header: Some(payload_header),
            ..Default::default()
        };

        let wrapper = location_nearby_connections::OfflineFrame {
            version: Some(location_nearby_connections::offline_frame::Version::V1.into()),
            v1: Some(location_nearby_connections::V1Frame {
                r#type: Some(
                    location_nearby_connections::v1_frame::FrameType::PayloadTransfer.into(),
                ),
                payload_transfer: Some(transfer),
                ..Default::default()
            }),
        };

        // Encrypt and send offline
        self.encrypt_and_send(&wrapper).await?;

        Ok(())
    }

    async fn encrypt_and_send(&mut self, frame: &OfflineFrame) -> Result<(), anyhow::Error> {
        let d2d_msg = DeviceToDeviceMessage {
            sequence_number: Some(self.get_server_seq_inc().await),
            message: Some(frame.encode_to_vec()),
        };

        let iv = gen_random(16);
        let encrypted = crypto::encrypt(
            self.state.encrypt_key.as_ref().unwrap(),
            &iv,
            &d2d_msg.encode_to_vec(),
        )?;

        let hb = HeaderAndBody {
            body: encrypted,
            header: Header {
                encryption_scheme: EncScheme::Aes256Cbc.into(),
                signature_scheme: SigScheme::HmacSha256.into(),
                iv: Some(iv),
                public_metadata: Some(
                    GcmMetadata {
                        r#type: Type::DeviceToDeviceMessage.into(),
                        version: Some(1),
                    }
                    .encode_to_vec(),
                ),
                ..Default::default()
            },
        };

        let header_and_body = hb.encode_to_vec();
        let signature = crypto::sign(self.state.send_hmac_key.as_ref().unwrap(), &header_and_body)?;

        let smsg = SecureMessage {
            header_and_body,
            signature,
        };

        self.send_frame(smsg.encode_to_vec()).await?;

        Ok(())
    }

    async fn send_keepalive(&mut self, ack: bool) -> Result<(), anyhow::Error> {
        let ack_frame = location_nearby_connections::OfflineFrame {
            version: Some(location_nearby_connections::offline_frame::Version::V1.into()),
            v1: Some(location_nearby_connections::V1Frame {
                r#type: Some(location_nearby_connections::v1_frame::FrameType::KeepAlive.into()),
                keep_alive: Some(KeepAliveFrame { ack: Some(ack) }),
                ..Default::default()
            }),
        };

        if self.state.encryption_done {
            self.encrypt_and_send(&ack_frame).await
        } else {
            self.send_frame(ack_frame.encode_to_vec()).await
        }
    }

    async fn send_frame(&mut self, data: Vec<u8>) -> Result<(), anyhow::Error> {
        self.transport.write_frame(&data).await
    }

    async fn get_server_seq_inc(&mut self) -> i32 {
        self.update_state(
            |e| {
                e.server_seq += 1;
            },
            false,
        )
        .await;

        self.state.server_seq
    }

    async fn get_client_seq_inc(&mut self) -> i32 {
        self.update_state(
            |e| {
                e.client_seq += 1;
            },
            false,
        )
        .await;

        self.state.client_seq
    }

    async fn update_state<F>(&mut self, f: F, inform: bool)
    where
        F: FnOnce(&mut InnerState),
    {
        f(&mut self.state);

        if !inform {
            return;
        }

        trace!("Sending msg into the channel");
        let _ = self.sender.send(ChannelMessage {
            id: self.state.id.clone(),
            direction: ChannelDirection::LibToFront,
            rtype: Some(crate::channel::TransferType::Inbound),
            state: Some(self.state.state.clone()),
            meta: self.state.transfer_metadata.clone(),
            ..Default::default()
        });
        // Add a small sleep timer to allow the Tokio runtime to have
        // some spare time to process channel's message. Otherwise it
        // get spammed by new requests. Currently set to 10 micro secs.
        tokio::time::sleep(SANITY_DURATION).await;
    }
}

#[cfg(test)]
mod tests {
    use tokio::sync::broadcast;

    use super::*;
    use crate::hdl::UpgradeRegistry;
    use crate::location_nearby_connections::{
        BandwidthUpgradeNegotiationFrame, ConnectionRequestFrame, ConnectionResponseFrame, V1Frame,
        offline_frame, v1_frame,
    };
    use crate::securegcm::ukey2_client_init::CipherCommitment;
    use crate::utils::{DeviceType, encode_endpoint_info};

    /// The phone's side of a session, just enough to drive an upgrade.
    struct Phone {
        keys: crypto::SessionKeys,
        send_seq: i32,
        recv_seq: i32,
    }

    impl Phone {
        /// Connects and runs UKEY2 over `ble`, up to the encrypted connection.
        async fn connect(ble: &mut Transport) -> Self {
            ble.write_frame(&v1(V1Frame {
                r#type: Some(v1_frame::FrameType::ConnectionRequest.into()),
                connection_request: Some(ConnectionRequestFrame {
                    endpoint_id: Some("PHNE".into()),
                    endpoint_info: Some(encode_endpoint_info(DeviceType::Phone, "Pixel")),
                    ..Default::default()
                }),
                ..Default::default()
            }))
            .await
            .unwrap();

            let (secret_key, public_key) = crypto::gen_keypair();
            let (x, y) = crypto::encode_public_key(&public_key);
            let public_key = GenericPublicKey {
                r#type: PublicKeyType::EcP256.into(),
                ec_p256_public_key: Some(EcP256PublicKey { x, y }),
                ..Default::default()
            };
            let finish = Ukey2Message {
                message_type: Some(ukey2_message::Type::ClientFinish.into()),
                message_data: Some(
                    Ukey2ClientFinished {
                        public_key: Some(public_key.encode_to_vec()),
                    }
                    .encode_to_vec(),
                ),
            }
            .encode_to_vec();
            let init = Ukey2Message {
                message_type: Some(ukey2_message::Type::ClientInit.into()),
                message_data: Some(
                    Ukey2ClientInit {
                        version: Some(1),
                        random: Some(gen_random(32)),
                        next_protocol: Some("AES_256_CBC-HMAC_SHA256".into()),
                        cipher_commitments: vec![CipherCommitment {
                            handshake_cipher: Some(Ukey2HandshakeCipher::P256Sha512.into()),
                            commitment: Some(Sha512::digest(&finish).to_vec()),
                        }],
                    }
                    .encode_to_vec(),
                ),
            }
            .encode_to_vec();
            ble.write_frame(&init).await.unwrap();

            let server_init = ble.read_frame().await.unwrap();
            let msg = Ukey2Message::decode(&*server_init).unwrap();
            let server_key = Ukey2ServerInit::decode(msg.message_data()).unwrap();
            let server_key = GenericPublicKey::decode(server_key.public_key())
                .unwrap()
                .ec_p256_public_key
                .unwrap();
            let server_key = crypto::decode_public_key(&server_key.x, &server_key.y).unwrap();
            ble.write_frame(&finish).await.unwrap();

            let keys = crypto::derive_session_keys(
                &secret_key,
                &server_key,
                &init,
                &server_init,
                Role::Client,
            )
            .unwrap();

            ble.write_frame(&v1(V1Frame {
                r#type: Some(v1_frame::FrameType::ConnectionResponse.into()),
                connection_response: Some(ConnectionResponseFrame::default()),
                ..Default::default()
            }))
            .await
            .unwrap();
            let response = OfflineFrame::decode(&*ble.read_frame().await.unwrap()).unwrap();
            assert_eq!(
                response.v1.unwrap().r#type(),
                v1_frame::FrameType::ConnectionResponse
            );

            Self {
                keys,
                send_seq: 0,
                recv_seq: 0,
            }
        }

        async fn send(&mut self, transport: &mut Transport, frame: &OfflineFrame) {
            self.send_seq += 1;
            let d2d = DeviceToDeviceMessage {
                sequence_number: Some(self.send_seq),
                message: Some(frame.encode_to_vec()),
            };
            let iv = gen_random(16);
            let hb = HeaderAndBody {
                body: crypto::encrypt(&self.keys.encrypt_key, &iv, &d2d.encode_to_vec()).unwrap(),
                header: Header {
                    encryption_scheme: EncScheme::Aes256Cbc.into(),
                    signature_scheme: SigScheme::HmacSha256.into(),
                    iv: Some(iv),
                    ..Default::default()
                },
            }
            .encode_to_vec();
            let smsg = SecureMessage {
                signature: crypto::sign(&self.keys.send_hmac_key, &hb).unwrap(),
                header_and_body: hb,
            };
            transport.write_frame(&smsg.encode_to_vec()).await.unwrap();
        }

        async fn recv(&mut self, transport: &mut Transport) -> OfflineFrame {
            let smsg = SecureMessage::decode(&*transport.read_frame().await.unwrap()).unwrap();
            crypto::verify(
                &self.keys.recv_hmac_key,
                &smsg.header_and_body,
                &smsg.signature,
            )
            .unwrap();
            let hb = HeaderAndBody::decode(&*smsg.header_and_body).unwrap();
            let plain = crypto::decrypt(&self.keys.decrypt_key, hb.header.iv(), &hb.body).unwrap();
            let d2d = DeviceToDeviceMessage::decode(&*plain).unwrap();

            self.recv_seq += 1;
            assert_eq!(d2d.sequence_number(), self.recv_seq);
            OfflineFrame::decode(d2d.message()).unwrap()
        }

        /// Receives frames up to the first one that isn't a payload transfer.
        async fn recv_control(&mut self, transport: &mut Transport) -> OfflineFrame {
            loop {
                let frame = self.recv(transport).await;
                if frame.v1.as_ref().unwrap().r#type() != v1_frame::FrameType::PayloadTransfer {
                    return frame;
                }
            }
        }
    }

    fn v1(frame: V1Frame) -> Vec<u8> {
        OfflineFrame {
            version: Some(offline_frame::Version::V1.into()),
            v1: Some(frame),
        }
        .encode_to_vec()
    }

    fn keep_alive() -> OfflineFrame {
        OfflineFrame::decode(&*v1(V1Frame {
            r#type: Some(v1_frame::FrameType::KeepAlive.into()),
            keep_alive: Some(KeepAliveFrame { ack: Some(false) }),
            ..Default::default()
        }))
        .unwrap()
    }

    fn bwu_event(event: UpgradeEvent) -> OfflineFrame {
        OfflineFrame::decode(&*v1(V1Frame {
            r#type: Some(v1_frame::FrameType::BandwidthUpgradeNegotiation.into()),
            bandwidth_upgrade_negotiation: Some(BandwidthUpgradeNegotiationFrame {
                event_type: Some(event.into()),
                ..Default::default()
            }),
            ..Default::default()
        }))
        .unwrap()
    }

    fn is_keep_alive(frame: &OfflineFrame) -> bool {
        frame.v1.as_ref().unwrap().r#type() == v1_frame::FrameType::KeepAlive
    }

    /// A BLE session over an in-memory channel, offering the upgrade.
    fn ble_session() -> (Transport, WifiLanUpgrade) {
        let (ours, phone) = tokio::io::duplex(64 * 1024);
        let upgrade = WifiLanUpgrade {
            port: 4242,
            registry: UpgradeRegistry::default(),
        };
        let (sender, _) = broadcast::channel(50);
        tokio::spawn(
            InboundRequest::new(Transport::new(ours), "ble-test".into(), sender)
                .with_wifi_lan_upgrade(upgrade.clone())
                .run(None),
        );

        (Transport::new(phone), upgrade)
    }

    #[tokio::test]
    async fn ble_session_upgrades_to_wifi_lan() {
        if lan_ipv4().is_none() {
            eprintln!("no LAN address on this machine, skipping");
            return;
        }

        let (mut ble, upgrade) = ble_session();
        let mut phone = Phone::connect(&mut ble).await;

        // The paired key encryption, then the upgrade offer.
        let offer = phone.recv_control(&mut ble).await;
        assert_eq!(bwu::event(&offer), Some(UpgradeEvent::UpgradePathAvailable));
        let path = offer
            .v1
            .unwrap()
            .bandwidth_upgrade_negotiation
            .unwrap()
            .upgrade_path_info
            .unwrap();
        assert_eq!(path.wifi_lan_socket.unwrap().wifi_port(), 4242);

        // Still served over BLE while the phone comes over.
        phone.send(&mut ble, &keep_alive()).await;
        assert!(is_keep_alive(&phone.recv(&mut ble).await));

        // What the TCP server does with a CLIENT_INTRODUCTION.
        let (tcp_ours, tcp_phone) = tokio::io::duplex(64 * 1024);
        let mut tcp = Transport::new(tcp_phone);
        assert!(upgrade.registry.deliver("PHNE", Transport::new(tcp_ours)));
        let ack = OfflineFrame::decode(&*tcp.read_frame().await.unwrap()).unwrap();
        assert_eq!(bwu::event(&ack), Some(UpgradeEvent::ClientIntroductionAck));

        // Draining: a frame sent before the phone's LAST_WRITE is still processed.
        phone.send(&mut ble, &keep_alive()).await;
        phone
            .send(&mut ble, &bwu_event(UpgradeEvent::LastWriteToPriorChannel))
            .await;
        assert_eq!(
            bwu::event(&phone.recv(&mut ble).await),
            Some(UpgradeEvent::LastWriteToPriorChannel)
        );
        assert!(is_keep_alive(&phone.recv(&mut ble).await));
        assert_eq!(
            bwu::event(&phone.recv(&mut ble).await),
            Some(UpgradeEvent::SafeToClosePriorChannel)
        );
        phone
            .send(&mut ble, &bwu_event(UpgradeEvent::SafeToClosePriorChannel))
            .await;

        // Plaintext DISCONNECTION both ways, then BLE is dropped.
        let disconnection = OfflineFrame::decode(&*ble.read_frame().await.unwrap()).unwrap();
        assert_eq!(
            disconnection.v1.unwrap().r#type(),
            v1_frame::FrameType::Disconnection
        );
        ble.write_frame(&bwu::prior_channel_disconnection().encode_to_vec())
            .await
            .unwrap();
        assert!(ble.read_frame().await.is_err());

        // Same keys, and sequence numbers carry on over TCP.
        phone.send(&mut tcp, &keep_alive()).await;
        assert!(is_keep_alive(&phone.recv(&mut tcp).await));
    }

    #[tokio::test]
    async fn ble_session_stays_on_ble_when_the_phone_cannot_upgrade() {
        if lan_ipv4().is_none() {
            eprintln!("no LAN address on this machine, skipping");
            return;
        }

        let (mut ble, upgrade) = ble_session();
        let mut phone = Phone::connect(&mut ble).await;
        let offer = phone.recv_control(&mut ble).await;
        assert_eq!(bwu::event(&offer), Some(UpgradeEvent::UpgradePathAvailable));

        phone
            .send(&mut ble, &bwu_event(UpgradeEvent::UpgradeFailure))
            .await;
        phone.send(&mut ble, &keep_alive()).await;
        assert!(is_keep_alive(&phone.recv(&mut ble).await));

        // Nobody waits for the phone anymore.
        let (tcp_ours, _) = tokio::io::duplex(16);
        assert!(!upgrade.registry.deliver("PHNE", Transport::new(tcp_ours)));
    }

    #[test]
    fn wifi_credentials_wpa_psk() {
        let creds = WifiCredentials {
            password: Some("testpass123".to_string()),
            hidden_ssid: Some(false),
        };
        let encoded = creds.encode_to_vec();

        let decoded = WifiCredentials::decode(encoded.as_slice()).unwrap();
        assert_eq!(decoded.password, Some("testpass123".to_string()));
        assert_eq!(decoded.hidden_ssid, Some(false));
    }

    #[test]
    fn wifi_credentials_open_network() {
        let creds = WifiCredentials {
            password: Some("".to_string()),
            hidden_ssid: Some(false),
        };
        let encoded = creds.encode_to_vec();

        let decoded = WifiCredentials::decode(encoded.as_slice()).unwrap();
        assert_eq!(decoded.password, Some("".to_string()));
        assert_eq!(decoded.hidden_ssid, Some(false));
    }

    #[test]
    fn wifi_credentials_hidden_network() {
        let creds = WifiCredentials {
            password: Some("secretpass".to_string()),
            hidden_ssid: Some(true),
        };
        let encoded = creds.encode_to_vec();

        let decoded = WifiCredentials::decode(encoded.as_slice()).unwrap();
        assert_eq!(decoded.password, Some("secretpass".to_string()));
        assert_eq!(decoded.hidden_ssid, Some(true));
    }

    #[test]
    fn wifi_credentials_sae() {
        // WPA3 SAE networks also use password format
        let creds = WifiCredentials {
            password: Some("wpa3pass".to_string()),
            hidden_ssid: Some(false),
        };
        let encoded = creds.encode_to_vec();

        let decoded = WifiCredentials::decode(encoded.as_slice()).unwrap();
        assert_eq!(decoded.password, Some("wpa3pass".to_string()));
        assert_eq!(decoded.hidden_ssid, Some(false));
    }
}
