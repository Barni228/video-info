// Prevent a console window from popping up alongside the GUI on Windows
// release builds.
#![cfg_attr(
    all(target_os = "windows", not(debug_assertions)),
    windows_subsystem = "windows"
)]

//! Video Info: shows the technical metadata of a video or audio file.
//!
//! This file is only the startup path -- create the windows, connect them,
//! and hand over to the event loop. The work lives in:
//!
//! - [`settings`]: what the Settings window persists, and where.
//! - [`ffprobe`]: running ffprobe and deserializing its report.
//! - [`report`]: turning that report into the rows the UI displays.
//! - [`ui`]: pushing state into the Slint windows and handling their
//!   callbacks.
//! - `macos_open`: receiving files from macOS "Open With" (macOS only).

slint::include_modules!();

mod ffprobe;
#[cfg(target_os = "macos")]
mod macos_open;
mod report;
mod settings;
mod ui;

use std::path::PathBuf;
use std::rc::Rc;

use settings::SettingsStore;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Select the winit-backed platform explicitly so we can hook raw
    // window events, which native OS drag & drop of files needs.
    slint::BackendSelector::new()
        .backend_name("winit".into())
        .select()?;

    // Set up before anything can be delivered to it, i.e. before the
    // window (and with it NSApplication) exists.
    #[cfg(target_os = "macos")]
    let opened_files = macos_open::take_receiver();

    let app = AppWindow::new()?;
    let settings_window = SettingsWindow::new()?;
    let store = Rc::new(SettingsStore::load());
    ui::wire(&app, &settings_window, &store);

    // Must come after AppWindow::new(): the swizzle needs winit to have
    // already created NSApplication and set its own delegate, since it
    // adds a method to that existing delegate's class rather than
    // replacing the delegate itself (see macos_open.rs for why).
    #[cfg(target_os = "macos")]
    {
        macos_open::install_open_urls_swizzle();
        forward_opened_files(&app, opened_files);
    }

    // Opened via a command-line argument: Windows file associations, or
    // `cargo run -- file.mp4`. macOS "Open With" doesn't arrive this way;
    // it comes through macos_open.rs instead.
    if let Some(path) = std::env::args().nth(1).map(PathBuf::from)
        && path.exists()
    {
        ui::analyze_file(&app.as_weak(), path);
    }

    app.run()?;
    Ok(())
}

/// Feeds the files macOS opens this app with -- including ones opened
/// while it's already running -- into the UI thread.
#[cfg(target_os = "macos")]
fn forward_opened_files(app: &AppWindow, opened_files: std::sync::mpsc::Receiver<String>) {
    let app_weak = app.as_weak();
    std::thread::spawn(move || {
        while let Ok(path) = opened_files.recv() {
            let app_weak = app_weak.clone();
            let _ = slint::invoke_from_event_loop(move || {
                ui::analyze_file(&app_weak, PathBuf::from(path));
            });
        }
    });
}
