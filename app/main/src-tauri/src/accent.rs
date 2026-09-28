//! The desktop's accent color, read from the XDG settings portal (GNOME 47+,
//! KDE Plasma, ...) and sent to the frontend as `#rrggbb` when it changes.

use futures_util::StreamExt;
use tauri::{AppHandle, Emitter};
use zbus::zvariant::{OwnedValue, Value};

const NAMESPACE: &str = "org.freedesktop.appearance";
const KEY: &str = "accent-color";

#[zbus::proxy(
    interface = "org.freedesktop.portal.Settings",
    default_service = "org.freedesktop.portal.Desktop",
    default_path = "/org/freedesktop/portal/desktop"
)]
trait PortalSettings {
    fn read_one(&self, namespace: &str, key: &str) -> zbus::Result<OwnedValue>;

    #[zbus(signal)]
    fn setting_changed(&self, namespace: &str, key: &str, value: Value<'_>) -> zbus::Result<()>;
}

/// The accent as `#rrggbb`, or `None` when the desktop has none.
pub async fn read() -> Option<String> {
    let connection = zbus::Connection::session().await.ok()?;
    let portal = PortalSettingsProxy::new(&connection).await.ok()?;
    let value = portal.read_one(NAMESPACE, KEY).await.ok()?;
    to_hex(&value)
}

/// Emits `system_accent_color` whenever the desktop's accent changes.
pub async fn watch(app: AppHandle) -> Result<(), zbus::Error> {
    let connection = zbus::Connection::session().await?;
    let portal = PortalSettingsProxy::new(&connection).await?;
    let mut changes = portal.receive_setting_changed().await?;

    while let Some(signal) = changes.next().await {
        let Ok(args) = signal.args() else { continue };
        if args.namespace == NAMESPACE && args.key == KEY {
            let _ = app.emit("system_accent_color", to_hex(&args.value));
        }
    }
    Ok(())
}

/// The portal's `(ddd)` sRGB triple; components outside 0..=1 mean unset.
fn to_hex(value: &Value<'_>) -> Option<String> {
    let value = match value {
        Value::Value(inner) => inner,
        other => other,
    };
    let (r, g, b) = <(f64, f64, f64)>::try_from(value.try_clone().ok()?).ok()?;
    let channel = |c: f64| (0.0..=1.0).contains(&c).then(|| (c * 255.0).round() as u8);
    Some(format!(
        "#{:02x}{:02x}{:02x}",
        channel(r)?,
        channel(g)?,
        channel(b)?
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_portal_colors() {
        let slate = Value::from((0.435294, 0.513726, 0.588235));
        assert_eq!(to_hex(&slate).as_deref(), Some("#6f8396"));
        assert_eq!(to_hex(&Value::from((-1.0, -1.0, -1.0))), None);
    }
}
