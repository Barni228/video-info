//! Checking GitHub for a newer release.
//!
//! This only ever *tells* the user an update exists and offers to open the
//! releases page -- nothing is downloaded or installed. The app ships
//! unsigned, so replacing it in place is the OS's business, not ours.

use std::time::Duration;

use serde::Deserialize;

/// The version of this build, straight from Cargo.toml.
pub const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Where the "See newest release" button sends the user.
pub const RELEASES_URL: &str = "https://github.com/Barni228/video-info/releases/latest";

/// The unauthenticated API GitHub serves the newest release from. Public
/// repositories need no token; the rate limit is per-IP and generous next
/// to one call a day.
const LATEST_RELEASE_API: &str =
    "https://api.github.com/repos/Barni228/video-info/releases/latest";

/// Long enough for a slow connection, short enough that a hung request
/// doesn't leave a thread parked for the life of the app.
const TIMEOUT: Duration = Duration::from_secs(10);

/// Only the field we need out of GitHub's release JSON.
#[derive(Deserialize)]
struct LatestRelease {
    tag_name: String,
}

/// Asks GitHub for the newest published release and returns its version,
/// with any leading `v` removed.
///
/// Every failure -- offline, rate-limited, GitHub down, JSON we don't
/// recognize -- comes back as `None`. There is nothing the user could do
/// about any of them, and an update check is not worth an error message.
pub fn latest_version() -> Option<String> {
    let agent = ureq::Agent::config_builder()
        .timeout_global(Some(TIMEOUT))
        .build()
        .new_agent();

    let body = agent
        .get(LATEST_RELEASE_API)
        .header("User-Agent", concat!("video-info/", env!("CARGO_PKG_VERSION")))
        .header("Accept", "application/vnd.github+json")
        .call()
        .ok()?
        .body_mut()
        .read_to_string()
        .ok()?;

    let release: LatestRelease = serde_json::from_str(&body).ok()?;
    Some(release.tag_name.trim_start_matches('v').to_string())
}

/// Whether `candidate` is a later version than `current`.
///
/// Anything unparsable is treated as "not newer": a tag that doesn't look
/// like a version is far more likely to be a mistake than a release worth
/// nagging about.
pub fn is_newer(candidate: &str, current: &str) -> bool {
    match (parse_version(candidate), parse_version(current)) {
        (Some(candidate), Some(current)) => candidate > current,
        _ => false,
    }
}

/// `"v1.2.3"` or `"1.2.3-beta"` to `(1, 2, 3)`, so versions compare as
/// numbers rather than as text -- `"0.10.0"` sorts before `"0.9.0"` as a
/// string, and after it as a version.
///
/// Any pre-release suffix is dropped, which makes `1.2.3-beta` compare
/// equal to `1.2.3`. That is deliberate: it stops a pre-release of the
/// version already installed from being announced as an update.
fn parse_version(version: &str) -> Option<(u64, u64, u64)> {
    let version = version.trim().trim_start_matches('v');
    let version = version.split(['-', '+']).next()?;

    let mut parts = version.split('.');
    let major = parts.next()?.parse().ok()?;
    // A tag may stop early: "1" and "1.2" are both understood as releases
    // with the missing components at zero.
    let minor = next_number(&mut parts)?;
    let patch = next_number(&mut parts)?;
    if parts.next().is_some() {
        return None;
    }
    Some((major, minor, patch))
}

/// The next dot-separated number, or zero once they run out. `None` only
/// for a component that is present but isn't a number.
fn next_number<'a>(parts: &mut impl Iterator<Item = &'a str>) -> Option<u64> {
    match parts.next() {
        Some(part) => part.parse().ok(),
        None => Some(0),
    }
}

/// Opens `url` in the user's browser, doing nothing if that fails --
/// there is no useful way to recover, and the user can reach the page
/// themselves.
pub fn open_in_browser(url: &str) {
    // Each platform's "open this the way the user would" command. Every
    // one of these hands off to the default browser.
    #[cfg(target_os = "macos")]
    let mut command = {
        let mut command = std::process::Command::new("open");
        command.arg(url);
        command
    };

    #[cfg(target_os = "windows")]
    let mut command = {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        let mut command = std::process::Command::new("cmd");
        // The empty string is `start`'s optional window title. Without it
        // `start` treats the URL as the title and opens nothing.
        command.args(["/C", "start", "", url]);
        command.creation_flags(CREATE_NO_WINDOW);
        command
    };

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let mut command = {
        let mut command = std::process::Command::new("xdg-open");
        command.arg(url);
        command
    };

    let _ = command.spawn();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compares_versions_as_numbers_not_text() {
        assert!(is_newer("0.10.0", "0.9.0"));
        assert!(is_newer("1.0.0", "0.99.99"));
        assert!(is_newer("0.5.2", "0.5.1"));
    }

    #[test]
    fn the_installed_version_is_not_an_update() {
        assert!(!is_newer("0.5.1", "0.5.1"));
        assert!(!is_newer("0.5.0", "0.5.1"));
        assert!(!is_newer("0.4.9", "0.5.1"));
    }

    #[test]
    fn tolerates_the_leading_v_on_git_tags() {
        assert!(is_newer("v0.6.0", "0.5.1"));
        assert!(!is_newer("v0.5.1", "v0.5.1"));
    }

    #[test]
    fn treats_short_tags_as_having_trailing_zeroes() {
        assert_eq!(parse_version("1"), Some((1, 0, 0)));
        assert_eq!(parse_version("1.2"), Some((1, 2, 0)));
        assert!(is_newer("1", "0.9.9"));
    }

    #[test]
    fn ignores_prerelease_suffixes() {
        assert_eq!(parse_version("1.2.3-beta.1"), Some((1, 2, 3)));
        // A pre-release of what is already installed is not an update.
        assert!(!is_newer("0.5.1-rc1", "0.5.1"));
        assert!(is_newer("0.6.0-rc1", "0.5.1"));
    }

    #[test]
    fn refuses_to_announce_an_unparsable_tag() {
        assert!(!is_newer("latest", "0.5.1"));
        assert!(!is_newer("", "0.5.1"));
        assert!(!is_newer("1.2.3.4", "0.5.1"));
        assert!(!is_newer("nightly-2026-01-01", "0.5.1"));
    }

    #[test]
    fn this_build_reports_a_usable_version() {
        assert!(parse_version(CURRENT_VERSION).is_some());
    }
}

