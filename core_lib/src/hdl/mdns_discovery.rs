use std::collections::HashMap;
use std::net::Ipv4Addr;
use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use mdns_sd::{ResolvedService, ServiceDaemon, ServiceEvent};
use serde::{Deserialize, Serialize};
use tokio::net::TcpStream;
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;
use ts_rs::TS;

use crate::DeviceType;
use crate::utils::{is_not_self_ip, parse_endpoint_info};

const SERVICE_TYPE: &str = "_FC9F5ED42C8A._tcp.local.";
const PROBE_TIMEOUT: Duration = Duration::from_secs(3);

/// The protocol a nearby device is reached over.
#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize, TS)]
#[ts(export)]
pub enum Protocol {
    #[default]
    QuickShare,
    LocalSend {
        https: bool,
    },
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, TS)]
#[ts(export)]
pub struct EndpointInfo {
    pub id: String,
    pub name: Option<String>,
    pub ip: Option<String>,
    pub port: Option<String>,
    pub rtype: Option<DeviceType>,
    pub present: Option<bool>,
    pub protocol: Protocol,
}

pub struct MDnsDiscovery {
    daemon: ServiceDaemon,
    sender: broadcast::Sender<EndpointInfo>,
}

impl MDnsDiscovery {
    pub fn new(sender: broadcast::Sender<EndpointInfo>) -> Result<Self, anyhow::Error> {
        let daemon = ServiceDaemon::new()?;

        Ok(Self { daemon, sender })
    }

    pub async fn run(self, ctk: CancellationToken) -> Result<(), anyhow::Error> {
        info!("MDnsDiscovery: service starting");

        let receiver = self.daemon.browse(SERVICE_TYPE)?;
        // fullname -> endpoint id
        let mut known: HashMap<String, String> = HashMap::new();

        loop {
            tokio::select! {
                _ = ctk.cancelled() => {
                    info!("MDnsDiscovery: tracker cancelled, breaking");
                    break;
                }
                r = receiver.recv_async() => match r {
                    Ok(ServiceEvent::ServiceResolved(info)) => {
                        if let Some(ei) = endpoint_from_service(&info) {
                            known.insert(info.get_fullname().to_string(), ei.id.clone());
                            // Android can keep announcing a service it no longer
                            // serves, so only show endpoints that accept connections.
                            let sender = self.sender.clone();
                            tokio::spawn(async move {
                                let reachable = tokio::time::timeout(PROBE_TIMEOUT, TcpStream::connect(&ei.id)).await;
                                if matches!(reachable, Ok(Ok(_))) {
                                    info!("MDnsDiscovery: resolved {:?}", ei);
                                    let _ = sender.send(ei);
                                }
                            });
                        }
                    }
                    Ok(ServiceEvent::ServiceRemoved(_, fullname)) => {
                        if let Some(id) = known.remove(&fullname) {
                            info!("MDnsDiscovery: removed {fullname}");
                            let _ = self.sender.send(EndpointInfo {
                                id,
                                ..Default::default()
                            });
                        }
                    }
                    Ok(_) => {}
                    Err(e) => {
                        error!("MDnsDiscovery: {e}");
                        break;
                    }
                }
            }
        }

        let _ = self.daemon.stop_browse(SERVICE_TYPE);
        let _ = self.daemon.shutdown();
        Ok(())
    }
}

fn endpoint_from_service(info: &ResolvedService) -> Option<EndpointInfo> {
    let ip: Ipv4Addr = info.get_addresses_v4().into_iter().find(is_not_self_ip)?;
    let port = info.get_port();

    let raw = URL_SAFE_NO_PAD
        .decode(info.get_property_val_str("n")?)
        .ok()?;
    let (device_type, name) = parse_endpoint_info(&raw).ok()?;
    // Devices that hide their name still announce a host name.
    let name = name.unwrap_or_else(|| {
        info.get_hostname()
            .trim_end_matches('.')
            .trim_end_matches(".local")
            .to_string()
    });

    Some(EndpointInfo {
        id: format!("{ip}:{port}"),
        name: Some(name),
        ip: Some(ip.to_string()),
        port: Some(port.to_string()),
        rtype: Some(device_type),
        present: Some(true),
        protocol: Protocol::QuickShare,
    })
}
