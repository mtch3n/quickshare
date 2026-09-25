use std::sync::{Arc, Mutex};
use std::time::Duration;

use mdns_sd::{IfKind, ServiceDaemon, ServiceInfo};
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast::Receiver;
use tokio::sync::watch;
use tokio::time::{Instant, interval_at};
use tokio_util::sync::CancellationToken;
use ts_rs::TS;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;

use crate::utils::{DeviceType, encode_endpoint_info, gen_mdns_name, hostname};

const INNER_NAME: &str = "MDnsServer";
const TICK_INTERVAL: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum Visibility {
    Visible,
    Invisible,
    Temporarily,
}

pub struct MDnsServer {
    daemon: ServiceDaemon,
    service_info: ServiceInfo,
    endpoint_id: [u8; 4],
    service_port: u16,
    ble_receiver: Receiver<()>,
    visibility_sender: Arc<Mutex<watch::Sender<Visibility>>>,
    visibility_receiver: watch::Receiver<Visibility>,
    device_name_receiver: watch::Receiver<String>,
}

impl MDnsServer {
    pub fn new(
        endpoint_id: [u8; 4],
        service_port: u16,
        ble_receiver: Receiver<()>,
        visibility_sender: Arc<Mutex<watch::Sender<Visibility>>>,
        visibility_receiver: watch::Receiver<Visibility>,
        device_name_receiver: watch::Receiver<String>,
    ) -> Result<Self, anyhow::Error> {
        let service_info =
            Self::build_service(endpoint_id, service_port, DeviceType::Laptop, &hostname())?;

        // The TCP listener is IPv4-only, so only announce IPv4 addresses.
        let daemon = ServiceDaemon::new()?;
        daemon.disable_interface(IfKind::IPv6)?;

        Ok(Self {
            daemon,
            service_info,
            endpoint_id,
            service_port,
            ble_receiver,
            visibility_sender,
            visibility_receiver,
            device_name_receiver,
        })
    }

    pub async fn run(&mut self, ctk: CancellationToken) -> Result<(), anyhow::Error> {
        info!("{INNER_NAME}: service starting");
        let monitor = self.daemon.monitor()?;
        let ble_receiver = &mut self.ble_receiver;
        let mut visibility = *self.visibility_receiver.borrow();
        let mut interval = interval_at(Instant::now() + TICK_INTERVAL, TICK_INTERVAL);

        loop {
            tokio::select! {
                _ = ctk.cancelled() => {
                    info!("{INNER_NAME}: tracker cancelled, breaking");
                    break;
                }
                r = monitor.recv_async() => {
                    match r {
                        Ok(_) => continue,
                        Err(err) => return Err(err.into()),
                    }
                },
                _ = self.visibility_receiver.changed() => {
                    visibility = *self.visibility_receiver.borrow_and_update();

                    debug!("{INNER_NAME}: visibility changed: {visibility:?}");
                    if visibility == Visibility::Visible {
                        self.daemon.register(self.service_info.clone())?;
                    } else if visibility == Visibility::Invisible {
                        let receiver = self.daemon.unregister(self.service_info.get_fullname())?;
                        let _ = receiver.recv_async().await;
                    } else if visibility == Visibility::Temporarily {
                        self.daemon.register(self.service_info.clone())?;
                        interval.reset();
                    }
                }
                _ = self.device_name_receiver.changed() => {
                    let device_name = self.device_name_receiver.borrow_and_update().clone();
                    debug!("{INNER_NAME}: device name changed: {device_name}");

                    // Unregister the old service if currently visible
                    if visibility != Visibility::Invisible {
                        let receiver = self.daemon.unregister(self.service_info.get_fullname())?;
                        let _ = receiver.recv_async().await;
                    }

                    // Rebuild service with new device name
                    self.service_info = Self::build_service(
                        self.endpoint_id,
                        self.service_port,
                        DeviceType::Laptop,
                        &device_name,
                    )?;

                    // Re-register if currently visible
                    if visibility != Visibility::Invisible {
                        self.daemon.register(self.service_info.clone())?;
                    }
                }
                _ = ble_receiver.recv() => {
                    if visibility == Visibility::Invisible {
                        continue;
                    }

                    debug!("{INNER_NAME}: ble_receiver: got event");
                    if visibility == Visibility::Visible || visibility == Visibility::Temporarily {
                        // Android can sometime not see the mDNS service if the service
                        // was running BEFORE Android started the Discovery phase for QuickShare.
                        // So resend a broadcast if there's a android device sending.
                        self.daemon.register(self.service_info.clone())?;
                    } else {
                        self.daemon.register(self.service_info.clone())?;
                    }
                },
                _ = interval.tick() => {
                    if visibility != Visibility::Temporarily {
                        continue;
                    }

                    let receiver = self.daemon.unregister(self.service_info.get_fullname())?;
                    let _ = receiver.recv_async().await;
                    let _ = self.visibility_sender.lock().unwrap().send(Visibility::Invisible);
                }
            }
        }

        // Unregister the mDNS service - we're shutting down
        let receiver = self.daemon.unregister(self.service_info.get_fullname())?;
        if let Ok(event) = receiver.recv_async().await {
            info!("MDnsServer: service unregistered: {:?}", event);
        }

        Ok(())
    }

    fn build_service(
        endpoint_id: [u8; 4],
        service_port: u16,
        device_type: DeviceType,
        device_name: &str,
    ) -> Result<ServiceInfo, anyhow::Error> {
        let name = gen_mdns_name(endpoint_id);
        info!("Broadcasting with: {device_name}");
        let endpoint_info = URL_SAFE_NO_PAD.encode(encode_endpoint_info(device_type, device_name));

        let properties = [("n", endpoint_info)];
        let si = ServiceInfo::new(
            "_FC9F5ED42C8A._tcp.local.",
            &name,
            &mdns_host_name(device_name),
            "",
            service_port,
            &properties[..],
        )?
        .enable_addr_auto();

        Ok(si)
    }
}

/// mDNS host names must be ASCII labels; the display name travels separately in
/// the endpoint info, so any safe label works here.
fn mdns_host_name(hostname: &str) -> String {
    let label: String = hostname
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let label = label.trim_matches('-');

    if label.is_empty() {
        "rquickshare.local.".to_string()
    } else {
        format!("{label}.local.")
    }
}
