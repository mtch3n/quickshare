//! Whether a newer release than this build is on GitHub.

use serde::{Deserialize, Serialize};

const LATEST_RELEASE: &str = "https://api.github.com/repos/mtch3n/quickshare/releases/latest";

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    html_url: String,
}

#[derive(Serialize)]
pub struct Update {
    version: String,
    /// The release page, which has the AppImage and the browser extension.
    url: String,
}

/// The latest release, if it is newer than `current`.
pub async fn check(current: &semver::Version) -> Result<Option<Update>, anyhow::Error> {
    let release: Release = reqwest::Client::new()
        .get(LATEST_RELEASE)
        // GitHub's API refuses requests without one.
        .header(reqwest::header::USER_AGENT, "quickshare")
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    let latest = semver::Version::parse(release.tag_name.trim_start_matches('v'))?;
    Ok((latest > *current).then(|| Update {
        version: latest.to_string(),
        url: release.html_url,
    }))
}
