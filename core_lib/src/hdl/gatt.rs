//! Quick Share receiving over Bluetooth LE, for phones that are off Wi-Fi.
//!
//! Recent Android phones (e.g. Pixels with "Share with Apple devices" on) leave
//! Wi-Fi while picking a share target, so they never see our mDNS service and
//! only know us from BLE. Nearby Connections then connects over a GATT "weave"
//! socket instead of TCP:
//!
//! 1. Discovery: we advertise service data under 0xFEF3 (see
//!    `blea::receiver_advertisement`): a mediums advertisement wrapping a
//!    connections advertisement with our endpoint id and endpoint info.
//! 2. GATT: the phone connects, discovers service 0xFEF3, reads the same
//!    advertisement from the slot 0 characteristic, subscribes to
//!    `…0102` (we notify) and writes to `…0101` (with response, so packets
//!    reach us one at a time and in order).
//! 3. Weave packets: each write/notification is one packet behind a 1-byte
//!    header, `1 | counter(3) | command(4)` for control packets and
//!    `0 | counter(3) | first(1) | last(1) | 00` for data packets, which are
//!    reassembled from the `first` one through the `last` one into a message.
//!    The phone opens with CONNECT_REQUEST `80 | min ver(2) | max ver(2) |
//!    max packet size(2)` and we answer CONNECT_CONFIRM `81 | ver(2) |
//!    packet size(2)`; our data packets then count up from 1.
//! 4. BLE socket: a message is `service id hash(3) | payload`. Hash `00 00 00`
//!    marks a socket control frame (`08 01` introduction, `08 02`
//!    disconnection); otherwise the payload is the usual `[len(4)][frame]`
//!    byte stream, exactly what TCP carries.
//!
//! So [`GattServer`] only strips and adds those layers and runs a regular
//! [`InboundRequest`] over an in-memory duplex. Right after the encrypted
//! connection is up, that request offers a bandwidth upgrade to Wi-Fi LAN,
//! because the socket moves tens of KB/s.

use bluer::gatt::local::{
    Application, ApplicationHandle, Characteristic, CharacteristicNotifier, CharacteristicNotify,
    CharacteristicNotifyMethod, CharacteristicRead, CharacteristicWrite, CharacteristicWriteMethod,
    ReqError, Service,
};
use bluer::{Address, Uuid, UuidExt};
use futures::StreamExt;
use tokio::io::{AsyncWriteExt, DuplexStream, ReadHalf, WriteHalf};
use tokio::sync::broadcast::Sender;
use tokio::sync::mpsc;
use tokio_util::codec::{FramedRead, LengthDelimitedCodec};
use tokio_util::sync::CancellationToken;

use super::{InboundRequest, Transport};
use crate::channel::ChannelMessage;
use crate::utils::SERVICE_ID_HASH;

const INNER_NAME: &str = "GattServer";

/// Nearby Connections' BLE service; the receiver advertisement is keyed by it too.
pub const SERVICE_UUID: u16 = 0xFEF3;
const ADVERTISEMENT_SLOT: Uuid = Uuid::from_u128(0x00000000_0000_3000_8000_000000000000);
const TO_PERIPHERAL: Uuid = Uuid::from_u128(0x00000100_0004_1000_8000_001a11000101);
const FROM_PERIPHERAL: Uuid = Uuid::from_u128(0x00000100_0004_1000_8000_001a11000102);

const WEAVE_CONTROL: u8 = 0x80;
const WEAVE_FIRST: u8 = 0x08;
const WEAVE_LAST: u8 = 0x04;
const WEAVE_COMMAND: u8 = 0x0F;
const COMMAND_CONNECT_REQUEST: u8 = 0;
const COMMAND_CONNECT_CONFIRM: u8 = 1;
const COMMAND_ERROR: u8 = 2;
const WEAVE_VERSION: u16 = 1;
/// Largest notification payload with a 512-byte ATT MTU.
const MAX_PACKET_SIZE: u16 = 509;
const MIN_PACKET_SIZE: u16 = 20;
/// Assumed when the connect request doesn't state a packet size.
const DEFAULT_PACKET_SIZE: u16 = 100;

const SOCKET_CONTROL_PREFIX: [u8; 3] = [0; 3];
/// `SocketControlFrame.type` (field 1, varint) values.
const SOCKET_CONTROL_INTRODUCTION: [u8; 2] = [0x08, 0x01];
const SOCKET_CONTROL_DISCONNECTION: [u8; 2] = [0x08, 0x02];

const DUPLEX_BUFFER: usize = 64 * 1024;

#[derive(Debug, PartialEq)]
enum WeavePacket<'a> {
    ConnectRequest {
        max_packet_size: u16,
    },
    Error,
    OtherControl,
    Data {
        first: bool,
        last: bool,
        payload: &'a [u8],
    },
}

fn parse_packet(packet: &[u8]) -> Option<WeavePacket<'_>> {
    let (&header, rest) = packet.split_first()?;
    if header & WEAVE_CONTROL == 0 {
        return Some(WeavePacket::Data {
            first: header & WEAVE_FIRST != 0,
            last: header & WEAVE_LAST != 0,
            payload: rest,
        });
    }

    Some(match header & WEAVE_COMMAND {
        COMMAND_CONNECT_REQUEST => WeavePacket::ConnectRequest {
            max_packet_size: rest
                .get(4..6)
                .map_or(DEFAULT_PACKET_SIZE, |b| u16::from_be_bytes([b[0], b[1]])),
        },
        COMMAND_ERROR => WeavePacket::Error,
        _ => WeavePacket::OtherControl,
    })
}

fn connect_confirm(packet_size: u16) -> Vec<u8> {
    let mut packet = vec![WEAVE_CONTROL | COMMAND_CONNECT_CONFIRM];
    packet.extend_from_slice(&WEAVE_VERSION.to_be_bytes());
    packet.extend_from_slice(&packet_size.to_be_bytes());
    packet
}

/// Splits outgoing messages into weave data packets.
struct PacketWriter {
    payload_size: usize,
    counter: u8,
}

impl PacketWriter {
    fn new(packet_size: u16) -> Self {
        Self {
            payload_size: packet_size as usize - 1,
            // The connect confirm used 0.
            counter: 1,
        }
    }

    fn packets(&mut self, message: &[u8]) -> Vec<Vec<u8>> {
        let count = message.len().div_ceil(self.payload_size);
        message
            .chunks(self.payload_size)
            .enumerate()
            .map(|(i, chunk)| {
                let mut header = (self.counter & 0x07) << 4;
                if i == 0 {
                    header |= WEAVE_FIRST;
                }
                if i + 1 == count {
                    header |= WEAVE_LAST;
                }
                self.counter = self.counter.wrapping_add(1);
                [&[header], chunk].concat()
            })
            .collect()
    }
}

/// Joins incoming weave data packets back into messages.
#[derive(Default)]
struct Reassembler {
    message: Vec<u8>,
}

impl Reassembler {
    fn push(&mut self, first: bool, last: bool, payload: &[u8]) -> Option<Vec<u8>> {
        if first {
            self.message.clear();
        }
        self.message.extend_from_slice(payload);
        last.then(|| std::mem::take(&mut self.message))
    }
}

#[derive(Debug, PartialEq)]
enum SocketMessage<'a> {
    Data(&'a [u8]),
    Introduction,
    Disconnection,
    OtherControl,
    Unknown,
}

fn parse_socket_message(message: &[u8]) -> SocketMessage<'_> {
    let Some((prefix, rest)) = message.split_at_checked(3) else {
        return SocketMessage::Unknown;
    };

    if prefix == SERVICE_ID_HASH {
        SocketMessage::Data(rest)
    } else if prefix == SOCKET_CONTROL_PREFIX {
        if rest.starts_with(&SOCKET_CONTROL_INTRODUCTION) {
            SocketMessage::Introduction
        } else if rest.starts_with(&SOCKET_CONTROL_DISCONNECTION) {
            SocketMessage::Disconnection
        } else {
            SocketMessage::OtherControl
        }
    } else {
        SocketMessage::Unknown
    }
}

/// One phone's weave socket, bridged to an [`InboundRequest`].
struct WeaveConnection {
    device: Address,
    to_inbound: WriteHalf<DuplexStream>,
    /// Yields each `[len][frame]` the request writes, length prefix included.
    from_inbound: FramedRead<ReadHalf<DuplexStream>, LengthDelimitedCodec>,
    writer: PacketWriter,
    reassembler: Reassembler,
}

/// Serves the 0xFEF3 GATT service and bridges its weave socket to the inbound
/// handshake. Notifications reach every subscriber, so one phone at a time.
pub struct GattServer {
    adapter: bluer::Adapter,
    _app: ApplicationHandle,
    packets: mpsc::UnboundedReceiver<(Address, Vec<u8>)>,
    notifiers: mpsc::UnboundedReceiver<CharacteristicNotifier>,
    sender: Sender<ChannelMessage>,

    notifier: Option<CharacteristicNotifier>,
    connection: Option<WeaveConnection>,
    /// A connect request that raced ahead of the phone's subscription.
    pending_connect: Option<(Address, u16)>,
}

impl GattServer {
    /// Registers the GATT service, serving `advertisement` on the slot 0 characteristic.
    pub async fn new(
        advertisement: Vec<u8>,
        sender: Sender<ChannelMessage>,
    ) -> Result<Self, anyhow::Error> {
        let session = bluer::Session::new().await?;
        let adapter = session.default_adapter().await?;

        let (packet_tx, packets) = mpsc::unbounded_channel();
        let (notifier_tx, notifiers) = mpsc::unbounded_channel();

        let app = Application {
            services: vec![Service {
                uuid: Uuid::from_u16(SERVICE_UUID),
                primary: true,
                characteristics: vec![
                    Characteristic {
                        uuid: ADVERTISEMENT_SLOT,
                        read: Some(CharacteristicRead {
                            read: true,
                            fun: Box::new(move |req| {
                                let value = advertisement
                                    .get(req.offset as usize..)
                                    .map(<[u8]>::to_vec)
                                    .ok_or(ReqError::InvalidOffset);
                                Box::pin(async move { value })
                            }),
                            ..Default::default()
                        }),
                        ..Default::default()
                    },
                    Characteristic {
                        uuid: TO_PERIPHERAL,
                        write: Some(CharacteristicWrite {
                            // With response only: the phone then sends one packet at a
                            // time, so they reach us in order.
                            write: true,
                            method: CharacteristicWriteMethod::Fun(Box::new(move |value, req| {
                                let _ = packet_tx.send((req.device_address, value));
                                Box::pin(async { Ok(()) })
                            })),
                            ..Default::default()
                        }),
                        ..Default::default()
                    },
                    Characteristic {
                        uuid: FROM_PERIPHERAL,
                        notify: Some(CharacteristicNotify {
                            notify: true,
                            indicate: true,
                            method: CharacteristicNotifyMethod::Fun(Box::new(move |notifier| {
                                let _ = notifier_tx.send(notifier);
                                Box::pin(async {})
                            })),
                            ..Default::default()
                        }),
                        ..Default::default()
                    },
                ],
                ..Default::default()
            }],
            ..Default::default()
        };

        let app = adapter.serve_gatt_application(app).await?;

        Ok(Self {
            adapter,
            _app: app,
            packets,
            notifiers,
            sender,
            notifier: None,
            connection: None,
            pending_connect: None,
        })
    }

    pub fn adapter(&self) -> &bluer::Adapter {
        &self.adapter
    }

    pub async fn run(mut self, ctk: CancellationToken) {
        info!(
            "{INNER_NAME}: serving 0x{SERVICE_UUID:04X} on {}",
            self.adapter.name()
        );

        loop {
            tokio::select! {
                biased;
                _ = ctk.cancelled() => {
                    info!("{INNER_NAME}: tracker cancelled, breaking");
                    break;
                }
                Some(notifier) = self.notifiers.recv() => {
                    debug!("{INNER_NAME}: phone subscribed");
                    // BlueZ only starts a new session once every earlier subscriber is gone.
                    self.close("superseded by a new subscription");
                    self.notifier = Some(notifier);
                    if let Some((device, packet_size)) = self.pending_connect.take() {
                        self.connect(device, packet_size).await;
                    }
                }
                _ = stopped(self.notifier.as_ref()) => {
                    debug!("{INNER_NAME}: phone unsubscribed");
                    self.notifier = None;
                    self.close("phone unsubscribed");
                }
                Some((device, packet)) = self.packets.recv() => {
                    self.on_packet(device, &packet).await;
                }
                frame = next_frame(self.connection.as_mut()) => match frame {
                    Some(Ok(frame)) => self.send(&frame).await,
                    Some(Err(e)) => self.close(&format!("bad frame from the handshake: {e}")),
                    // The handshake ended, or moved to Wi-Fi.
                    None => self.close("inbound side closed"),
                },
            }
        }
    }

    async fn on_packet(&mut self, device: Address, packet: &[u8]) {
        match parse_packet(packet) {
            Some(WeavePacket::ConnectRequest { max_packet_size }) => {
                let packet_size = max_packet_size.clamp(MIN_PACKET_SIZE, MAX_PACKET_SIZE);
                self.close("phone reconnected");
                if self.notifier.is_some() {
                    self.connect(device, packet_size).await;
                } else {
                    self.pending_connect = Some((device, packet_size));
                }
            }
            Some(WeavePacket::Error) => {
                if self.is_connected_to(device) {
                    self.close("phone sent a weave error");
                }
            }
            Some(WeavePacket::Data {
                first,
                last,
                payload,
            }) => {
                if !self.is_connected_to(device) {
                    trace!("{INNER_NAME}: dropping a packet from {device} outside a connection");
                    return;
                }
                let Some(connection) = self.connection.as_mut() else {
                    return;
                };
                let Some(message) = connection.reassembler.push(first, last, payload) else {
                    return;
                };

                match parse_socket_message(&message) {
                    SocketMessage::Data(data) => {
                        if let Err(e) = connection.to_inbound.write_all(data).await {
                            self.close(&format!("handshake gone: {e}"));
                        }
                    }
                    SocketMessage::Introduction => debug!("{INNER_NAME}: socket introduction"),
                    SocketMessage::Disconnection => self.close("phone disconnected the socket"),
                    SocketMessage::OtherControl => debug!("{INNER_NAME}: socket control message"),
                    SocketMessage::Unknown => warn!(
                        "{INNER_NAME}: dropping a socket message for another service ({} bytes)",
                        message.len()
                    ),
                }
            }
            Some(WeavePacket::OtherControl) | None => {}
        }
    }

    async fn connect(&mut self, device: Address, packet_size: u16) {
        let Some(notifier) = self.notifier.as_mut() else {
            return;
        };
        if let Err(e) = notifier.notify(connect_confirm(packet_size)).await {
            warn!("{INNER_NAME}: couldn't confirm the weave connection: {e}");
            return;
        }

        let (inbound_side, bridge_side) = tokio::io::duplex(DUPLEX_BUFFER);
        let (from_inbound, to_inbound) = tokio::io::split(bridge_side);
        let codec = LengthDelimitedCodec::builder()
            .length_field_length(4)
            .big_endian()
            .num_skip(0)
            .new_codec();

        let id = format!("ble-{:08x}", rand::random::<u32>());
        info!("{INNER_NAME}: weave socket open with {device} as {id} ({packet_size}-byte packets)");

        let inbound = InboundRequest::new(Transport::new(inbound_side), id, self.sender.clone());
        tokio::spawn(inbound.run());

        self.connection = Some(WeaveConnection {
            device,
            to_inbound,
            from_inbound: FramedRead::new(from_inbound, codec),
            writer: PacketWriter::new(packet_size),
            reassembler: Reassembler::default(),
        });
    }

    /// Sends one `[len][frame]` from the handshake to the phone.
    async fn send(&mut self, frame: &[u8]) {
        let (Some(notifier), Some(connection)) = (self.notifier.as_mut(), self.connection.as_mut())
        else {
            return;
        };

        let message = [&SERVICE_ID_HASH[..], frame].concat();
        for packet in connection.writer.packets(&message) {
            if let Err(e) = notifier.notify(packet).await {
                self.close(&format!("notification failed: {e}"));
                return;
            }
        }
    }

    fn is_connected_to(&self, device: Address) -> bool {
        self.connection.as_ref().is_some_and(|c| c.device == device)
    }

    /// Drops the current weave connection, which ends its handshake unless it
    /// already moved to Wi-Fi.
    fn close(&mut self, reason: &str) {
        if let Some(connection) = self.connection.take() {
            info!(
                "{INNER_NAME}: weave socket with {} closed: {reason}",
                connection.device
            );
        }
    }
}

async fn stopped(notifier: Option<&CharacteristicNotifier>) {
    match notifier {
        Some(notifier) => notifier.stopped().await,
        None => std::future::pending().await,
    }
}

async fn next_frame(
    connection: Option<&mut WeaveConnection>,
) -> Option<Result<bytes::BytesMut, std::io::Error>> {
    match connection {
        Some(connection) => connection.from_inbound.next().await,
        None => std::future::pending().await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_connect_request() {
        assert_eq!(
            parse_packet(&[0x80, 0x00, 0x01, 0x00, 0x01, 0x01, 0xFD]),
            Some(WeavePacket::ConnectRequest {
                max_packet_size: 509
            })
        );
        assert_eq!(
            parse_packet(&[0x80]),
            Some(WeavePacket::ConnectRequest {
                max_packet_size: DEFAULT_PACKET_SIZE
            })
        );
        assert_eq!(parse_packet(&[0x92]), Some(WeavePacket::Error));
        assert_eq!(parse_packet(&[]), None);
    }

    #[test]
    fn parses_data_packets() {
        assert_eq!(
            parse_packet(&[0x1C, 1, 2]),
            Some(WeavePacket::Data {
                first: true,
                last: true,
                payload: &[1, 2]
            })
        );
        assert_eq!(
            parse_packet(&[0x20]),
            Some(WeavePacket::Data {
                first: false,
                last: false,
                payload: &[]
            })
        );
    }

    #[test]
    fn connect_confirm_bytes() {
        assert_eq!(connect_confirm(509), [0x81, 0x00, 0x01, 0x01, 0xFD]);
    }

    #[test]
    fn fragments_and_reassembles() {
        let message: Vec<u8> = (0..1200).map(|i| i as u8).collect();
        let mut writer = PacketWriter::new(509);
        let packets = writer.packets(&message);

        assert_eq!(packets.len(), 3);
        // Counters start at 1, first/last flags on the ends only.
        assert_eq!(packets[0][0], 0x18);
        assert_eq!(packets[1][0], 0x20);
        assert_eq!(packets[2][0], 0x34);
        assert!(packets.iter().all(|p| p.len() <= 509));

        let mut reassembler = Reassembler::default();
        let mut out = None;
        for packet in &packets {
            let Some(WeavePacket::Data {
                first,
                last,
                payload,
            }) = parse_packet(packet)
            else {
                panic!("not a data packet");
            };
            out = reassembler.push(first, last, payload);
        }
        assert_eq!(out, Some(message));
    }

    #[test]
    fn single_packet_message_and_counter_wrap() {
        let mut writer = PacketWriter::new(20);
        for i in 1..=9u8 {
            let packets = writer.packets(&[0xAB]);
            assert_eq!(packets, vec![vec![((i & 0x07) << 4) | 0x0C, 0xAB]]);
        }
    }

    #[test]
    fn first_packet_discards_a_partial_message() {
        let mut reassembler = Reassembler::default();
        assert_eq!(reassembler.push(true, false, &[1, 2]), None);
        assert_eq!(reassembler.push(true, true, &[3]), Some(vec![3]));
    }

    #[test]
    fn parses_socket_messages() {
        assert_eq!(
            parse_socket_message(&[0xFC, 0x9F, 0x5E, 0, 0, 0, 1, 9]),
            SocketMessage::Data(&[0, 0, 0, 1, 9])
        );
        assert_eq!(
            parse_socket_message(&[0, 0, 0, 0x08, 0x01, 0x12, 0x00]),
            SocketMessage::Introduction
        );
        assert_eq!(
            parse_socket_message(&[0, 0, 0, 0x08, 0x02]),
            SocketMessage::Disconnection
        );
        assert_eq!(
            parse_socket_message(&[0, 0, 0, 0x08, 0x03]),
            SocketMessage::OtherControl
        );
        assert_eq!(parse_socket_message(&[1, 2, 3, 4]), SocketMessage::Unknown);
        assert_eq!(parse_socket_message(&[0xFC]), SocketMessage::Unknown);
    }
}
