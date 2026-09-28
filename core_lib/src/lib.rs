#[macro_use]
extern crate log;

use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};

use anyhow::anyhow;
use channel::ChannelMessage;
use hdl::{BleAdvertiser, MDnsDiscovery};
use rand::RngExt;
use rand::distr::Alphanumeric;
use tokio::net::TcpListener;
use tokio::sync::{broadcast, mpsc, watch};
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use crate::hdl::{
    BleListener, GattServer, MDnsServer, ReceiverAdvertiser, UpgradeRegistry, WifiLanUpgrade,
    receiver_advertisement,
};
use crate::localsend::LocalSend;
use crate::manager::TcpServer;

pub mod channel;
mod errors;
mod hdl;
mod localsend;
mod manager;
mod utils;

pub use hdl::info::{WifiNetwork, WifiSecurity};
pub use hdl::{EndpointInfo, OutboundPayload, Protocol, State, TextPayloadType, Visibility};
pub use manager::SendInfo;
pub use utils::{
    DeviceType, RemoteDeviceInfo, effective_device_name, expand_directories, get_download_dir,
    hostname, is_web_url, normalize_device_name,
};

pub mod sharing_nearby {
    include!(concat!(env!("OUT_DIR"), "/sharing.nearby.rs"));
}

pub mod securemessage {
    include!(concat!(env!("OUT_DIR"), "/securemessage.rs"));
}

pub mod securegcm {
    include!(concat!(env!("OUT_DIR"), "/securegcm.rs"));
}

pub mod location_nearby_connections {
    include!(concat!(env!("OUT_DIR"), "/location.nearby.connections.rs"));
}

static CUSTOM_DOWNLOAD: RwLock<Option<PathBuf>> = RwLock::new(None);
static CUSTOM_DEVICE_NAME: RwLock<Option<String>> = RwLock::new(None);

/// Held by tests that set or depend on the download directory.
#[cfg(test)]
static DOWNLOAD_DIR_TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[derive(Debug)]
pub struct RQS {
    tracker: Option<TaskTracker>,
    ctoken: Option<CancellationToken>,
    // Discovery token is different than ctoken because he is on his own
    // - can be cancelled while the ctoken is still active
    discovery_ctk: Option<CancellationToken>,

    // Used to trigger a change in the mDNS visibility (and later on, BLE)
    pub visibility_sender: Arc<Mutex<watch::Sender<Visibility>>>,
    visibility_receiver: watch::Receiver<Visibility>,

    // Used to trigger device name changes in mDNS and BLE
    pub device_name_sender: Arc<Mutex<watch::Sender<String>>>,
    device_name_receiver: watch::Receiver<String>,

    // Only used to send the info "a nearby device is sharing"
    ble_sender: broadcast::Sender<()>,

    /// Quick Share's TCP port; any free one when unset.
    port_number: Option<u16>,
    /// LocalSend's HTTPS port; its default when unset.
    localsend_port: Option<u16>,
    /// Keeps the LocalSend certificate, and so our LocalSend identity.
    data_dir: Option<PathBuf>,
    localsend: Option<LocalSend>,

    pub message_sender: broadcast::Sender<ChannelMessage>,
}

impl RQS {
    pub fn new(
        visibility: Visibility,
        port_number: Option<u16>,
        localsend_port: Option<u16>,
        download_path: Option<PathBuf>,
        device_name: Option<String>,
        data_dir: Option<PathBuf>,
    ) -> Self {
        *CUSTOM_DOWNLOAD.write().unwrap() = download_path;
        *CUSTOM_DEVICE_NAME.write().unwrap() = device_name;

        let (message_sender, _) = broadcast::channel(50);
        let (ble_sender, _) = broadcast::channel(5);

        // Define default visibility as per the args inside the new()
        let (visibility_sender, visibility_receiver) = watch::channel(Visibility::Invisible);
        let _ = visibility_sender.send(visibility);

        // Initialize device_name with the effective name (normalized device_name or hostname)
        let effective_name = utils::effective_device_name();
        let (device_name_sender, device_name_receiver) = watch::channel(effective_name);

        Self {
            tracker: None,
            ctoken: None,
            discovery_ctk: None,
            visibility_sender: Arc::new(Mutex::new(visibility_sender)),
            visibility_receiver,
            device_name_sender: Arc::new(Mutex::new(device_name_sender)),
            device_name_receiver,
            ble_sender,
            port_number,
            localsend_port,
            data_dir,
            localsend: None,
            message_sender,
        }
    }

    pub async fn run(
        &mut self,
    ) -> Result<(mpsc::Sender<SendInfo>, broadcast::Receiver<()>), anyhow::Error> {
        let tracker = TaskTracker::new();
        let ctoken = CancellationToken::new();
        self.tracker = Some(tracker.clone());
        self.ctoken = Some(ctoken.clone());

        let endpoint_id: Vec<u8> = rand::rng().sample_iter(Alphanumeric).take(4).collect();
        let tcp_listener =
            TcpListener::bind(format!("0.0.0.0:{}", self.port_number.unwrap_or(0))).await?;
        let binded_addr = tcp_listener.local_addr()?;
        info!("TcpListener on: {}", binded_addr);

        // BLE sessions move to Wi-Fi through the TCP listener.
        let upgrade = WifiLanUpgrade {
            port: binded_addr.port(),
            registry: UpgradeRegistry::default(),
        };

        // Quick Share sends go through the TcpServer
        let (quick_share_sender, quick_share_receiver) = mpsc::channel(10);
        // Start TcpServer in own "task"
        let mut server = TcpServer::new(
            endpoint_id[..4].try_into()?,
            tcp_listener,
            self.message_sender.clone(),
            quick_share_receiver,
            upgrade.registry.clone(),
        )?;
        let ctk = ctoken.clone();
        tracker.spawn(async move { server.run(ctk).await });

        // LocalSend is a nice to have too, e.g. when its port is taken.
        self.localsend = LocalSend::start(
            self.localsend_port.unwrap_or(localsend::PORT),
            self.data_dir.clone(),
            self.device_name_receiver.clone(),
            self.visibility_receiver.clone(),
            self.message_sender.clone(),
            &tracker,
            ctoken.clone(),
        )
        .inspect_err(|e| warn!("LocalSend unavailable: {e}"))
        .ok();

        // Hands each send to the protocol the device was found over.
        let (send_sender, mut send_receiver) = mpsc::channel::<SendInfo>(10);
        let localsend = self.localsend.clone();
        let ctk = ctoken.clone();
        tracker.spawn(async move {
            loop {
                let info = tokio::select! {
                    _ = ctk.cancelled() => break,
                    info = send_receiver.recv() => match info {
                        Some(info) => info,
                        None => break,
                    },
                };
                match (info.protocol.clone(), &localsend) {
                    (Protocol::QuickShare, _) => {
                        let _ = quick_share_sender.send(info).await;
                    }
                    (Protocol::LocalSend { https }, Some(localsend)) => {
                        localsend.send(info, https);
                    }
                    (Protocol::LocalSend { .. }, None) => {
                        warn!("Can't send to {}: LocalSend is unavailable", info.name);
                    }
                }
            }
        });

        // Bluetooth is a nice to have: without it we still work over Wi-Fi LAN.
        match BleListener::new(self.ble_sender.clone()).await {
            Ok(ble) => {
                let ctk = ctoken.clone();
                tracker.spawn(async move {
                    if let Err(e) = ble.run(ctk).await {
                        warn!("BleListener stopped: {e}");
                    }
                });
            }
            Err(e) => warn!("BleListener unavailable: {e}"),
        }

        // Start MDnsServer in own "task"
        let mut mdns = MDnsServer::new(
            endpoint_id[..4].try_into()?,
            binded_addr.port(),
            self.ble_sender.subscribe(),
            self.visibility_sender.clone(),
            self.visibility_receiver.clone(),
            self.device_name_receiver.clone(),
        )?;
        let ctk = ctoken.clone();
        tracker.spawn(async move { mdns.run(ctk).await });

        // Receiving from phones that left Wi-Fi to share: a BLE advertisement
        // (same endpoint id as mDNS) and the GATT socket it leads to. Set up in
        // the background so a slow bluetoothd can't hold up the Wi-Fi side.
        let initial_device_name = utils::effective_device_name();
        let advertisement =
            receiver_advertisement(endpoint_id[..4].try_into()?, &initial_device_name);
        let sender = self.message_sender.clone();
        let visibility = self.visibility_receiver.clone();
        let device_name = self.device_name_receiver.clone();
        let endpoint_id_copy = endpoint_id[..4].try_into()?;
        let ctk = ctoken.clone();
        tracker.spawn(async move {
            let gatt = match GattServer::new(advertisement.clone(), sender, upgrade).await {
                Ok(gatt) => gatt,
                Err(e) => {
                    warn!("Receiving over Bluetooth unavailable: {e}");
                    return;
                }
            };
            let advertiser = ReceiverAdvertiser::new(
                gatt.adapter().clone(),
                endpoint_id_copy,
                visibility,
                device_name,
            );
            tokio::join!(advertiser.run(ctk.clone()), gatt.run(ctk));
        });

        tracker.close();

        Ok((send_sender, self.ble_sender.subscribe()))
    }

    pub fn discovery(
        &mut self,
        sender: broadcast::Sender<EndpointInfo>,
    ) -> Result<(), anyhow::Error> {
        self.stop_discovery();

        let tracker = self
            .tracker
            .as_ref()
            .ok_or_else(|| anyhow!("The service wasn't first started"))?;

        let ctk = CancellationToken::new();
        self.discovery_ctk = Some(ctk.clone());

        let ctk_blea = ctk.clone();
        tracker.spawn(async move {
            let blea = match BleAdvertiser::new().await {
                Ok(b) => b,
                Err(e) => {
                    error!("Couldn't init BleAdvertiser: {}", e);
                    return;
                }
            };

            if let Err(e) = blea.run(ctk_blea).await {
                error!("Couldn't start BleAdvertiser: {}", e);
            }
        });

        if let Some(localsend) = &self.localsend {
            localsend.start_discovery(sender.clone());
        }

        let discovery = MDnsDiscovery::new(sender)?;
        tracker.spawn(async move { discovery.run(ctk.clone()).await });

        Ok(())
    }

    pub fn stop_discovery(&mut self) {
        if let Some(localsend) = &self.localsend {
            localsend.stop_discovery();
        }
        if let Some(discovert_ctk) = &self.discovery_ctk {
            discovert_ctk.cancel();
            self.discovery_ctk = None;
        }
    }

    pub fn change_visibility(&mut self, nv: Visibility) {
        self.visibility_sender
            .lock()
            .unwrap()
            .send_modify(|state| *state = nv);
    }

    pub fn set_device_name(&self, name: String) {
        let name = utils::normalize_device_name(&name);
        *CUSTOM_DEVICE_NAME.write().unwrap() = (!name.is_empty()).then_some(name);

        let effective_name = utils::effective_device_name();
        self.device_name_sender
            .lock()
            .unwrap()
            .send_modify(|state| *state = effective_name);
    }

    pub async fn stop(&mut self) {
        self.stop_discovery();

        if let Some(ctoken) = &self.ctoken {
            ctoken.cancel();
        }

        if let Some(tracker) = &self.tracker {
            tracker.wait().await;
        }

        self.ctoken = None;
        self.tracker = None;
        self.localsend = None;
    }

    // Setting None here will resume the default settings
    pub fn set_download_path(&self, p: Option<PathBuf>) {
        debug!("Setting the download path to {:?}", p);
        let mut guard = CUSTOM_DOWNLOAD.write().unwrap();
        *guard = p;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_name_falls_back_to_hostname() {
        let _download_dir = DOWNLOAD_DIR_TEST_LOCK.blocking_lock();
        let rqs = RQS::new(
            Visibility::Visible,
            None,
            None,
            None,
            Some("Desk".into()),
            None,
        );
        assert_eq!(*rqs.device_name_receiver.borrow(), "Desk");

        rqs.set_device_name("  ".into());
        assert_eq!(*rqs.device_name_receiver.borrow(), utils::hostname());
    }
}
