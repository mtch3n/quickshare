use notify_rust::{Hint, Notification, Urgency};
use rqs_lib::Visibility;
use rqs_lib::channel::ChannelAction;
use tauri::{AppHandle, Manager};

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
