//! "Send with Quick Share" entries for the Nautilus, Dolphin and Nemo context
//! menus, installed per user so they point at however we were launched
//! (package, AppImage, ...).

use std::path::PathBuf;

use tauri::{AppHandle, Manager};

struct Integration {
    /// Relative to the XDG data dir (~/.local/share).
    path: &'static str,
    template: &'static str,
    quote: fn(&str) -> String,
}

const INTEGRATIONS: [Integration; 3] = [
    Integration {
        path: "nautilus-python/extensions/rquickshare_nautilus.py",
        template: include_str!("../linux/rquickshare_nautilus.py"),
        quote: quote_python,
    },
    Integration {
        path: "kio/servicemenus/rquickshare-send.desktop",
        template: include_str!("../linux/rquickshare-send.desktop"),
        quote: quote_desktop,
    },
    Integration {
        path: "nemo/actions/rquickshare.nemo_action",
        template: include_str!("../linux/rquickshare.nemo_action"),
        quote: quote_desktop,
    },
];

pub fn installed(app: &AppHandle) -> bool {
    data_dir(app).is_some_and(|dir| INTEGRATIONS.iter().all(|i| dir.join(i.path).exists()))
}

pub fn install(app: &AppHandle) -> Result<(), anyhow::Error> {
    let dir = data_dir(app).ok_or_else(|| anyhow::anyhow!("no data directory"))?;
    let exec = exec_path()?;

    for i in &INTEGRATIONS {
        let path = dir.join(i.path);
        std::fs::create_dir_all(path.parent().unwrap())?;
        std::fs::write(&path, i.template.replace("@EXEC@", &(i.quote)(&exec)))?;
        // Plasma 6 only runs service menus that are executable.
        set_executable(&path)?;
    }

    Ok(())
}

pub fn uninstall(app: &AppHandle) -> Result<(), anyhow::Error> {
    let Some(dir) = data_dir(app) else {
        return Ok(());
    };

    for i in &INTEGRATIONS {
        match std::fs::remove_file(dir.join(i.path)) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e.into()),
            _ => {}
        }
    }

    Ok(())
}

fn data_dir(app: &AppHandle) -> Option<PathBuf> {
    app.path().data_dir().ok()
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

fn set_executable(path: &std::path::Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
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
}
