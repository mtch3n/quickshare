use notify_rust::{Hint, Notification, Urgency};
use rqs_lib::Visibility;
use rqs_lib::channel::ChannelAction;
use rqs_lib::hdl::info::TransferMetadata;
use tauri::{AppHandle, Manager};
use tauri_plugin_clipboard_manager::ClipboardExt;
use tauri_plugin_opener::OpenerExt;

use crate::{AppState, commands, open_main_window};

fn base() -> Notification {
    let mut n = Notification::new();
    n.appname("RQuickShare")
        .summary("RQuickShare")
        .icon("rquickshare")
        .hint(Hint::DesktopEntry("rquickshare".into()));
    n
}

pub fn send_request_notification(name: String, id: String, app_handle: &AppHandle) {
    let shown = base()
        .body(&format!("{name} wants to share with you"))
        .urgency(Urgency::Critical)
        .action("default", "Open")
        .action("accept", "Accept")
        .action("reject", "Decline")
        .show();

    let n = match shown {
        Ok(n) => n,
        Err(e) => return error!("Couldn't show notification: {e}"),
    };

    let app_handle = app_handle.clone();
    // wait_for_action blocks until the notification is closed.
    tokio::task::spawn_blocking(move || {
        n.wait_for_action(|action| {
            let action = match action {
                "accept" => ChannelAction::AcceptTransfer,
                "reject" => ChannelAction::RejectTransfer,
                "default" => return open_main_window(&app_handle),
                _ => return,
            };

            commands::send_action(&app_handle.state::<AppState>(), id, action);
        });
    });
}

pub fn send_temporarily_notification(app_handle: &AppHandle) {
    let shown = base()
        .body("A nearby device is sharing, but you're hidden")
        .action("visible", "Be visible for 1 minute")
        .action("ignore", "Ignore")
        .id(1919)
        .show();

    let n = match shown {
        Ok(n) => n,
        Err(e) => return error!("Couldn't show notification: {e}"),
    };

    let app_handle = app_handle.clone();
    tokio::task::spawn_blocking(move || {
        n.wait_for_action(|action| {
            if action == "visible" {
                commands::set_visibility(Visibility::Temporarily, app_handle.state());
            }
        });
    });
}

pub fn send_received_notification(meta: TransferMetadata, app_handle: &AppHandle) {
    let name = meta
        .source
        .as_ref()
        .map(|source| source.name.clone())
        .unwrap_or_else(|| "A nearby device".to_string());

    let body = if let Some(files) = &meta.files {
        if files.is_empty() {
            if let Some(text_type) = &meta.text_type {
                match text_type {
                    rqs_lib::hdl::TextPayloadType::Url => "a link".to_string(),
                    rqs_lib::hdl::TextPayloadType::Text => "some text".to_string(),
                    rqs_lib::hdl::TextPayloadType::Wifi => "Wi-Fi info".to_string(),
                }
            } else {
                "content".to_string()
            }
        } else if files.len() == 1 {
            files[0].clone()
        } else {
            format!("{} files", files.len())
        }
    } else {
        "content".to_string()
    };

    let mut notification = base()
        .summary(&format!("Received from {}", name))
        .body(&body)
        .action("default", "Open folder");

    match &meta.text_type {
        Some(rqs_lib::hdl::TextPayloadType::Url) => {
            notification = notification.action("open", "Open");
        }
        Some(rqs_lib::hdl::TextPayloadType::Text) => {
            notification = notification.action("copy", "Copy");
        }
        _ => {}
    }

    let shown = notification.show();

    let n = match shown {
        Ok(n) => n,
        Err(e) => return error!("Couldn't show received notification: {e}"),
    };

    let app_handle = app_handle.clone();
    let destination = meta.destination.clone();
    let text_payload = meta.text_payload.clone();
    let text_type = meta.text_type.clone();

    tokio::task::spawn_blocking(move || {
        n.wait_for_action(move |action| match action {
            "default" => {
                if let Some(dest) = &destination {
                    let opener = app_handle.opener();
                    if let Err(e) = opener.open_path(dest, None::<&str>) {
                        error!("Couldn't open folder: {e}");
                    } else {
                        return;
                    }
                }
                open_main_window(&app_handle);
            }
            "open" => {
                if let Some(url) = &text_payload {
                    let opener = app_handle.opener();
                    if let Err(e) = opener.open_url(url, None::<&str>) {
                        error!("Couldn't open URL: {e}");
                    }
                }
            }
            "copy" => {
                if let Some(text) = &text_payload {
                    let clipboard = app_handle.clipboard();
                    if let Err(e) = clipboard.write_text(text.clone()) {
                        error!("Couldn't copy text: {e}");
                    }
                }
            }
            _ => {}
        });
    });
}
