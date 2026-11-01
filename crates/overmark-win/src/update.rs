//! The update check's one request: only when the user has turned "Check for updates"
//! on, and only ever this: a `GET` of the project's latest-release page on GitHub
//! ([`core::update::LATEST_URL`]) with redirects off, reading the redirect's `Location`. Nothing
//! is downloaded or installed; the app shows a link.
//!
//! The request is [`crate::win32::http`]'s, with WinHTTP, part of Windows: no HTTP or TLS crate
//! ships in Overmark. These two modules are the only ones in Overmark that touch the network.

use std::time::Duration;

use crate::core::tr;
use crate::core::update::{self, Outcome};
use crate::win32::http;

/// How long each step of the request may take.
const TIMEOUT: Duration = Duration::from_secs(15);

/// Why a check didn't work, in the system's words.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckError(pub String);

impl std::fmt::Display for CheckError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Asks GitHub for the latest release and compares it with `current` (this build's version).
/// Blocks for up to a few seconds: call it off the UI thread.
pub fn check_latest(current: &str) -> Result<Outcome, CheckError> {
    let agent = format!("Overmark/{}", env!("CARGO_PKG_VERSION"));
    let answer = http::redirect_location(
        "github.com",
        "/Suremise/overmark/releases/latest",
        &agent,
        TIMEOUT,
    )
    .map_err(CheckError)?;
    if !(300..400).contains(&answer.status) {
        return Err(CheckError(tr!(
            "update-failed-status",
            status = answer.status
        )));
    }
    let location = answer.location;
    let Some((version, url)) = location.as_deref().and_then(update::latest_from_location) else {
        tracing::info!(location = ?location, "no release named by GitHub");
        return Ok(Outcome::UpToDate);
    };
    update::outcome(current, version, url)
        .ok_or_else(|| CheckError(tr!("update-failed-version", version = current)))
}
