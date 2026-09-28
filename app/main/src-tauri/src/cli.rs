//! Command-line control of the running app, and the browser extension's native
//! messaging host. Both talk to it over D-Bus (see `dbus.rs`), starting it in
//! the tray first when it isn't running.

use std::io::{Read, Write};
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, anyhow, bail};
use clap::{Parser, Subcommand, ValueEnum};
use futures_util::StreamExt;

use crate::dbus::{BUS_NAME, OBJECT_PATH, is_terminal};

#[zbus::proxy(
    interface = "dev.mandre.RQuickShare1",
    default_service = "dev.mandre.RQuickShare",
    default_path = "/dev/mandre/RQuickShare"
)]
trait RQuickShare {
    fn show(&self) -> zbus::Result<()>;
    fn pick_device_for_files(&self, paths: Vec<String>) -> zbus::Result<()>;
    fn pick_device_for_text(&self, text: String) -> zbus::Result<()>;
    fn devices(&self, seconds: u32) -> zbus::Result<Vec<(String, String, String)>>;
    fn send_files(&self, device: String, paths: Vec<String>) -> zbus::Result<String>;
    fn send_text(&self, device: String, text: String) -> zbus::Result<String>;
    fn cancel(&self, id: String) -> zbus::Result<()>;
    #[zbus(property)]
    fn visibility(&self) -> zbus::Result<String>;
    #[zbus(property)]
    fn set_visibility(&self, visibility: String) -> zbus::Result<()>;
    #[zbus(property)]
    fn device_name(&self) -> zbus::Result<String>;
    #[zbus(signal)]
    fn transfer_changed(
        &self,
        id: String,
        direction: String,
        state: String,
        peer: String,
        done: u64,
        total: u64,
    ) -> zbus::Result<()>;
}

#[derive(Parser)]
#[command(
    name = "rquickshare",
    version,
    about = "Quick Share and LocalSend for Linux"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Send files, folders or a text. Without --to, opens the app to pick a device.
    Send {
        /// Files or folders to send.
        #[arg(required_unless_present = "text", conflicts_with = "text")]
        paths: Vec<PathBuf>,
        /// A text or link to send instead of files.
        #[arg(long)]
        text: Option<String>,
        /// Name of the nearby device to send to, or the start of it.
        #[arg(long)]
        to: Option<String>,
    },
    /// List nearby devices.
    Devices {
        /// Seconds to look for devices.
        #[arg(long, default_value_t = 5)]
        wait: u32,
    },
    /// Show or change who can see this device.
    Visibility { value: Option<VisibilityArg> },
    /// Show the app window.
    Show,
}

#[derive(Clone, Copy, ValueEnum)]
enum VisibilityArg {
    Visible,
    Hidden,
    /// Visible to everyone for a minute.
    Temporary,
}

/// Whether `args` are a CLI or native messaging invocation rather than a
/// launch of the app itself.
pub fn is_client(args: &[String]) -> bool {
    let first = args.get(1).map(String::as_str);
    matches!(
        first,
        Some(
            "send"
                | "devices"
                | "visibility"
                | "show"
                | "help"
                | "--help"
                | "-h"
                | "--version"
                | "-V"
        )
    ) || first.is_some_and(|a| a.starts_with("chrome-extension://"))
}

/// Runs a client invocation and returns the process exit code.
pub fn run(args: Vec<String>) -> i32 {
    let native = args
        .get(1)
        .is_some_and(|a| a.starts_with("chrome-extension://"));
    // Exits on its own for --help, --version and usage errors.
    let cli = (!native).then(|| Cli::parse_from(&args));

    let result = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(anyhow::Error::from)
        .and_then(|runtime| match cli {
            Some(cli) => runtime.block_on(command(cli.command)),
            None => runtime.block_on(native_host()),
        });
    match result {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("rquickshare: {e:#}");
            1
        }
    }
}

async fn command(command: Command) -> Result<(), anyhow::Error> {
    let connection = zbus::Connection::session().await?;
    let app = connect(&connection).await?;

    match command {
        Command::Send { paths, text, to } => {
            let paths = paths
                .iter()
                .map(|p| {
                    std::path::absolute(p)
                        .map(|p| p.to_string_lossy().into_owned())
                        .with_context(|| format!("{}", p.display()))
                })
                .collect::<Result<Vec<_>, _>>()?;
            match (to, text) {
                (None, Some(text)) => app.pick_device_for_text(text).await?,
                (None, None) => app.pick_device_for_files(paths).await?,
                (Some(device), text) => send(&app, &device, paths, text).await?,
            }
        }
        Command::Devices { wait } => {
            let devices = app.devices(wait).await?;
            if devices.is_empty() {
                bail!("no devices found nearby");
            }
            for (name, protocol, kind) in devices {
                println!("{name}\t{protocol}\t{kind}");
            }
        }
        Command::Visibility { value: None } => println!("{}", app.visibility().await?),
        Command::Visibility { value: Some(value) } => {
            let value = match value {
                VisibilityArg::Visible => "visible",
                VisibilityArg::Hidden => "hidden",
                VisibilityArg::Temporary => "temporary",
            };
            app.set_visibility(value.into()).await?;
        }
        Command::Show => app.show().await?,
    }
    Ok(())
}

/// Sends to `device` and follows the transfer until it ends.
async fn send(
    app: &RQuickShareProxy<'_>,
    device: &str,
    paths: Vec<String>,
    text: Option<String>,
) -> Result<(), anyhow::Error> {
    // Subscribed first, so no update is missed.
    let mut updates = app.receive_transfer_changed().await?;
    eprintln!("Looking for {device}…");
    let id = match text {
        Some(text) => app.send_text(device.into(), text).await?,
        None => app.send_files(device.into(), paths).await?,
    };

    let mut ctrl_c = std::pin::pin!(tokio::signal::ctrl_c());
    loop {
        let update = tokio::select! {
            _ = &mut ctrl_c => {
                app.cancel(id).await?;
                bail!("cancelled");
            }
            update = updates.next() => update.ok_or_else(|| anyhow!("the app went away"))?,
        };
        let args = update.args()?;
        if args.id != id || args.direction != "outbound" {
            continue;
        }
        match args.state.as_str() {
            "SentIntroduction" => eprintln!("Waiting for {} to accept…", args.peer),
            "SendingFiles" if args.total > 0 => {
                eprint!("\rSending… {}%", args.done * 100 / args.total);
            }
            "Finished" => {
                eprintln!("\rSent to {}.       ", args.peer);
                return Ok(());
            }
            "Rejected" => bail!("{} declined", args.peer),
            state if is_terminal(state) => bail!("transfer ended: {state}"),
            _ => {}
        }
    }
}

/// A proxy to the running app, starting it in the tray if needed.
async fn connect(connection: &zbus::Connection) -> Result<RQuickShareProxy<'_>, anyhow::Error> {
    let dbus = zbus::fdo::DBusProxy::new(connection).await?;
    let name = zbus::names::BusName::try_from(BUS_NAME)?;
    if !dbus.name_has_owner(name.clone()).await? {
        let exe = match std::env::var_os("APPIMAGE") {
            Some(appimage) => PathBuf::from(appimage),
            None => std::env::current_exe()?,
        };
        let mut app = std::process::Command::new(exe);
        without_appimage_env(&mut app);
        // The AppImage runtime hands us a descriptor on its mount (1023),
        // which would keep this process's mount alive as long as the app runs.
        // SAFETY: close_range is async-signal-safe and touches no Rust state.
        unsafe {
            app.pre_exec(|| {
                libc::close_range(3, libc::c_uint::MAX, 0);
                Ok(())
            });
        }
        app.arg(crate::HIDDEN_ARG)
            // Our working directory may be inside our AppImage's mount.
            .current_dir("/")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .context("couldn't start QuickShare")?;

        let started = tokio::time::timeout(Duration::from_secs(20), async {
            while !dbus.name_has_owner(name.clone()).await.unwrap_or(false) {
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
        })
        .await;
        if started.is_err() {
            bail!("QuickShare didn't start");
        }
    }

    Ok(RQuickShareProxy::builder(connection)
        .path(OBJECT_PATH)?
        .build()
        .await?)
}

/// Drops what our own AppImage put in the environment (library and data paths
/// into its mount), so the app we start doesn't use or pin this process's mount.
fn without_appimage_env(command: &mut std::process::Command) {
    let Some(appdir) = std::env::var_os("APPDIR") else {
        return;
    };
    let appdir = appdir.to_string_lossy().into_owned();
    for (key, value) in std::env::vars_os() {
        let value = value.to_string_lossy();
        if !value.contains(&appdir) {
            continue;
        }
        let kept = value
            .split(':')
            .filter(|part| !part.contains(&appdir))
            .collect::<Vec<_>>()
            .join(":");
        if kept.is_empty() {
            command.env_remove(key);
        } else {
            command.env(key, kept);
        }
    }
    for key in ["APPDIR", "APPIMAGE", "ARGV0", "OWD"] {
        command.env_remove(key);
    }
}

/// Chrome's native messaging: one length-prefixed JSON request on stdin, one
/// reply on stdout. `{"text": "..."}` opens the app to send that text or link.
async fn native_host() -> Result<(), anyhow::Error> {
    let request = read_native_message()?;
    let reply = match request.get("text").and_then(|t| t.as_str()) {
        Some(text) if !text.is_empty() => match pick_device_for_text(text).await {
            Ok(()) => serde_json::json!({ "ok": true }),
            Err(e) => serde_json::json!({ "ok": false, "error": format!("{e:#}") }),
        },
        _ => serde_json::json!({ "ok": false, "error": "nothing to send" }),
    };
    write_native_message(&reply)
}

async fn pick_device_for_text(text: &str) -> Result<(), anyhow::Error> {
    let connection = zbus::Connection::session().await?;
    connect(&connection)
        .await?
        .pick_device_for_text(text.into())
        .await?;
    Ok(())
}

fn read_native_message() -> Result<serde_json::Value, anyhow::Error> {
    let mut stdin = std::io::stdin().lock();
    let mut len = [0; 4];
    stdin.read_exact(&mut len)?;
    let len = u32::from_ne_bytes(len) as usize;
    // Chrome caps messages to the host at 4 GB; ours are a link or some text.
    if len > 1024 * 1024 {
        bail!("message too large");
    }
    let mut body = vec![0; len];
    stdin.read_exact(&mut body)?;
    Ok(serde_json::from_slice(&body)?)
}

fn write_native_message(message: &serde_json::Value) -> Result<(), anyhow::Error> {
    let body = serde_json::to_vec(message)?;
    let mut stdout = std::io::stdout().lock();
    stdout.write_all(&(body.len() as u32).to_ne_bytes())?;
    stdout.write_all(&body)?;
    stdout.flush()?;
    Ok(())
}
