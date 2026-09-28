use notify_rust::{Hint, Notification, Urgency};
use rqs_lib::Visibility;
use rqs_lib::channel::ChannelAction;
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

/// Shows `notification` and hands its clicked action to `on_action`.
///
/// notify-rust talks to D-Bus through zbus' blocking API, which panics when it
/// runs on a tokio worker thread, so all of it happens on a blocking thread.
fn show(notification: Notification, on_action: impl FnOnce(&str) + Send + 'static) {
    tokio::task::spawn_blocking(move || match notification.show() {
        Ok(handle) => handle.wait_for_action(on_action),
        Err(e) => error!("Couldn't show notification: {e}"),
    });
}

/// Describes what a transfer carries, e.g. "photo.jpg", "4 files" or "a link".
fn content_summary(files: Option<&[String]>, text_type: Option<&str>) -> String {
    match files {
        Some([file]) => file.clone(),
        Some(files) if !files.is_empty() => format!("{} files", files.len()),
        _ => match text_type {
            Some("Url") => "a link".to_string(),
            Some("Text") => "some text".to_string(),
            Some("Wifi") => "Wi-Fi info".to_string(),
            _ => "content".to_string(),
        },
    }
}

pub fn send_request_notification(
    name: String,
    pin_code: Option<String>,
    files: Option<Vec<String>>,
    text_type: Option<String>,
    id: String,
    app_handle: &AppHandle,
) {
    let content = content_summary(files.as_deref(), text_type.as_deref());
    let body = match pin_code {
        Some(pin) => format!("Wants to send {content}\nPIN: {pin}"),
        None => format!("Wants to send {content}"),
    };

    let mut notification = base();
    notification
        .summary(&format!("Incoming from {name}"))
        .body(&body)
        .urgency(Urgency::Critical)
        .action("default", "Open")
        .action("accept", "Accept")
        .action("reject", "Decline");

    let app_handle = app_handle.clone();
    show(notification, move |action| {
        let action = match action {
            "accept" => ChannelAction::AcceptTransfer,
            "reject" => ChannelAction::RejectTransfer,
            "default" => return open_main_window(&app_handle),
            _ => return,
        };

        commands::send_action(&app_handle.state::<AppState>(), id, action);
    });
}

pub fn send_temporarily_notification(app_handle: &AppHandle) {
    let mut notification = base();
    notification
        .body("A nearby device is sharing, but you're hidden")
        .action("visible", "Be visible for 1 minute")
        .action("ignore", "Ignore")
        .id(1919);

    let app_handle = app_handle.clone();
    show(notification, move |action| {
        if action == "visible" {
            commands::set_visibility(Visibility::Temporarily, app_handle.state());
        }
    });
}

pub fn send_received_notification(
    name: String,
    files: Option<Vec<String>>,
    destination: Option<String>,
    text_type: Option<String>,
    text_payload: Option<String>,
    app_handle: &AppHandle,
) {
    let body = content_summary(files.as_deref(), text_type.as_deref());

    let has_url = text_type.as_deref() == Some("Url");
    let has_text = text_type.as_deref() == Some("Text");

    let mut notification = base();
    notification
        .summary(&format!("Received from {name}"))
        .body(&body)
        .action("default", "Open folder");
    if has_url {
        notification.action("open", "Open");
    } else if has_text {
        notification.action("copy", "Copy");
    }

    let app_handle = app_handle.clone();
    show(notification, move |action| match action {
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
            if let Some(url) = &text_payload
                && crate::is_web_url(url)
            {
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
}
