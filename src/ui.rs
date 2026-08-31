//! Everything that touches the Slint windows: pushing state into them,
//! and wiring their callbacks back to the rest of the app.

use std::path::{Path, PathBuf};
use std::rc::Rc;

use copypasta::ClipboardProvider;
use slint::winit_030::{EventResult, WinitWindowAccessor, winit};
use slint::{ComponentHandle, ModelRc, SharedString, VecModel, Weak};
use winit::event::WindowEvent;

use crate::file_kind::{AUDIO_EXTENSIONS, VIDEO_EXTENSIONS};
use crate::settings::{self, Settings, SettingsStore, ThemeSetting};
use crate::{AppWindow, InfoRow, SettingsWindow, Theme, ThemeMode, ffprobe, report};

impl From<ThemeSetting> for ThemeMode {
    fn from(setting: ThemeSetting) -> Self {
        match setting {
            ThemeSetting::System => ThemeMode::System,
            ThemeSetting::Light => ThemeMode::Light,
            ThemeSetting::Dark => ThemeMode::Dark,
        }
    }
}

/// Fills both windows with the stored settings and connects every
/// callback they expose. Call once, after both windows exist.
pub fn wire(app: &AppWindow, settings_window: &SettingsWindow, store: &Rc<SettingsStore>) {
    apply_stored_settings(app, settings_window, store.get());
    wire_shutdown(app, settings_window);
    wire_window_events(app, settings_window, store);
    wire_main_window(app, settings_window);
    wire_settings_window(settings_window, app, store);
}

/// Analyzes `path` on a background thread, updating the UI before and
/// after. Safe to call from any of the ways a file can arrive: drag and
/// drop, the file picker, "Open With", or a command-line argument.
pub fn analyze_file(app_weak: &Weak<AppWindow>, path: PathBuf) {
    if let Some(app) = app_weak.upgrade() {
        app.set_has_file(true);
        app.set_file_name(display_name(&path).into());
        app.set_status_text("Analyzing...".into());
        set_status_kind(&app, StatusKind::Progress);
        app.set_status_detail(SharedString::new());
        app.set_info_rows(ModelRc::new(VecModel::from(Vec::<InfoRow>::new())));
        app.set_raw_info_text(SharedString::new());
    }

    // ffprobe takes long enough on large files to freeze the UI, so run it
    // off the event loop and marshal the result back.
    let app_weak = app_weak.clone();
    std::thread::spawn(move || {
        let result = ffprobe::run(&path)
            .map(|probed| (report::rows(&path, &probed.report), probed.warning));
        let _ = slint::invoke_from_event_loop(move || {
            let Some(app) = app_weak.upgrade() else {
                return;
            };
            match result {
                Ok((rows, warning)) => {
                    match warning {
                        // The file was readable, but ffprobe grumbled on
                        // the way through -- so show the rows, and say
                        // they may not be the whole story.
                        Some(warning) => {
                            app.set_status_text(
                                "This file has problems, but here's what could be read.".into(),
                            );
                            app.set_status_detail(warning.into());
                            set_status_kind(&app, StatusKind::Warning);
                        }
                        None => {
                            app.set_status_text(SharedString::new());
                            set_status_kind(&app, StatusKind::Progress);
                        }
                    }
                    app.set_raw_info_text(report::as_text(&rows).into());
                    app.set_info_rows(ModelRc::new(VecModel::from(rows)));
                }
                Err(err) => {
                    app.set_status_text(err.headline().into());
                    app.set_status_detail(err.detail().unwrap_or_default().into());
                    set_status_kind(&app, StatusKind::Error);
                }
            }
        });
    });
}

/// How the status line under the Browse button should read: as an
/// error, as a caveat on results that are shown anyway, or as neither.
enum StatusKind {
    Progress,
    Warning,
    Error,
}

fn set_status_kind(app: &AppWindow, kind: StatusKind) {
    app.set_is_error(matches!(kind, StatusKind::Error));
    app.set_is_warning(matches!(kind, StatusKind::Warning));
}

/// Pushes `stored` into both windows, along with the defaults their
/// "reset to default" buttons restore.
fn apply_stored_settings(app: &AppWindow, settings_window: &SettingsWindow, stored: Settings) {
    let defaults = Settings::default();

    app.set_base_font_size(stored.font_size);
    app.set_keep_on_top(stored.always_on_top);

    settings_window.set_font_size(stored.font_size);
    settings_window.set_font_size_default(settings::DEFAULT_FONT_SIZE);
    settings_window.set_theme_options(theme_options());
    settings_window.set_theme_index(stored.theme.index());
    settings_window.set_theme_default_index(defaults.theme.index());
    settings_window.set_keep_on_top(stored.always_on_top);

    // The theme is applied once the event loop is running instead of right
    // now: resolving ThemeSetting::System needs the winit window, which
    // only exists once the loop is pumping (see WinitWindowAccessor).
    let app_weak = app.as_weak();
    let settings_window_weak = settings_window.as_weak();
    let _ = slint::invoke_from_event_loop(move || {
        if let (Some(app), Some(settings_window)) =
            (app_weak.upgrade(), settings_window_weak.upgrade())
        {
            apply_theme(&app, &settings_window, stored.theme);
        }
    });
}

/// The theme dropdown's model, straight from [`ThemeSetting::ALL`] so the
/// UI never has its own copy of the list.
fn theme_options() -> ModelRc<SharedString> {
    let labels: Vec<SharedString> = ThemeSetting::ALL
        .iter()
        .map(|theme| theme.label().into())
        .collect();
    ModelRc::new(VecModel::from(labels))
}

/// Pushes `theme` into the `Theme` global (see ui/theme.slint) on both
/// windows: each top-level component gets its own independent copy of the
/// globals it uses, so both need keeping in sync.
fn apply_theme(app: &AppWindow, settings_window: &SettingsWindow, theme: ThemeSetting) {
    let is_dark = system_is_dark(app);
    for global in [app.global::<Theme>(), settings_window.global::<Theme>()] {
        global.set_mode(theme.into());
        global.set_system_is_dark(is_dark);
    }
}

/// The OS's current color scheme, used to resolve `ThemeSetting::System`.
fn system_is_dark(app: &AppWindow) -> bool {
    app.window()
        .with_winit_window(|window| window.theme() == Some(winit::window::Theme::Dark))
        .unwrap_or(false)
}

/// Closing the main window doesn't quit the app on its own while the
/// Settings window is still shown -- it keeps its own strong reference
/// alive. Close it too, so the app actually exits.
fn wire_shutdown(app: &AppWindow, settings_window: &SettingsWindow) {
    let settings_window_weak = settings_window.as_weak();
    app.window().on_close_requested(move || {
        if let Some(settings_window) = settings_window_weak.upgrade() {
            let _ = settings_window.hide();
        }
        slint::CloseRequestResponse::HideWindow
    });
}

/// Raw winit events, for the two things Slint doesn't surface itself:
/// native OS file drag & drop, and OS color-scheme changes.
fn wire_window_events(
    app: &AppWindow,
    settings_window: &SettingsWindow,
    store: &Rc<SettingsStore>,
) {
    let app_weak = app.as_weak();
    let settings_window_weak = settings_window.as_weak();
    let store = store.clone();

    app.window().on_winit_window_event(move |_window, event| {
        match event {
            WindowEvent::HoveredFile(_) => set_drag_hover(&app_weak, true),
            WindowEvent::HoveredFileCancelled => set_drag_hover(&app_weak, false),
            WindowEvent::DroppedFile(path) => {
                set_drag_hover(&app_weak, false);
                analyze_file(&app_weak, path.clone());
            }
            WindowEvent::ThemeChanged(_) => {
                if store.get().theme == ThemeSetting::System
                    && let (Some(app), Some(settings_window)) =
                        (app_weak.upgrade(), settings_window_weak.upgrade())
                {
                    apply_theme(&app, &settings_window, ThemeSetting::System);
                }
            }
            _ => {}
        }
        EventResult::Propagate
    });
}

fn set_drag_hover(app_weak: &Weak<AppWindow>, hovering: bool) {
    if let Some(app) = app_weak.upgrade() {
        app.set_is_drag_hover(hovering);
    }
}

fn wire_main_window(app: &AppWindow, settings_window: &SettingsWindow) {
    let app_weak = app.as_weak();
    app.on_browse_clicked(move || {
        let file = rfd::FileDialog::new()
            .set_title("Choose a file")
            .add_filter("Video files", VIDEO_EXTENSIONS)
            .add_filter("Audio files", AUDIO_EXTENSIONS)
            .add_filter("All files", &["*"])
            .pick_file();

        if let Some(path) = file {
            analyze_file(&app_weak, path);
        }
    });

    let app_weak = app.as_weak();
    app.on_copy_all_clicked(move || {
        if let Some(app) = app_weak.upgrade()
            && let Ok(mut clipboard) = copypasta::ClipboardContext::new()
        {
            let _ = clipboard.set_contents(app.get_raw_info_text().to_string());
        }
    });

    let settings_window_weak = settings_window.as_weak();
    app.on_settings_clicked(move || {
        let Some(settings_window) = settings_window_weak.upgrade() else {
            return;
        };
        let _ = settings_window.show();

        // Work around the window sometimes staying blank until its first
        // resize when it's shown long after being created: ask for a
        // redraw on the next event loop tick, once the OS has finished
        // mapping the window.
        let settings_window_weak = settings_window.as_weak();
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(settings_window) = settings_window_weak.upgrade() {
                settings_window.window().request_redraw();
            }
        });
    });
}

/// Each setting is applied to the live UI and saved as it changes -- there
/// is no OK/Apply button.
fn wire_settings_window(
    settings_window: &SettingsWindow,
    app: &AppWindow,
    store: &Rc<SettingsStore>,
) {
    let app_weak = app.as_weak();
    let store_ref = store.clone();
    settings_window.on_font_size_changed(move |font_size| {
        if let Some(app) = app_weak.upgrade() {
            app.set_base_font_size(font_size);
        }
        store_ref.update(|settings| settings.font_size = font_size);
    });

    let app_weak = app.as_weak();
    let settings_window_weak = settings_window.as_weak();
    let store_ref = store.clone();
    settings_window.on_theme_changed(move |label| {
        let theme = ThemeSetting::from_label(&label);
        if let (Some(app), Some(settings_window)) =
            (app_weak.upgrade(), settings_window_weak.upgrade())
        {
            apply_theme(&app, &settings_window, theme);
        }
        store_ref.update(|settings| settings.theme = theme);
    });

    let app_weak = app.as_weak();
    let store_ref = store.clone();
    settings_window.on_keep_on_top_changed(move |keep_on_top| {
        if let Some(app) = app_weak.upgrade() {
            app.set_keep_on_top(keep_on_top);
        }
        store_ref.update(|settings| settings.always_on_top = keep_on_top);
    });
}

/// The file's name for display, falling back to the whole path for the
/// odd path that has no final component.
fn display_name(path: &Path) -> String {
    path.file_name()
        .unwrap_or(path.as_os_str())
        .to_string_lossy()
        .into_owned()
}
