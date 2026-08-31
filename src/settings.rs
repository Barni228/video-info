//! Persisted user settings: their on-disk shape, where that file lives,
//! and a small write-through store the UI callbacks mutate.

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// Default for [`Settings::font_size`], in logical pixels. Also handed to
/// the Settings window so its "reset to default" buttons don't have to
/// hardcode a copy of it.
pub const DEFAULT_FONT_SIZE: f32 = 16.0;

/// The theme the user picked in the Settings window. `System` follows the
/// OS color scheme; the other two pin it regardless of the OS.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemeSetting {
    #[default]
    System,
    Light,
    Dark,
}

impl ThemeSetting {
    /// Every theme, in the order the Settings window lists them. This is
    /// the single source of truth for that dropdown: the labels and the
    /// index each one reports back both come from here (see
    /// `theme-options` in ui/settings-window.slint, which Rust fills in).
    pub const ALL: [ThemeSetting; 3] = [Self::System, Self::Light, Self::Dark];

    /// The label shown in the dropdown.
    pub fn label(self) -> &'static str {
        match self {
            Self::System => "System",
            Self::Light => "Light",
            Self::Dark => "Dark",
        }
    }

    /// This theme's position in [`Self::ALL`], i.e. the dropdown's
    /// `current-index`.
    pub fn index(self) -> i32 {
        Self::ALL
            .iter()
            .position(|&theme| theme == self)
            .unwrap_or(0) as i32
    }

    /// Resolves the label the dropdown hands back on selection, falling
    /// back to the default for anything unrecognized.
    pub fn from_label(label: &str) -> Self {
        Self::ALL
            .into_iter()
            .find(|theme| theme.label() == label)
            .unwrap_or_default()
    }
}

/// How often the app should ask GitHub whether a newer release exists.
/// The check is only ever a notification -- see [`crate::update`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UpdateInterval {
    /// On every launch.
    Always,
    #[default]
    Daily,
    Weekly,
    /// Not at all. Nothing is sent to GitHub.
    Never,
}

impl UpdateInterval {
    /// Every interval, in the order the Settings window lists them --
    /// most frequent first, the same source-of-truth arrangement as
    /// [`ThemeSetting::ALL`].
    pub const ALL: [UpdateInterval; 4] = [Self::Always, Self::Daily, Self::Weekly, Self::Never];

    pub fn label(self) -> &'static str {
        match self {
            Self::Always => "Every launch",
            Self::Daily => "Daily",
            Self::Weekly => "Weekly",
            Self::Never => "Never",
        }
    }

    pub fn index(self) -> i32 {
        Self::ALL
            .iter()
            .position(|&interval| interval == self)
            .unwrap_or(0) as i32
    }

    pub fn from_label(label: &str) -> Self {
        Self::ALL
            .into_iter()
            .find(|interval| interval.label() == label)
            .unwrap_or_default()
    }

    /// How long to wait between checks, or `None` for the intervals that
    /// aren't a wait at all.
    fn gap(self) -> Option<Duration> {
        match self {
            Self::Always => Some(Duration::ZERO),
            Self::Daily => Some(Duration::from_secs(24 * 60 * 60)),
            Self::Weekly => Some(Duration::from_secs(7 * 24 * 60 * 60)),
            Self::Never => None,
        }
    }

    /// Whether a check is owed, given when the last one ran. A check that
    /// has never run is always owed -- except under `Never`.
    pub fn is_due(self, last_checked: Option<SystemTime>) -> bool {
        let Some(gap) = self.gap() else {
            return false;
        };
        let Some(last_checked) = last_checked else {
            return true;
        };
        match last_checked.elapsed() {
            Ok(since) => since >= gap,
            // The stored time is in the future, so the system clock moved
            // between checks. Waiting for it to catch up could suppress
            // checks for as long as the jump lasted; check now instead.
            Err(_) => true,
        }
    }
}

/// Everything the Settings window can change, as stored in settings.json.
/// `#[serde(default)]` fills in any field a file written by an older
/// version is missing, so settings survive upgrades.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub font_size: f32,
    pub theme: ThemeSetting,
    pub always_on_top: bool,
    pub update_interval: UpdateInterval,
    /// When the last update check ran, as seconds since the Unix epoch.
    /// Stored rather than kept in memory so the interval survives a
    /// restart -- otherwise every launch would be "due".
    pub last_update_check: Option<u64>,
    /// The newest version GitHub reported, remembered so the notice
    /// survives restarts between checks: found once on Monday, a weekly
    /// interval shouldn't go quiet again until the following Monday.
    pub latest_seen_version: Option<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            font_size: DEFAULT_FONT_SIZE,
            theme: ThemeSetting::default(),
            always_on_top: false,
            update_interval: UpdateInterval::default(),
            last_update_check: None,
            latest_seen_version: None,
        }
    }
}

impl Settings {
    /// [`Self::last_update_check`] as a [`SystemTime`].
    pub fn last_update_check_time(&self) -> Option<SystemTime> {
        Some(UNIX_EPOCH + Duration::from_secs(self.last_update_check?))
    }

    /// Records that a check just ran, along with what it found. `latest`
    /// is `None` when the check failed, which leaves the last known
    /// version in place rather than forgetting it.
    pub fn record_update_check(&mut self, latest: Option<String>) {
        self.last_update_check = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .ok()
            .map(|since| since.as_secs());
        if latest.is_some() {
            self.latest_seen_version = latest;
        }
    }
}

/// The settings held in memory, written back to disk on every change.
/// Saving eagerly is cheap here -- the file is a few hundred bytes and
/// only changes when the user moves a control -- and means the app never
/// has to flush anything on exit.
///
/// A `Mutex` rather than a `RefCell` because the update check records its
/// result from the background thread it runs on (see `ui::check_for_update`).
pub struct SettingsStore {
    current: Mutex<Settings>,
}

impl SettingsStore {
    /// Reads the saved settings, falling back to the defaults if the file
    /// is missing, unreadable, or corrupt.
    pub fn load() -> Self {
        let current = settings_path()
            .and_then(|path| std::fs::read_to_string(path).ok())
            .and_then(|json| serde_json::from_str(&json).ok())
            .unwrap_or_default();
        Self {
            current: Mutex::new(current),
        }
    }

    pub fn get(&self) -> Settings {
        self.lock().clone()
    }

    /// Applies `edit` to the settings and persists the result.
    pub fn update(&self, edit: impl FnOnce(&mut Settings)) {
        let mut current = self.lock();
        edit(&mut current);
        save(&current);
    }

    /// The settings, recovering the contents if a thread panicked while
    /// holding the lock. Losing a preference is not worth a second panic.
    fn lock(&self) -> std::sync::MutexGuard<'_, Settings> {
        self.current.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// Cross-platform settings location: `%APPDATA%\Video Info` on Windows,
/// `~/Library/Application Support/Video Info` on macOS, and
/// `~/.config/Video Info` elsewhere.
fn settings_path() -> Option<PathBuf> {
    let dir = if cfg!(target_os = "windows") {
        std::env::var_os("APPDATA").map(PathBuf::from)
    } else if cfg!(target_os = "macos") {
        std::env::var_os("HOME").map(|home| PathBuf::from(home).join("Library/Application Support"))
    } else {
        std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config"))
    };
    dir.map(|dir| dir.join("Video Info").join("settings.json"))
}

/// Writes the settings out, silently giving up if the location isn't
/// writable: a preference that fails to persist isn't worth interrupting
/// the user over.
fn save(settings: &Settings) {
    let Some(path) = settings_path() else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(json) = serde_json::to_string_pretty(settings) {
        let _ = std::fs::write(path, json);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ago(seconds: u64) -> Option<SystemTime> {
        Some(SystemTime::now() - Duration::from_secs(seconds))
    }

    #[test]
    fn never_checking_means_never() {
        assert!(!UpdateInterval::Never.is_due(None));
        assert!(!UpdateInterval::Never.is_due(ago(365 * 24 * 60 * 60)));
    }

    #[test]
    fn every_launch_always_checks() {
        assert!(UpdateInterval::Always.is_due(None));
        assert!(UpdateInterval::Always.is_due(ago(1)));
    }

    #[test]
    fn a_first_run_is_always_due() {
        assert!(UpdateInterval::Daily.is_due(None));
        assert!(UpdateInterval::Weekly.is_due(None));
    }

    #[test]
    fn waits_out_the_interval_before_checking_again() {
        let hour = 60 * 60;
        assert!(!UpdateInterval::Daily.is_due(ago(23 * hour)));
        assert!(UpdateInterval::Daily.is_due(ago(25 * hour)));
        assert!(!UpdateInterval::Weekly.is_due(ago(6 * 24 * hour)));
        assert!(UpdateInterval::Weekly.is_due(ago(8 * 24 * hour)));
    }

    #[test]
    fn a_clock_that_jumped_forward_does_not_block_checks_forever() {
        let tomorrow = SystemTime::now() + Duration::from_secs(24 * 60 * 60);
        assert!(UpdateInterval::Daily.is_due(Some(tomorrow)));
    }

    #[test]
    fn settings_files_from_older_versions_still_load() {
        // Written before any of the update fields existed.
        let json = r#"{"font_size":18.0,"theme":"dark","always_on_top":true}"#;
        let settings: Settings = serde_json::from_str(json).expect("should load");
        assert_eq!(settings.font_size, 18.0);
        assert_eq!(settings.theme, ThemeSetting::Dark);
        assert_eq!(settings.update_interval, UpdateInterval::Daily);
        assert_eq!(settings.last_update_check, None);
    }

    #[test]
    fn a_failed_check_keeps_the_last_known_version() {
        let mut settings = Settings::default();
        settings.record_update_check(Some("0.6.0".to_string()));
        assert_eq!(settings.latest_seen_version.as_deref(), Some("0.6.0"));
        assert!(settings.last_update_check.is_some());

        let checked_at = settings.last_update_check;
        settings.record_update_check(None);
        assert_eq!(settings.latest_seen_version.as_deref(), Some("0.6.0"));
        // The attempt still counts, so a dead network isn't retried on
        // every launch.
        assert!(settings.last_update_check >= checked_at);
    }
}
