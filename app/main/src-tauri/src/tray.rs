//! Tray icon through the StatusNotifierItem D-Bus protocol (KDE, and GNOME with
//! the AppIndicator extension). Unlike libappindicator it supports a symbolic,
//! panel-coloured icon and opening the window with a click.

use std::collections::HashSet;

use ksni::menu::{CheckmarkItem, StandardItem};
use ksni::{Icon, MenuItem, Status, ToolTip, TrayMethods};
use rqs_lib::Visibility;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_clipboard_manager::ClipboardExt;

use crate::{AppState, kill_app, open_main_window};

const ICON_NAME: &str = "rquickshare-symbolic";
const ATTENTION_ICON_NAME: &str = "rquickshare-attention-symbolic";

pub struct Tray {
    app: AppHandle,
    visibility: Visibility,
    /// Transfers waiting for the user to accept or decline.
    waiting: HashSet<String>,
}

/// `None` when no StatusNotifierItem host is running.
pub struct TrayHandle(Option<ksni::Handle<Tray>>);

pub async fn spawn(app: &AppHandle, visibility: Visibility) -> TrayHandle {
    let tray = Tray {
        app: app.clone(),
        visibility,
        waiting: HashSet::new(),
    };

    match tray.spawn().await {
        Ok(handle) => TrayHandle(Some(handle)),
        Err(e) => {
            warn!("Tray unavailable: {e}");
            TrayHandle(None)
        }
    }
}

impl TrayHandle {
    pub async fn set_visibility(&self, visibility: Visibility) {
        if let Some(handle) = &self.0 {
            handle.update(|t| t.visibility = visibility).await;
        }
    }

    pub async fn set_waiting(&self, id: String, waiting: bool) {
        if let Some(handle) = &self.0 {
            handle
                .update(|t| {
                    if waiting {
                        t.waiting.insert(id);
                    } else {
                        t.waiting.remove(&id);
                    }
                })
                .await;
        }
    }
}

impl ksni::Tray for Tray {
    fn id(&self) -> String {
        "rquickshare".into()
    }

    fn title(&self) -> String {
        "QuickShare".into()
    }

    fn icon_theme_path(&self) -> String {
        // AppImages carry their icons with them instead of installing them.
        std::env::var("APPDIR")
            .map(|dir| format!("{dir}/usr/share/icons"))
            .unwrap_or_default()
    }

    fn icon_name(&self) -> String {
        ICON_NAME.into()
    }

    fn icon_pixmap(&self) -> Vec<Icon> {
        vec![pixmap()]
    }

    fn attention_icon_name(&self) -> String {
        ATTENTION_ICON_NAME.into()
    }

    fn status(&self) -> Status {
        if self.waiting.is_empty() {
            Status::Active
        } else {
            Status::NeedsAttention
        }
    }

    fn tool_tip(&self) -> ToolTip {
        let description = match (self.waiting.len(), self.visibility) {
            (0, Visibility::Visible) => "Visible to nearby devices".to_string(),
            (0, Visibility::Temporarily) => "Temporarily visible".to_string(),
            (0, Visibility::Invisible) => "Hidden".to_string(),
            (1, _) => "A device wants to share with you".to_string(),
            (n, _) => format!("{n} devices want to share with you"),
        };

        ToolTip {
            title: "QuickShare".into(),
            description,
            ..Default::default()
        }
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        open_main_window(&self.app);
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        vec![
            StandardItem {
                label: "Open".into(),
                activate: Box::new(|t: &mut Self| open_main_window(&t.app)),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "Send files…".into(),
                activate: Box::new(|t: &mut Self| {
                    open_main_window(&t.app);
                    let _ = t.app.emit("pick_files", ());
                }),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "Send clipboard".into(),
                activate: Box::new(|t: &mut Self| {
                    if let Ok(text) = t.app.clipboard().read_text()
                        && !text.is_empty()
                    {
                        open_main_window(&t.app);
                        let _ = t.app.emit("send_text", text);
                    }
                }),
                ..Default::default()
            }
            .into(),
            CheckmarkItem {
                label: "Visible to everyone".into(),
                checked: self.visibility != Visibility::Invisible,
                activate: Box::new(|t: &mut Self| {
                    let next = if t.visibility == Visibility::Invisible {
                        Visibility::Visible
                    } else {
                        Visibility::Invisible
                    };
                    t.app
                        .state::<AppState>()
                        .rqs
                        .lock()
                        .unwrap()
                        .change_visibility(next);
                }),
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            StandardItem {
                label: "Quit".into(),
                icon_name: "application-exit".into(),
                activate: Box::new(|t: &mut Self| kill_app(&t.app)),
                ..Default::default()
            }
            .into(),
        ]
    }
}

/// Fallback for hosts that can't find our icon in the theme.
fn pixmap() -> Icon {
    let image = tauri::image::Image::from_bytes(include_bytes!("../icons/64x64.png"))
        .expect("bundled tray icon is a valid PNG");

    // RGBA -> ARGB32 in network byte order.
    let data = image
        .rgba()
        .chunks_exact(4)
        .flat_map(|p| [p[3], p[0], p[1], p[2]])
        .collect();

    Icon {
        width: image.width() as i32,
        height: image.height() as i32,
        data,
    }
}
