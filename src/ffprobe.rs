//! Running `ffprobe` and deserializing its JSON report.
//!
//! Only the fields this app displays are modelled. Every value ffprobe
//! reports is optional in practice, so the structs mirror that and the
//! accessors below hand back `None` for anything missing or unparsable.

use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Deserialize;

/// Why an analysis didn't produce a report. The `Display` text is what the
/// UI shows the user.
#[derive(Debug)]
pub enum Error {
    /// ffprobe couldn't be started at all: not installed, not on PATH, or
    /// not executable.
    Spawn(std::io::Error),
    /// ffprobe ran but rejected the file; carries its stderr.
    Rejected(String),
    /// ffprobe succeeded but its JSON wasn't the shape we expect.
    Parse(serde_json::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Spawn(err) => write!(
                f,
                "Could not run ffprobe ({err}). Make sure ffmpeg/ffprobe is installed and on your PATH."
            ),
            Self::Rejected(stderr) => write!(f, "ffprobe couldn't read this file: {stderr}"),
            Self::Parse(err) => write!(f, "Failed to parse ffprobe output: {err}"),
        }
    }
}

impl std::error::Error for Error {}

/// The kinds of stream this app reports on, i.e. the `codec_type` values
/// it recognizes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamKind {
    Video,
    Audio,
    Subtitle,
}

impl StreamKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Video => "video",
            Self::Audio => "audio",
            Self::Subtitle => "subtitle",
        }
    }
}

/// ffprobe's `-print_format json` output.
#[derive(Debug, Deserialize)]
pub struct Report {
    #[serde(default)]
    pub streams: Vec<Stream>,
    pub format: Option<Format>,
    #[serde(default)]
    pub chapters: Vec<Chapter>,
}

impl Report {
    /// Every stream of the given kind, in file order.
    pub fn streams_of(&self, kind: StreamKind) -> impl Iterator<Item = &Stream> {
        self.streams
            .iter()
            .filter(move |stream| stream.codec_type.as_deref() == Some(kind.as_str()))
    }

    /// The first stream of the given kind -- the one the UI treats as
    /// *the* video/audio track.
    pub fn first_stream(&self, kind: StreamKind) -> Option<&Stream> {
        self.streams_of(kind).next()
    }
}

/// One entry of ffprobe's `-show_streams`.
#[derive(Debug, Deserialize)]
pub struct Stream {
    pub codec_type: Option<String>,
    pub codec_name: Option<String>,
    pub codec_long_name: Option<String>,
    pub profile: Option<String>,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub pix_fmt: Option<String>,
    pub r_frame_rate: Option<String>,
    pub avg_frame_rate: Option<String>,
    pub bit_rate: Option<String>,
    pub sample_rate: Option<String>,
    pub channels: Option<i64>,
    pub channel_layout: Option<String>,
    pub display_aspect_ratio: Option<String>,
    pub bits_per_raw_sample: Option<String>,
    #[serde(default)]
    pub tags: HashMap<String, String>,
}

impl Stream {
    /// The codec's name, preferring ffprobe's descriptive long form
    /// ("H.264 / AVC / ...") over its short one ("h264").
    pub fn codec(&self) -> Option<&str> {
        self.codec_long_name
            .as_deref()
            .or(self.codec_name.as_deref())
    }

    /// `"1920 x 1080"`, when the stream has both dimensions.
    pub fn resolution(&self) -> Option<String> {
        let (width, height) = (self.width?, self.height?);
        Some(format!("{width} x {height}"))
    }

    /// Frames per second, preferring the average rate (which reflects the
    /// whole file) over the nominal one.
    pub fn frame_rate(&self) -> Option<f64> {
        self.avg_frame_rate
            .as_deref()
            .or(self.r_frame_rate.as_deref())
            .and_then(parse_fraction)
    }

    pub fn bitrate_bps(&self) -> Option<f64> {
        number(&self.bit_rate)
    }

    /// A metadata tag such as `language` or `title`.
    pub fn tag(&self, name: &str) -> Option<&str> {
        self.tags.get(name).map(String::as_str)
    }
}

/// ffprobe's `-show_format`: what the container itself says.
#[derive(Debug, Deserialize)]
pub struct Format {
    pub format_name: Option<String>,
    pub format_long_name: Option<String>,
    pub duration: Option<String>,
    pub bit_rate: Option<String>,
}

impl Format {
    /// The container's name, preferring the long form ("QuickTime / MOV")
    /// over the short one ("mov,mp4,m4a,3gp,3g2,mj2").
    pub fn container(&self) -> Option<&str> {
        self.format_long_name
            .as_deref()
            .or(self.format_name.as_deref())
    }

    pub fn duration_secs(&self) -> Option<f64> {
        number(&self.duration)
    }

    pub fn bitrate_bps(&self) -> Option<f64> {
        number(&self.bit_rate)
    }
}

/// One entry of ffprobe's `-show_chapters`.
#[derive(Debug, Deserialize)]
pub struct Chapter {
    pub start_time: Option<String>,
    pub end_time: Option<String>,
    #[serde(default)]
    pub tags: HashMap<String, String>,
}

impl Chapter {
    pub fn start_secs(&self) -> Option<f64> {
        number(&self.start_time)
    }

    pub fn end_secs(&self) -> Option<f64> {
        number(&self.end_time)
    }

    pub fn title(&self) -> Option<&str> {
        self.tags.get("title").map(String::as_str)
    }
}

/// Analyzes `path` with ffprobe and returns its report.
pub fn run(path: &Path) -> Result<Report, Error> {
    let mut command = Command::new(locate_ffprobe());
    command
        .args([
            // "error" (rather than "quiet") so failures come back with a
            // reason instead of an empty message.
            "-v",
            "error",
            "-print_format",
            "json",
            "-show_format",
            "-show_streams",
            "-show_chapters",
        ])
        .arg(path);

    // Keep ffprobe's console window off the screen: ffprobe.exe is a
    // console app, and spawning one from a windowed (console-less) app
    // otherwise briefly pops up a new console window.
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        command.creation_flags(CREATE_NO_WINDOW);
    }

    let output = command.output().map_err(Error::Spawn)?;
    if !output.status.success() {
        return Err(Error::Rejected(
            String::from_utf8_lossy(&output.stderr).trim().to_string(),
        ));
    }

    serde_json::from_slice(&output.stdout).map_err(Error::Parse)
}

/// Finds the `ffprobe` binary to run: prefer the copy bundled next to this
/// app's own executable (see `external-binaries` in Cargo.toml, put there
/// by `cargo packager`), falling back to the bare name -- and so to PATH
/// -- for dev builds run via `cargo run`, or when the user has ffmpeg
/// installed system-wide.
fn locate_ffprobe() -> PathBuf {
    let exe_name = if cfg!(windows) {
        "ffprobe.exe"
    } else {
        "ffprobe"
    };

    let bundled = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join(exe_name)));
    match bundled {
        Some(bundled) if bundled.is_file() => bundled,
        _ => {
            eprintln!("warning: could not find bundled ffprobe, using ffprobe from PATH");
            PathBuf::from(exe_name)
        }
    }
}

/// ffprobe reports numbers as JSON strings, and writes "N/A" where a value
/// is unknown -- so anything that doesn't parse is treated as absent.
fn number(field: &Option<String>) -> Option<f64> {
    field.as_deref()?.trim().parse().ok()
}

/// Parses ffprobe's `"num/den"` rate strings, e.g. `"30000/1001"`.
/// Rates it doesn't know are reported as `"0/0"`, hence the zero check.
fn parse_fraction(fraction: &str) -> Option<f64> {
    let (numerator, denominator) = fraction.split_once('/')?;
    let numerator: f64 = numerator.parse().ok()?;
    let denominator: f64 = denominator.parse().ok()?;
    (denominator != 0.0).then(|| numerator / denominator)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_fractional_frame_rates() {
        assert_eq!(parse_fraction("30/1"), Some(30.0));
        assert_eq!(parse_fraction("30000/1001"), Some(29.97002997002997));
    }

    #[test]
    fn rejects_unusable_fractions() {
        assert_eq!(parse_fraction("0/0"), None);
        assert_eq!(parse_fraction("30"), None);
        assert_eq!(parse_fraction(""), None);
    }

    #[test]
    fn treats_unparsable_numbers_as_absent() {
        assert_eq!(number(&Some("1234.5".to_string())), Some(1234.5));
        assert_eq!(number(&Some("N/A".to_string())), None);
        assert_eq!(number(&None), None);
    }
}
