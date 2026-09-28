//! Desktop integrations, installed per user so they point at however we were
//! launched (package, AppImage, ...): "Send with Quick Share" in the Nautilus,
//! Dolphin and Nemo context menus, a GNOME Shell Quick Settings tile, and the
//! native messaging host of the browser extension. They drive the app through
//! the CLI and its D-Bus service.

use std::path::{Path, PathBuf};

use tauri::{AppHandle, Manager};

pub const GNOME_EXTENSION_UUID: &str = "rquickshare@mandre.dev";
/// Fixed by the `key` in the extension's manifest.
const BROWSER_EXTENSION_ID: &str = "inbbifcfgodjmkpohmifohhkdaofllei";
const NATIVE_HOST: &str = "dev.mandre.rquickshare";

/// Config dirs of Chromium-based browsers, whose `NativeMessagingHosts` we
/// register with when the browser is installed.
const BROWSERS: [&str; 7] = [
    "google-chrome",
    "google-chrome-beta",
    "google-chrome-unstable",
    "chromium",
    "BraveSoftware/Brave-Browser",
    "microsoft-edge",
    "vivaldi",
];

enum Content {
    /// Text with `@EXEC@` replaced by our executable, quoted for the format.
    Template(&'static str, fn(&str) -> String),
    Static(&'static [u8]),
}

struct File {
    /// Relative to the XDG data dir (~/.local/share).
    path: &'static str,
    content: Content,
    executable: bool,
}

const fn template(path: &'static str, text: &'static str, quote: fn(&str) -> String) -> File {
    File {
        path,
        content: Content::Template(text, quote),
        executable: true,
    }
}

const fn asset(path: &'static str, bytes: &'static [u8]) -> File {
    File {
        path,
        content: Content::Static(bytes),
        executable: false,
    }
}

const FILES: [File; 6] = [
    template(
        "nautilus-python/extensions/rquickshare_nautilus.py",
        include_str!("../linux/rquickshare_nautilus.py"),
        quote_python,
    ),
    // Plasma 6 only runs service menus that are executable.
    template(
        "kio/servicemenus/rquickshare-send.desktop",
        include_str!("../linux/rquickshare-send.desktop"),
        quote_desktop,
    ),
    template(
        "nemo/actions/rquickshare.nemo_action",
        include_str!("../linux/rquickshare.nemo_action"),
        quote_desktop,
    ),
    asset(
        "gnome-shell/extensions/rquickshare@mandre.dev/metadata.json",
        include_bytes!("../linux/gnome-extension/metadata.json"),
    ),
    asset(
        "gnome-shell/extensions/rquickshare@mandre.dev/extension.js",
        include_bytes!("../linux/gnome-extension/extension.js"),
    ),
    asset(
        "gnome-shell/extensions/rquickshare@mandre.dev/rquickshare-symbolic.svg",
        include_bytes!("../linux/gnome-extension/rquickshare-symbolic.svg"),
    ),
];

/// Whether the user turned the integrations on. Any file counts, so a version
/// that adds integrations installs them over an older set at startup.
pub fn installed(app: &AppHandle) -> bool {
    data_dir(app).is_some_and(|dir| FILES.iter().any(|f| dir.join(f.path).exists()))
}

pub fn install(app: &AppHandle) -> Result<(), anyhow::Error> {
    let dir = data_dir(app).ok_or_else(|| anyhow::anyhow!("no data directory"))?;
    let exec = exec_path()?;

    for file in &FILES {
        let bytes = match &file.content {
            Content::Template(text, quote) => text.replace("@EXEC@", &quote(&exec)).into_bytes(),
            Content::Static(bytes) => bytes.to_vec(),
        };
        write(&dir.join(file.path), &bytes, file.executable)?;
    }

    let host = native_host_manifest(&exec);
    for manifest in native_host_manifests(app) {
        write(&manifest, host.as_bytes(), false)?;
    }

    enable_gnome_extension();
    Ok(())
}

pub fn uninstall(app: &AppHandle) -> Result<(), anyhow::Error> {
    let Some(dir) = data_dir(app) else {
        return Ok(());
    };

    let paths = FILES
        .iter()
        .map(|f| dir.join(f.path))
        .chain(native_host_manifests(app));
    for path in paths {
        match std::fs::remove_file(path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e.into()),
            _ => {}
        }
    }
    Ok(())
}

fn write(path: &Path, bytes: &[u8], executable: bool) -> Result<(), anyhow::Error> {
    use std::os::unix::fs::PermissionsExt;

    std::fs::create_dir_all(path.parent().unwrap())?;
    std::fs::write(path, bytes)?;
    let mode = if executable { 0o755 } else { 0o644 };
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))?;
    Ok(())
}

fn data_dir(app: &AppHandle) -> Option<PathBuf> {
    app.path().data_dir().ok()
}

/// Our native host manifest in every installed browser's config.
fn native_host_manifests(app: &AppHandle) -> Vec<PathBuf> {
    let Ok(config) = app.path().config_dir() else {
        return vec![];
    };
    BROWSERS
        .iter()
        .map(|browser| config.join(browser))
        .filter(|dir| dir.is_dir())
        .map(|dir| {
            dir.join("NativeMessagingHosts")
                .join(format!("{NATIVE_HOST}.json"))
        })
        .collect()
}

fn native_host_manifest(exec: &str) -> String {
    serde_json::to_string_pretty(&serde_json::json!({
        "name": NATIVE_HOST,
        "description": "QuickShare",
        "path": exec,
        "type": "stdio",
        "allowed_origins": [format!("chrome-extension://{BROWSER_EXTENSION_ID}/")],
    }))
    .unwrap()
}

/// Turns the Quick Settings tile on. GNOME Shell on Wayland only picks up a
/// newly installed extension at the next login, which the setting survives.
fn enable_gnome_extension() {
    let enabled = std::process::Command::new("gnome-extensions")
        .args(["enable", GNOME_EXTENSION_UUID])
        .output()
        .is_ok_and(|out| out.status.success());
    if enabled {
        return;
    }

    let Ok(out) = std::process::Command::new("gsettings")
        .args(["get", "org.gnome.shell", "enabled-extensions"])
        .output()
    else {
        return; // Not GNOME.
    };
    let current = String::from_utf8_lossy(&out.stdout);
    if !out.status.success() || current.contains(GNOME_EXTENSION_UUID) {
        return;
    }
    let list = match current.trim().trim_start_matches("@as").trim() {
        "[]" => format!("['{GNOME_EXTENSION_UUID}']"),
        list => format!("{}, '{GNOME_EXTENSION_UUID}']", list.trim_end_matches(']')),
    };
    if let Err(e) = std::process::Command::new("gsettings")
        .args(["set", "org.gnome.shell", "enabled-extensions", &list])
        .status()
    {
        warn!("Couldn't enable the GNOME Shell extension: {e}");
    }
}

/// The command a file manager should run: the AppImage itself when we run from
/// one (the binary lives in a temporary mount), otherwise our executable.
fn exec_path() -> Result<String, anyhow::Error> {
    let path = match std::env::var_os("APPIMAGE") {
        Some(appimage) => PathBuf::from(appimage),
        None => std::env::current_exe()?,
    };

    Ok(path.to_string_lossy().into_owned())
}

/// Contents of a Python double-quoted string literal.
fn quote_python(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

/// A desktop entry `Exec` argument, see the Desktop Entry Specification.
fn quote_desktop(s: &str) -> String {
    if !s.contains(|c: char| c.is_whitespace() || "\"'\\><~|&;$*?#()`".contains(c)) {
        return s.to_string();
    }

    let escaped: String = s
        .chars()
        .flat_map(|c| match c {
            '"' | '`' | '$' | '\\' => vec!['\\', c],
            _ => vec![c],
        })
        .collect();
    format!("\"{escaped}\"")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desktop_quoting() {
        assert_eq!(
            quote_desktop("/usr/bin/rquickshare"),
            "/usr/bin/rquickshare"
        );
        assert_eq!(
            quote_desktop("/home/me/My Apps/rqs.AppImage"),
            "\"/home/me/My Apps/rqs.AppImage\""
        );
        assert_eq!(quote_desktop("/a/$b"), "\"/a/\\$b\"");
    }

    #[test]
    fn python_quoting() {
        assert_eq!(quote_python(r#"/a/"b"\c"#), r#"/a/\"b\"\\c"#);
    }

    #[test]
    fn native_host_allows_our_extension() {
        let manifest: serde_json::Value =
            serde_json::from_str(&native_host_manifest("/opt/rqs \"x\".AppImage")).unwrap();
        assert_eq!(manifest["path"], "/opt/rqs \"x\".AppImage");
        assert_eq!(
            manifest["allowed_origins"][0],
            "chrome-extension://inbbifcfgodjmkpohmifohhkdaofllei/"
        );
    }
}
