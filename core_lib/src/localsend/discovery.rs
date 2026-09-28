//! Multicast discovery: announcing ourselves, and answering peers' announcements.

use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use socket2::{Domain, SockRef, Socket, Type};
use tokio::net::UdpSocket;
use tokio_util::sync::CancellationToken;

use super::{API, DeviceInfo, MULTICAST, PORT, Shared};
use crate::utils::private_ipv4s;

/// Repeats of an announcement, since multicast drops packets.
const ANNOUNCE_DELAYS: [Duration; 3] = [
    Duration::ZERO,
    Duration::from_secs(1),
    Duration::from_secs(3),
];
/// How often we announce ourselves while the frontend is looking for devices.
/// Peers that start later, or whose announcements we missed, hear one and
/// register with us over HTTP, which multicast can't drop.
const SEARCH_INTERVAL: Duration = Duration::from_secs(4);

struct Multicast {
    socket: UdpSocket,
    /// Picking the outgoing interface and sending must not interleave.
    send_lock: tokio::sync::Mutex<()>,
}

impl Multicast {
    fn bind() -> std::io::Result<Self> {
        let socket = Socket::new(Domain::IPV4, Type::DGRAM, None)?;
        // The official app may be running on this machine too.
        socket.set_reuse_address(true)?;
        socket.set_reuse_port(true)?;
        socket.bind(&SocketAddr::from((Ipv4Addr::UNSPECIFIED, PORT)).into())?;
        socket.set_nonblocking(true)?;
        // Every LAN we are on, not just the default route's, which may be a VPN.
        for ip in private_ipv4s() {
            if let Err(e) = socket.join_multicast_v4(&MULTICAST, &ip) {
                debug!("LocalSend: can't join multicast on {ip}: {e}");
            }
        }
        Ok(Self {
            socket: UdpSocket::from_std(socket.into())?,
            send_lock: tokio::sync::Mutex::new(()),
        })
    }

    /// Sends `info` to the multicast group on every LAN we are on.
    async fn send(&self, info: &DeviceInfo) {
        let Ok(bytes) = serde_json::to_vec(info) else {
            return;
        };
        let _guard = self.send_lock.lock().await;
        for ip in private_ipv4s() {
            if let Err(e) = SockRef::from(&self.socket).set_multicast_if_v4(&ip) {
                debug!("LocalSend: can't send multicast on {ip}: {e}");
                continue;
            }
            if let Err(e) = self.socket.send_to(&bytes, (MULTICAST, PORT)).await {
                debug!("LocalSend: multicast on {ip} failed: {e}");
            }
        }
    }
}

pub async fn run(shared: Arc<Shared>, ctk: CancellationToken) -> Result<(), anyhow::Error> {
    let multicast = Arc::new(Multicast::bind()?);
    // Peers already running learn about us without waiting for our next search.
    announce(&shared, &multicast);

    let mut buf = vec![0; 64 * 1024];
    let mut search = tokio::time::interval(SEARCH_INTERVAL);
    search.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            _ = ctk.cancelled() => return Ok(()),
            _ = shared.announce.notified() => announce(&shared, &multicast),
            _ = search.tick() => {
                if shared.discovery.lock().unwrap().is_some() {
                    multicast.send(&shared.info(Some(true))).await;
                }
            }
            received = multicast.socket.recv_from(&mut buf) => {
                let (len, from) = received?;
                let Ok(peer) = serde_json::from_slice::<DeviceInfo>(&buf[..len]) else {
                    continue;
                };
                if peer.fingerprint == shared.fingerprint {
                    continue;
                }
                shared.found(&peer, from.ip());
                if peer.announce == Some(true) && shared.visible() {
                    tokio::spawn(answer(shared.clone(), multicast.clone(), peer, from));
                }
            }
        }
    }
}

fn announce(shared: &Arc<Shared>, multicast: &Arc<Multicast>) {
    if !shared.visible() && shared.discovery.lock().unwrap().is_none() {
        return;
    }
    let (shared, multicast) = (shared.clone(), multicast.clone());
    tokio::spawn(async move {
        for delay in ANNOUNCE_DELAYS {
            tokio::time::sleep(delay).await;
            multicast.send(&shared.info(Some(true))).await;
        }
    });
}

/// Tells an announcing peer about us: by registering with it, else by multicast.
async fn answer(
    shared: Arc<Shared>,
    multicast: Arc<Multicast>,
    peer: DeviceInfo,
    from: SocketAddr,
) {
    let scheme = if peer.https() { "https" } else { "http" };
    let url = format!(
        "{scheme}://{}:{}{API}/register",
        from.ip(),
        peer.port.unwrap_or(PORT)
    );
    let registered = match shared.client(&peer.fingerprint, peer.https()) {
        Ok(client) => client
            .post(url)
            .timeout(Duration::from_secs(3))
            .json(&shared.info(None))
            .send()
            .await
            .is_ok_and(|response| response.status().is_success()),
        Err(e) => {
            debug!("LocalSend: can't register with {}: {e}", peer.alias);
            false
        }
    };
    if !registered {
        multicast.send(&shared.info(Some(false))).await;
    }
}
