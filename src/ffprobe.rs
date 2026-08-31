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

use crate::file_kind::{self, FileKind};

/// Why an analysis didn't produce a report.
///
/// Each variant is a distinct thing that can go wrong, so the UI can say
/// something specific rather than passing ffprobe's own wording along as
/// the whole explanation: [`headline`](Error::headline) is the plain
/// sentence shown to the user, and [`detail`](Error::detail) is the
/// supporting text under it -- ffprobe's own words, where it had any.
#[derive(Debug)]
pub enum Error {
    /// Nothing is at that path (any more -- a dropped file can be moved
    /// or deleted between the drop and the probe).
    NotFound,
    /// The path is a directory, not a file.
    IsDirectory,
    /// The file is zero bytes.
    Empty,
    /// The file exists but this user can't read it.
    PermissionDenied,
    /// The file couldn't be examined for some other OS-level reason.
    Unreadable(std::io::Error),
    /// ffprobe couldn't be started at all: not installed, not on PATH, or
    /// not executable.
    NotInstalled(std::io::Error),
    /// ffprobe rejected the file, and its name doesn't suggest media
    /// either -- so it most likely never was a video or audio file.
    NotMedia {
        extension: Option<String>,
        detail: String,
    },
    /// ffprobe rejected a file whose name says it should be readable, so
    /// the contents are the problem: truncated, corrupt, or unfinished.
    Damaged { kind: FileKind, detail: String },
    /// ffprobe rejected the file for a reason not recognized above.
    Rejected(String),
    /// ffprobe read the file but found nothing to describe.
    NoTracks,
    /// ffprobe succeeded but its JSON wasn't the shape we expect.
    Parse(serde_json::Error),
}

impl Error {
    /// The plain-language line naming what went wrong. Always ends in a
    /// full stop -- it is a sentence on its own.
    pub fn headline(&self) -> String {
        match self {
            Self::NotFound => "That file isn't there any more.".into(),
            Self::IsDirectory => "That's a folder, not a file.".into(),
            Self::Empty => "This file is empty.".into(),
            Self::PermissionDenied => "No permission to read this file.".into(),
            Self::Unreadable(_) => "This file can't be opened.".into(),
            Self::NotInstalled(_) => "ffprobe couldn't be run.".into(),
            Self::NotMedia { extension, .. } => match extension {
                Some(ext) => format!("Unsupported file type: .{ext} isn't video or audio."),
                None => "Unsupported file type: this isn't video or audio.".into(),
            },
            Self::Damaged { kind, .. } => format!("This {} is damaged.", kind.describe()),
            Self::Rejected(_) => "ffprobe couldn't read this file.".into(),
            Self::NoTracks => "This file has no video or audio tracks.".into(),
            Self::Parse(_) => "ffprobe's output couldn't be understood.".into(),
        }
    }

    /// The supporting line shown under the headline: ffprobe's own error
    /// text where there is one, and an explanation where there isn't.
    /// `None` when the headline already says everything.
    pub fn detail(&self) -> Option<String> {
        match self {
            Self::NotFound => Some("It may have been moved, renamed, or deleted.".into()),
            Self::IsDirectory => Some("Open a single video or audio file instead.".into()),
            Self::Empty => Some("It's 0 bytes, so there's nothing to read.".into()),
            Self::PermissionDenied => {
                Some("Check the file's permissions, or copy it somewhere you can read.".into())
            }
            Self::Unreadable(err) => Some(err.to_string()),
            Self::NotInstalled(err) => Some(format!(
                "Install ffmpeg and make sure ffprobe is on your PATH. ({err})"
            )),
            Self::NotMedia { detail, .. } | Self::Damaged { detail, .. } => {
                Some(detail.clone()).filter(|detail| !detail.is_empty())
            }
            Self::Rejected(detail) => Some(detail.clone()).filter(|detail| !detail.is_empty()),
            Self::NoTracks => {
                Some("ffprobe opened it, but it contains no streams to describe.".into())
            }
            Self::Parse(err) => Some(err.to_string()),
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.detail() {
            Some(detail) => write!(f, "{} {detail}", self.headline()),
            None => write!(f, "{}", self.headline()),
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

/// A successful analysis: the report, plus anything ffprobe complained
/// about while reading the file. A file can be damaged enough to produce
/// warnings and still yield a usable report, so those complaints are
/// carried alongside it rather than thrown away.
#[derive(Debug)]
pub struct Probe {
    pub report: Report,
    pub warning: Option<String>,
}

/// Analyzes `path` with ffprobe and returns its report.
pub fn run(path: &Path) -> Result<Probe, Error> {
    check_readable(path)?;

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

    let output = command.output().map_err(Error::NotInstalled)?;
    let stderr = tidy_stderr(&String::from_utf8_lossy(&output.stderr), path);

    if !output.status.success() {
        return Err(classify_rejection(path, &stderr));
    }

    let report: Report = serde_json::from_slice(&output.stdout).map_err(Error::Parse)?;
    if report.streams.is_empty() && report.chapters.is_empty() {
        return Err(Error::NoTracks);
    }

    // ffprobe exited cleanly, but a file that is truncated or subtly
    // corrupt still gets read -- it just grumbles on the way through, and
    // what it managed to report may be wrong.
    Ok(Probe {
        report,
        warning: (!stderr.is_empty()).then_some(stderr),
    })
}

/// The checks worth making before spending a process on the file, so the
/// everyday mistakes (a folder, a deleted file, an unreadable one) get a
/// precise answer rather than whatever ffprobe would have said.
fn check_readable(path: &Path) -> Result<(), Error> {
    let metadata = std::fs::metadata(path).map_err(|err| match err.kind() {
        std::io::ErrorKind::NotFound => Error::NotFound,
        std::io::ErrorKind::PermissionDenied => Error::PermissionDenied,
        _ => Error::Unreadable(err),
    })?;

    if metadata.is_dir() {
        return Err(Error::IsDirectory);
    }
    if metadata.len() == 0 {
        return Err(Error::Empty);
    }
    Ok(())
}

/// Works out which [`Error`] a non-zero ffprobe exit means, from what it
/// wrote to stderr and what the file's name suggests it should have been.
fn classify_rejection(path: &Path, stderr: &str) -> Error {
    let lower = stderr.to_lowercase();
    let says = |needle: &str| lower.contains(needle);

    // The OS-level failures, in case the file changed underneath us
    // between check_readable and ffprobe opening it.
    if says("no such file") {
        return Error::NotFound;
    }
    if says("is a directory") {
        return Error::IsDirectory;
    }
    if says("permission denied") {
        return Error::PermissionDenied;
    }

    // ffprobe's vocabulary for "I opened it, but the bytes aren't what
    // they claim to be". "Invalid data found when processing input" is
    // its catch-all, and covers both a genuinely broken video and a file
    // that was never video at all -- the extension is what separates the
    // two.
    const UNREADABLE_CONTENT: &[&str] = &[
        "invalid data found",
        "moov atom not found",
        "end of file",
        "truncat",
        "partial file",
        "could not find codec parameters",
        "invalid argument",
        "header missing",
        "corrupt",
    ];
    if UNREADABLE_CONTENT.iter().any(|marker| says(marker)) {
        let kind = file_kind::of(path);
        return if kind == FileKind::Unknown {
            Error::NotMedia {
                extension: file_kind::extension(path),
                detail: stderr.to_string(),
            }
        } else {
            Error::Damaged {
                kind,
                detail: stderr.to_string(),
            }
        };
    }

    Error::Rejected(stderr.to_string())
}

/// Makes ffprobe's stderr fit to show a person: it prefixes most lines
/// with the internal demuxer that produced them (`[mov,mp4,... @ 0x7f..]`)
/// and names the file in full, neither of which means anything to the
/// user. Repeated lines (a broken stream can produce hundreds) collapse,
/// and only the first few survive.
fn tidy_stderr(stderr: &str, path: &Path) -> String {
    const MAX_LINES: usize = 4;

    let file_prefix = format!("{}: ", path.display());
    let mut lines: Vec<&str> = Vec::new();

    for line in stderr.lines() {
        let line = line.trim();
        // Drop the "[demuxer @ 0xaddress] " prefix, keeping the message.
        let line = match line.strip_prefix('[').and_then(|rest| rest.split_once("] ")) {
            Some((tag, message)) if tag.contains(" @ ") => message,
            _ => line,
        };
        let line = line.strip_prefix(&file_prefix).unwrap_or(line);

        if !line.is_empty() && !lines.contains(&line) {
            lines.push(line);
        }
    }

    let truncated = lines.len() > MAX_LINES;
    lines.truncate(MAX_LINES);
    let mut tidied = lines.join("\n");
    if truncated {
        tidied.push_str("\n...");
    }
    tidied
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
    use std::path::PathBuf;

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
    fn strips_demuxer_tags_and_paths_from_stderr() {
        let path = PathBuf::from("/Users/me/clips/holiday.mp4");
        let stderr = "[mov,mp4,m4a,3gp,3g2,mj2 @ 0x914c30000] moov atom not found\n\
                      /Users/me/clips/holiday.mp4: Invalid data found when processing input\n";
        assert_eq!(
            tidy_stderr(stderr, &path),
            "moov atom not found\nInvalid data found when processing input"
        );
    }

    #[test]
    fn collapses_repeats_and_caps_stderr_length() {
        let path = PathBuf::from("clip.mp4");
        let repeated = "[h264 @ 0x1] Invalid NAL unit size\n".repeat(20);
        assert_eq!(tidy_stderr(&repeated, &path), "Invalid NAL unit size");

        let many: String = (0..10).map(|n| format!("problem {n}\n")).collect();
        assert_eq!(
            tidy_stderr(&many, &path),
            "problem 0\nproblem 1\nproblem 2\nproblem 3\n..."
        );
    }

    #[test]
    fn blames_the_contents_of_a_file_that_should_have_been_readable() {
        let error = classify_rejection(
            &PathBuf::from("holiday.mp4"),
            "moov atom not found\nInvalid data found when processing input",
        );
        assert!(matches!(
            error,
            Error::Damaged {
                kind: FileKind::Video,
                ..
            }
        ));
        assert_eq!(error.headline(), "This video file is damaged.");
    }

    #[test]
    fn blames_the_file_type_when_the_name_never_promised_media() {
        let error = classify_rejection(
            &PathBuf::from("paper.pdf"),
            "Invalid data found when processing input",
        );
        assert_eq!(
            error.headline(),
            "Unsupported file type: .pdf isn't video or audio."
        );
    }

    #[test]
    fn recognizes_os_level_failures_in_ffprobes_own_words() {
        let path = PathBuf::from("gone.mp4");
        assert!(matches!(
            classify_rejection(&path, "No such file or directory"),
            Error::NotFound
        ));
        assert!(matches!(
            classify_rejection(&path, "Permission denied"),
            Error::PermissionDenied
        ));
    }

    #[test]
    fn falls_back_to_ffprobes_wording_when_the_reason_is_unfamiliar() {
        let error = classify_rejection(&PathBuf::from("clip.mp4"), "Something else went wrong");
        assert_eq!(error.headline(), "ffprobe couldn't read this file.");
        assert_eq!(error.detail().as_deref(), Some("Something else went wrong"));
    }

    #[test]
    fn treats_unparsable_numbers_as_absent() {
        assert_eq!(number(&Some("1234.5".to_string())), Some(1234.5));
        assert_eq!(number(&Some("N/A".to_string())), None);
        assert_eq!(number(&None), None);
    }
}

