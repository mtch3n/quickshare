//! Opens links and folders through the XDG desktop portal. The desktop starts
//! the user's default browser or file manager itself, so it doesn't inherit the
//! AppImage's environment (bundled libraries, GIO modules, xdg-open).

use std::collections::HashMap;

use anyhow::bail;
use zbus::zvariant::{Fd, OwnedObjectPath, Value};

#[zbus::proxy(
    interface = "org.freedesktop.portal.OpenURI",
    default_service = "org.freedesktop.portal.Desktop",
    default_path = "/org/freedesktop/portal/desktop"
)]
trait OpenUri {
    #[zbus(name = "OpenURI")]
    fn open_uri(
        &self,
        parent_window: &str,
        uri: &str,
        options: HashMap<&str, Value<'_>>,
    ) -> zbus::Result<OwnedObjectPath>;

    fn open_file(
        &self,
        parent_window: &str,
        fd: Fd<'_>,
        options: HashMap<&str, Value<'_>>,
    ) -> zbus::Result<OwnedObjectPath>;
}

async fn portal() -> Result<OpenUriProxy<'static>, zbus::Error> {
    let connection = zbus::Connection::session().await?;
    OpenUriProxy::new(&connection).await
}

/// Opens a web link in the default browser. Other schemes could launch
/// arbitrary handlers, so they are refused.
pub async fn url(url: &str) -> Result<(), anyhow::Error> {
    if !rqs_lib::is_web_url(url) {
        bail!("not a web link: {url}");
    }
    portal().await?.open_uri("", url, HashMap::new()).await?;
    Ok(())
}

/// Opens a file with its default app, or a folder in the file manager.
pub async fn path(path: &str) -> Result<(), anyhow::Error> {
    let file = std::fs::File::open(path)?;
    portal()
        .await?
        .open_file("", Fd::from(&file), HashMap::new())
        .await?;
    Ok(())
}
