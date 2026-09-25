use std::collections::HashSet;
use std::time::{Duration, Instant};

use bluer::monitor::{Monitor, MonitorEvent, MonitorHandle, Pattern, RssiSamplingPeriod};
use bluer::{AdapterEvent, Address, DiscoveryFilter, DiscoveryTransport, Uuid};
use futures::StreamExt;
use tokio::sync::broadcast::Sender;
use tokio_util::sync::CancellationToken;

const INNER_NAME: &str = "BleListener";

/// The Nearby Sharing service, advertised by Android devices that are sharing.
const SHARING_UUID: Uuid = Uuid::from_u128(0x0000fe2c_0000_1000_8000_00805f9b34fb);
/// AD type "Service Data - 16-bit UUID".
const AD_SERVICE_DATA_16: u8 = 0x16;
/// 0xFE2C, little-endian as it appears on air.
const SHARING_UUID_16: [u8; 2] = [0x2C, 0xFE];
/// Don't alert more than once per interval.
const ALERT_INTERVAL: Duration = Duration::from_secs(30);

pub struct BleListener {
    adapter: bluer::Adapter,
    sender: Sender<()>,
    last_alert: Option<Instant>,
}

impl BleListener {
    pub async fn new(sender: Sender<()>) -> Result<Self, anyhow::Error> {
        let session = bluer::Session::new().await?;
        let adapter = session.default_adapter().await?;

        Ok(Self {
            adapter,
            sender,
            last_alert: None,
        })
    }

    /// Watches for nearby Android devices advertising that they are sharing.
    ///
    /// Prefers BlueZ passive advertisement monitoring, which doesn't disturb
    /// other Bluetooth links (audio, keyboards, mice). BlueZ only offers it with
    /// `Experimental = true` in /etc/bluetooth/main.conf; otherwise this falls
    /// back to LE discovery filtered on the sharing service.
    pub async fn run(mut self, ctk: CancellationToken) -> Result<(), anyhow::Error> {
        info!("{INNER_NAME}: service starting on {}", self.adapter.name());

        match self.passive_monitor().await {
            Ok(monitor) => self.watch_monitor(monitor, ctk).await,
            Err(e) => {
                info!("{INNER_NAME}: passive monitoring unavailable ({e}), using discovery");
                self.watch_discovery(ctk).await
            }
        }
    }

    async fn passive_monitor(&self) -> bluer::Result<MonitorHandle> {
        self.adapter
            .monitor()
            .await?
            .register(Monitor {
                rssi_sampling_period: Some(RssiSamplingPeriod::First),
                patterns: Some(vec![Pattern::new(AD_SERVICE_DATA_16, 0, &SHARING_UUID_16)]),
                ..Default::default()
            })
            .await
    }

    async fn watch_monitor(
        &mut self,
        mut monitor: MonitorHandle,
        ctk: CancellationToken,
    ) -> Result<(), anyhow::Error> {
        loop {
            tokio::select! {
                _ = ctk.cancelled() => break,
                event = monitor.next() => match event {
                    Some(MonitorEvent::DeviceFound(id)) => self.alert(id.device),
                    Some(_) => {}
                    None => break,
                }
            }
        }

        Ok(())
    }

    async fn watch_discovery(&mut self, ctk: CancellationToken) -> Result<(), anyhow::Error> {
        self.adapter
            .set_discovery_filter(DiscoveryFilter {
                uuids: HashSet::from([SHARING_UUID]),
                transport: DiscoveryTransport::Le,
                ..Default::default()
            })
            .await?;
        let mut events = self.adapter.discover_devices_with_changes().await?;

        loop {
            tokio::select! {
                _ = ctk.cancelled() => break,
                event = events.next() => match event {
                    Some(AdapterEvent::DeviceAdded(address)) => {
                        if self.is_sharing(address).await {
                            self.alert(address);
                        }
                    }
                    Some(_) => {}
                    None => break,
                }
            }
        }

        Ok(())
    }

    /// Discovery also reports cached devices that are out of range, so check
    /// that the device is in range and currently advertising the service.
    async fn is_sharing(&self, address: Address) -> bool {
        let Ok(device) = self.adapter.device(address) else {
            return false;
        };

        let in_range = matches!(device.rssi().await, Ok(Some(_)));
        let sharing = matches!(
            device.service_data().await,
            Ok(Some(data)) if data.contains_key(&SHARING_UUID)
        );
        in_range && sharing
    }

    fn alert(&mut self, address: Address) {
        if self
            .last_alert
            .is_some_and(|t| t.elapsed() < ALERT_INTERVAL)
        {
            return;
        }

        debug!("{INNER_NAME}: {address} is sharing nearby");
        let _ = self.sender.send(());
        self.last_alert = Some(Instant::now());
    }
}
