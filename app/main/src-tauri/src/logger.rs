use std::path::Path;
use std::str::FromStr;
use std::time::SystemTime;

use fern::colors::{Color, ColoredLevelConfig};
use tauri::{AppHandle, Manager};

use crate::store;

pub fn set_up_logging(app_handle: &AppHandle) -> Result<(), anyhow::Error> {
    let default_level = std::env::var("RQS_LOG")
        .ok()
        .or_else(|| store::log_level(app_handle))
        .and_then(|level| log::LevelFilter::from_str(&level).ok())
        .unwrap_or(if cfg!(debug_assertions) {
            log::LevelFilter::Trace
        } else {
            log::LevelFilter::Info
        });

    let colors = ColoredLevelConfig::new()
        .error(Color::Red)
        .warn(Color::Yellow)
        .info(Color::Green)
        .debug(Color::Blue)
        .trace(Color::Cyan);

    let dispatch = fern::Dispatch::new()
        .format(move |out, message, record| {
            out.finish(format_args!(
                "\x1B[2m{date}\x1b[0m {level: >5} \x1B[2m{target}:\x1b[0m {message}",
                date = humantime::format_rfc3339_seconds(SystemTime::now()),
                target = record.target(),
                level = colors.color(record.level()),
                message = message,
            ));
        })
        .level(default_level)
        .level_for("mdns_sd", log::LevelFilter::Error)
        .level_for("polling", log::LevelFilter::Error)
        .level_for("bluer", log::LevelFilter::Error)
        .level_for("async_io", log::LevelFilter::Error)
        .chain(std::io::stdout());

    if let Ok(dir) = app_handle.path().app_log_dir() {
        std::fs::create_dir_all(&dir)?;
        let path = dir.join(format!("{}.log", app_handle.package_info().name));
        rotate(&path)?;

        dispatch.chain(fern::log_file(path)?).apply()?;
    } else {
        dispatch.apply()?;
    }

    debug!("Finished setting up logging");
    Ok(())
}

/// Keeps a single previous log around once the current one grows past the limit.
fn rotate(path: &Path) -> Result<(), anyhow::Error> {
    const MAX_LOG_SIZE: u64 = 5 * 1024 * 1024;

    if std::fs::metadata(path).is_ok_and(|m| m.len() > MAX_LOG_SIZE) {
        std::fs::rename(path, path.with_extension("old.log"))?;
    }

    Ok(())
}
