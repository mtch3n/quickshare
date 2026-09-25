//! Wi-Fi network connection via NetworkManager D-Bus.

use rqs_lib::{WifiNetwork, WifiSecurity};
use std::collections::HashMap;
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

#[zbus::proxy(
    interface = "org.freedesktop.NetworkManager",
    default_service = "org.freedesktop.NetworkManager",
    default_path = "/org/freedesktop/NetworkManager"
)]
trait NetworkManager {
    fn get_devices(&self) -> zbus::Result<Vec<OwnedObjectPath>>;

    fn add_and_activate_connection(
        &self,
        connection: HashMap<&str, HashMap<&str, OwnedValue>>,
        device: OwnedObjectPath,
        specific_object: OwnedObjectPath,
    ) -> zbus::Result<(OwnedObjectPath, OwnedObjectPath)>;
}

#[zbus::proxy(
    interface = "org.freedesktop.NetworkManager.Device",
    default_service = "org.freedesktop.NetworkManager"
)]
trait Device {
    #[zbus(property)]
    fn device_type(&self) -> zbus::Result<u32>;
}

/// Connect to a Wi-Fi network using NetworkManager.
pub async fn connect_wifi(network: WifiNetwork) -> Result<(), String> {
    let connection = zbus::Connection::system()
        .await
        .map_err(|e| format!("Failed to connect to system bus: {e}"))?;

    let nm = NetworkManagerProxy::new(&connection)
        .await
        .map_err(|e| format!("NetworkManager not running: {e}"))?;

    let devices = nm
        .get_devices()
        .await
        .map_err(|e| format!("Failed to get devices: {e}"))?;

    let wifi_device = find_wifi_device(&connection, devices)
        .await
        .map_err(|e| format!("Failed to find Wi-Fi device: {e}"))?
        .ok_or("No Wi-Fi device found")?;

    let ssid = network.ssid.clone();
    let password = network.password.clone();

    let mut settings: HashMap<&str, HashMap<&str, OwnedValue>> = HashMap::new();

    let mut conn = HashMap::new();
    conn.insert(
        "type",
        OwnedValue::try_from(Value::from("802-11-wireless")).map_err(|e| format!("{e}"))?,
    );
    conn.insert(
        "id",
        OwnedValue::try_from(Value::from(ssid.clone())).map_err(|e| format!("{e}"))?,
    );
    settings.insert("connection", conn);

    let mut wireless = HashMap::new();
    wireless.insert(
        "ssid",
        OwnedValue::try_from(Value::from(ssid.as_bytes().to_vec())).map_err(|e| format!("{e}"))?,
    );
    wireless.insert(
        "mode",
        OwnedValue::try_from(Value::from("infrastructure")).map_err(|e| format!("{e}"))?,
    );
    wireless.insert(
        "hidden",
        OwnedValue::try_from(Value::from(network.hidden)).map_err(|e| format!("{e}"))?,
    );
    settings.insert("802-11-wireless", wireless);

    if !password.is_empty() {
        let mut security = HashMap::new();

        match network.security {
            WifiSecurity::Open => {
                // No security section needed for open networks
            }
            WifiSecurity::WpaPsk => {
                security.insert(
                    "key-mgmt",
                    OwnedValue::try_from(Value::from("wpa-psk")).map_err(|e| format!("{e}"))?,
                );
                security.insert(
                    "psk",
                    OwnedValue::try_from(Value::from(password.clone()))
                        .map_err(|e| format!("{e}"))?,
                );
                settings.insert("802-11-wireless-security", security);
            }
            WifiSecurity::Sae => {
                security.insert(
                    "key-mgmt",
                    OwnedValue::try_from(Value::from("sae")).map_err(|e| format!("{e}"))?,
                );
                security.insert(
                    "psk",
                    OwnedValue::try_from(Value::from(password.clone()))
                        .map_err(|e| format!("{e}"))?,
                );
                settings.insert("802-11-wireless-security", security);
            }
            WifiSecurity::Wep => {
                security.insert(
                    "key-mgmt",
                    OwnedValue::try_from(Value::from("none")).map_err(|e| format!("{e}"))?,
                );
                security.insert(
                    "wep-key0",
                    OwnedValue::try_from(Value::from(password.clone()))
                        .map_err(|e| format!("{e}"))?,
                );
                security.insert(
                    "wep-key-type",
                    OwnedValue::try_from(Value::from(1i32)).map_err(|e| format!("{e}"))?,
                );
                settings.insert("802-11-wireless-security", security);
            }
        }
    }

    let root_path = OwnedObjectPath::try_from("/".to_string())
        .map_err(|e| format!("Invalid root path: {e}"))?;

    nm.add_and_activate_connection(settings, wifi_device, root_path)
        .await
        .map_err(|e| format!("Failed to connect to network: {e}"))?;

    Ok(())
}

async fn find_wifi_device(
    connection: &zbus::Connection,
    devices: Vec<OwnedObjectPath>,
) -> Result<Option<OwnedObjectPath>, String> {
    for device_path in devices {
        let device_proxy = DeviceProxy::builder(connection)
            .path(device_path.clone())
            .map_err(|e| format!("Failed to build proxy: {e}"))?
            .build()
            .await;

        if let Ok(proxy) = device_proxy
            && let Ok(dtype) = proxy.device_type().await
            && dtype == 2
        {
            return Ok(Some(device_path));
        }
    }

    Ok(None)
}
