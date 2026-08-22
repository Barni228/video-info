//! Persisted user settings: their on-disk shape, where that file lives,
//! and a small write-through store the UI callbacks mutate.

use std::cell::RefCell;
use std::path::PathBuf;

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

/// Everything the Settings window can change, as stored in settings.json.
/// `#[serde(default)]` fills in any field a file written by an older
/// version is missing, so settings survive upgrades.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub font_size: f32,
    pub theme: ThemeSetting,
    pub always_on_top: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            font_size: DEFAULT_FONT_SIZE,
            theme: ThemeSetting::default(),
            always_on_top: false,
        }
    }
}

/// The settings held in memory, written back to disk on every change.
/// Saving eagerly is cheap here -- the file is a few hundred bytes and
/// only changes when the user moves a control -- and means the app never
/// has to flush anything on exit.
pub struct SettingsStore {
    current: RefCell<Settings>,
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
            current: RefCell::new(current),
        }
    }

    pub fn get(&self) -> Settings {
        *self.current.borrow()
    }

    /// Applies `edit` to the settings and persists the result.
    pub fn update(&self, edit: impl FnOnce(&mut Settings)) {
        let mut current = self.current.borrow_mut();
        edit(&mut current);
        save(&current);
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
