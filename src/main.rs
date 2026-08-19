// Prevent a console window from popping up alongside the GUI on Windows release builds.
#![cfg_attr(
    all(target_os = "windows", not(debug_assertions)),
    windows_subsystem = "windows"
)]

slint::include_modules!();

#[cfg(target_os = "macos")]
mod macos_open;

use serde::Deserialize;
use slint::{ModelRc, VecModel, Weak};
use std::path::{Path, PathBuf};
use std::process::Command;

use slint::winit_030::{winit, EventResult, WinitWindowAccessor};
use std::collections::HashMap;
use winit::event::WindowEvent as WinitWindowEvent;

#[derive(Debug, Deserialize)]
struct FfprobeOutput {
    #[serde(default)]
    streams: Vec<StreamInfo>,
    format: Option<FormatInfo>,
    #[serde(default)]
    chapters: Vec<ChapterInfo>,
}

#[derive(Debug, Deserialize)]
struct ChapterInfo {
    start_time: Option<String>,
    end_time: Option<String>,
    #[serde(default)]
    tags: HashMap<String, String>,
}

#[derive(Debug, Deserialize)]
struct StreamInfo {
    codec_type: Option<String>,
    codec_name: Option<String>,
    codec_long_name: Option<String>,
    profile: Option<String>,
    width: Option<i64>,
    height: Option<i64>,
    pix_fmt: Option<String>,
    r_frame_rate: Option<String>,
    avg_frame_rate: Option<String>,
    bit_rate: Option<String>,
    sample_rate: Option<String>,
    channels: Option<i64>,
    channel_layout: Option<String>,
    display_aspect_ratio: Option<String>,
    bits_per_raw_sample: Option<String>,
    #[serde(default)]
    tags: HashMap<String, String>,
}

#[derive(Debug, Deserialize)]
struct FormatInfo {
    format_long_name: Option<String>,
    format_name: Option<String>,
    duration: Option<String>,
    bit_rate: Option<String>,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Make sure the winit-backed platform is selected so we can hook raw window events
    // (needed for native OS drag-and-drop of files).
    slint::BackendSelector::new()
        .backend_name("winit".into())
        .select()?;

    #[cfg(target_os = "macos")]
    let open_doc_rx = {
        macos_open::log_line("[main] process started");
        macos_open::take_receiver()
    };

    let app = AppWindow::new()?;

    // --- macOS "Open With" / already-running file-open support ---
    // Must come after AppWindow::new(), since it needs winit to have already
    // created NSApplication and set its own delegate -- we're adding a
    // method to that existing delegate's class, not replacing the delegate
    // itself (see macos_open.rs for why).
    #[cfg(target_os = "macos")]
    {
        macos_open::log_line("[main] AppWindow created");
        macos_open::install_open_urls_swizzle();

        let app_weak = app.as_weak();
        std::thread::spawn(move || {
            while let Ok(path) = open_doc_rx.recv() {
                let app_weak = app_weak.clone();
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(app) = app_weak.upgrade() {
                        analyze_file(&app.as_weak(), PathBuf::from(path));
                    }
                });
            }
        });
    }

    // --- Native OS drag & drop support ---
    {
        let app_weak = app.as_weak();
        app.window().on_winit_window_event(move |_window, event| {
            match event {
                WinitWindowEvent::HoveredFile(_) => {
                    if let Some(app) = app_weak.upgrade() {
                        app.set_is_drag_hover(true);
                    }
                }
                WinitWindowEvent::HoveredFileCancelled => {
                    if let Some(app) = app_weak.upgrade() {
                        app.set_is_drag_hover(false);
                    }
                }
                WinitWindowEvent::DroppedFile(path) => {
                    if let Some(app) = app_weak.upgrade() {
                        app.set_is_drag_hover(false);
                    }
                    analyze_file(&app_weak, path.clone());
                }
                _ => {}
            }
            EventResult::Propagate
        });
    }

    // --- Browse button ---
    {
        let app_weak = app.as_weak();
        app.on_browse_clicked(move || {
            let app_weak = app_weak.clone();
            let file = rfd::FileDialog::new()
                .set_title("Choose a video file")
                .add_filter(
                    "Video files",
                    &[
                        "mp4", "mkv", "mov", "avi", "webm", "flv", "wmv", "m4v", "mpg", "mpeg",
                        "ts", "3gp", "ogv",
                    ],
                )
                .add_filter("All files", &["*"])
                .pick_file();

            if let Some(path) = file {
                analyze_file(&app_weak, path);
            }
        });
    }

    // --- Opened via CLI arg (Windows file associations, or `cargo run -- file.mp4`) ---
    // Not used by macOS "Open With", which is handled via the delegate
    // swizzle in macos_open.rs above.
    if let Some(path) = std::env::args().nth(1) {
        let path = PathBuf::from(path);
        if path.exists() {
            analyze_file(&app.as_weak(), path);
        }
    }

    app.run()?;
    Ok(())
}

/// Kick off ffprobe analysis of `path` on a background thread and update the UI when done.
fn analyze_file(app_weak: &Weak<AppWindow>, path: PathBuf) {
    if let Some(app) = app_weak.upgrade() {
        app.set_has_file(true);
        app.set_file_name(
            path.file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| path.to_string_lossy().to_string())
                .into(),
        );
        app.set_status_text("Analyzing...".into());
        app.set_is_error(false);
        app.set_info_rows(ModelRc::new(VecModel::from(Vec::<InfoRow>::new())));
    }

    let app_weak = app_weak.clone();
    std::thread::spawn(move || {
        let result = run_ffprobe(&path);
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(app) = app_weak.upgrade() {
                match result {
                    Ok(rows) => {
                        app.set_status_text("".into());
                        app.set_is_error(false);
                        app.set_info_rows(ModelRc::new(VecModel::from(rows)));
                    }
                    Err(err) => {
                        app.set_status_text(err.into());
                        app.set_is_error(true);
                    }
                }
            }
        });
    });
}

fn run_ffprobe(path: &Path) -> Result<Vec<InfoRow>, String> {
    // let output = Command::new("ffprobe")
    let output = Command::new("/opt/homebrew/bin/ffprobe")
        .args([
            "-v", "quiet", "-print_format", "json", "-show_format", "-show_streams",
            "-show_chapters",
        ])
        .arg(path)
        .output()
        .map_err(|e| {
            format!(
                "Could not run ffprobe ({e}). Make sure ffmpeg/ffprobe is installed and on your PATH."
            )
        })?;

    if !output.status.success() {
        return Err(format!(
            "ffprobe couldn't read this file: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }

    let parsed: FfprobeOutput = serde_json::from_slice(&output.stdout)
        .map_err(|e| format!("Failed to parse ffprobe output: {e}"))?;

    let mut rows = Vec::new();

    if let Ok(meta) = std::fs::metadata(path) {
        rows.push(row("File size", &human_size(meta.len())));
    }

    if let Some(format) = &parsed.format {
        let container = format
            .format_long_name
            .clone()
            .or_else(|| format.format_name.clone());
        if let Some(name) = container {
            rows.push(row("Container", &name));
        }
        if let Some(secs) = format
            .duration
            .as_deref()
            .and_then(|d| d.parse::<f64>().ok())
        {
            rows.push(row("Duration", &human_duration(secs)));
        }
        if let Some(b) = format
            .bit_rate
            .as_deref()
            .and_then(|b| b.parse::<f64>().ok())
        {
            rows.push(row("Overall bitrate", &format!("{:.0} kb/s", b / 1000.0)));
        }
    }

    let video = parsed
        .streams
        .iter()
        .find(|s| s.codec_type.as_deref() == Some("video"));
    let audio = parsed
        .streams
        .iter()
        .find(|s| s.codec_type.as_deref() == Some("audio"));

    match video {
        Some(v) => {
            let codec = v.codec_long_name.clone().or_else(|| v.codec_name.clone());
            if let Some(codec) = codec {
                rows.push(row("Video codec", &codec));
            }
            if let Some(profile) = &v.profile {
                rows.push(row("Profile", profile));
            }
            if let (Some(w), Some(h)) = (v.width, v.height) {
                rows.push(row("Resolution", &format!("{w} x {h}")));
            }
            if let Some(dar) = &v.display_aspect_ratio {
                rows.push(row("Aspect ratio", dar));
            }
            let frame_rate = v.avg_frame_rate.as_deref().or(v.r_frame_rate.as_deref());
            if let Some(fps) = frame_rate.and_then(parse_fraction) {
                rows.push(row("Frame rate", &format!("{fps:.2} fps")));
            }
            if let Some(pf) = &v.pix_fmt {
                rows.push(row("Pixel format", pf));
            }
            if let Some(bps) = &v.bits_per_raw_sample {
                rows.push(row("Bit depth", &format!("{bps}-bit")));
            }
            if let Some(b) = v.bit_rate.as_deref().and_then(|b| b.parse::<f64>().ok()) {
                rows.push(row("Video bitrate", &format!("{:.0} kb/s", b / 1000.0)));
            }
        }
        None => rows.push(row("Video", "No video stream found")),
    }

    match audio {
        Some(a) => {
            let codec = a.codec_long_name.clone().or_else(|| a.codec_name.clone());
            if let Some(codec) = codec {
                rows.push(row("Audio codec", &codec));
            }
            if let Some(sr) = &a.sample_rate {
                rows.push(row("Sample rate", &format!("{sr} Hz")));
            }
            if let Some(layout) = &a.channel_layout {
                rows.push(row("Channels", layout));
            } else if let Some(ch) = a.channels {
                rows.push(row("Channels", &ch.to_string()));
            }
            if let Some(b) = a.bit_rate.as_deref().and_then(|b| b.parse::<f64>().ok()) {
                rows.push(row("Audio bitrate", &format!("{:.0} kb/s", b / 1000.0)));
            }
        }
        None => rows.push(row("Audio", "No audio stream found")),
    }

    let subtitle_streams: Vec<&StreamInfo> = parsed
        .streams
        .iter()
        .filter(|s| s.codec_type.as_deref() == Some("subtitle"))
        .collect();

    if subtitle_streams.is_empty() {
        rows.push(row("Subtitles", "None"));
    } else {
        for (i, s) in subtitle_streams.iter().enumerate() {
            let codec = s.codec_long_name.clone().or_else(|| s.codec_name.clone());
            let lang = s.tags.get("language").cloned();
            let title = s.tags.get("title").cloned();

            let mut desc = codec.unwrap_or_else(|| "Unknown format".to_string());
            if let Some(lang) = lang {
                desc.push_str(&format!(" ({lang})"));
            }
            if let Some(title) = title {
                desc.push_str(&format!(" - {title}"));
            }

            rows.push(row(&format!("Subtitle track {}", i + 1), &desc));
        }
    }

    if !parsed.chapters.is_empty() {
        rows.push(row("Chapters", &parsed.chapters.len().to_string()));
        for (i, c) in parsed.chapters.iter().enumerate() {
            let start = c.start_time.as_deref().and_then(|s| s.parse::<f64>().ok());
            let end = c.end_time.as_deref().and_then(|s| s.parse::<f64>().ok());
            let title = c.tags.get("title").cloned();

            let range = match (start, end) {
                (Some(s), Some(e)) => format!("{} - {}", human_duration(s), human_duration(e)),
                (Some(s), None) => human_duration(s),
                _ => "Unknown timing".to_string(),
            };
            let label = title.unwrap_or_else(|| format!("Chapter {}", i + 1));

            rows.push(row(&format!("  {}", i + 1), &format!("{label} ({range})")));
        }
    }

    Ok(rows)
}

fn row(label: &str, value: &str) -> InfoRow {
    InfoRow {
        label: label.into(),
        value: value.into(),
    }
}

fn parse_fraction(s: &str) -> Option<f64> {
    let mut parts = s.split('/');
    let num: f64 = parts.next()?.parse().ok()?;
    let den: f64 = parts.next()?.parse().ok()?;
    if den == 0.0 {
        None
    } else {
        Some(num / den)
    }
}

fn human_duration(secs: f64) -> String {
    let total = secs.round() as u64;
    let h = total / 3600;
    let m = (total % 3600) / 60;
    let s = total % 60;
    if h > 0 {
        format!("{h:02}:{m:02}:{s:02}")
    } else {
        format!("{m:02}:{s:02}")
    }
}

fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut size = bytes as f64;
    let mut unit = 0;
    while size >= 1024.0 && unit < UNITS.len() - 1 {
        size /= 1024.0;
        unit += 1;
    }
    format!("{size:.2} {}", UNITS[unit])
}
