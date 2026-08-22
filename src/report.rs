//! Turning an ffprobe report into the flat label/value rows the UI shows.
//!
//! The row order here is the display order, top to bottom: file and
//! container facts, then the video and audio tracks, then subtitles and
//! chapters.

use std::path::Path;

use slint::SharedString;

use crate::InfoRow;
use crate::ffprobe::{Chapter, Report, Stream, StreamKind};

/// Builds the rows shown for `path`, whose analysis produced `report`.
pub fn rows(path: &Path, report: &Report) -> Vec<InfoRow> {
    let mut rows = Rows::default();

    // ffprobe reports the stream sizes, not the file's, so ask the OS.
    if let Ok(metadata) = std::fs::metadata(path) {
        rows.push("File size", human_size(metadata.len()));
    }

    if let Some(format) = &report.format {
        rows.push_some("Container", format.container());
        rows.push_some("Duration", format.duration_secs().map(human_duration));
        rows.push_some("Overall bitrate", format.bitrate_bps().map(human_bitrate));
    }

    video_rows(&mut rows, report.first_stream(StreamKind::Video));
    audio_rows(&mut rows, report.first_stream(StreamKind::Audio));
    subtitle_rows(&mut rows, report);
    chapter_rows(&mut rows, report);

    rows.into_vec()
}

/// The same rows flattened to `label: value` lines, for the raw-text view
/// and the Copy All button.
pub fn as_text(rows: &[InfoRow]) -> String {
    rows.iter()
        .map(|row| format!("{}: {}", row.label, row.value))
        .collect::<Vec<_>>()
        .join("\n")
}

/// A growing list of rows. `push_some` keeps the callers below to one line
/// per row, since most values are optional and are simply left out when
/// ffprobe didn't report them.
#[derive(Default)]
struct Rows(Vec<InfoRow>);

impl Rows {
    fn push(&mut self, label: impl Into<SharedString>, value: impl Into<SharedString>) {
        self.0.push(InfoRow {
            label: label.into(),
            value: value.into(),
        });
    }

    fn push_some(&mut self, label: &str, value: Option<impl Into<SharedString>>) {
        if let Some(value) = value {
            self.push(label, value);
        }
    }

    fn into_vec(self) -> Vec<InfoRow> {
        self.0
    }
}

fn video_rows(rows: &mut Rows, video: Option<&Stream>) {
    let Some(video) = video else {
        rows.push("Video", "No video stream found");
        return;
    };

    rows.push_some("Video codec", video.codec());
    rows.push_some("Profile", video.profile.as_deref());
    rows.push_some("Resolution", video.resolution());
    rows.push_some("Aspect ratio", video.display_aspect_ratio.as_deref());
    rows.push_some(
        "Frame rate",
        video.frame_rate().map(|fps| format!("{fps:.2} fps")),
    );
    rows.push_some("Pixel format", video.pix_fmt.as_deref());
    rows.push_some(
        "Bit depth",
        video
            .bits_per_raw_sample
            .as_deref()
            .map(|bits| format!("{bits}-bit")),
    );
    rows.push_some("Video bitrate", video.bitrate_bps().map(human_bitrate));
}

fn audio_rows(rows: &mut Rows, audio: Option<&Stream>) {
    let Some(audio) = audio else {
        rows.push("Audio", "No audio stream found");
        return;
    };

    rows.push_some("Audio codec", audio.codec());
    rows.push_some(
        "Sample rate",
        audio.sample_rate.as_deref().map(|hz| format!("{hz} Hz")),
    );
    // The layout ("5.1") is more useful than the bare count, so it wins
    // when ffprobe reports both.
    rows.push_some(
        "Channels",
        audio
            .channel_layout
            .clone()
            .or_else(|| audio.channels.map(|count| count.to_string())),
    );
    rows.push_some("Audio bitrate", audio.bitrate_bps().map(human_bitrate));
}

fn subtitle_rows(rows: &mut Rows, report: &Report) {
    let subtitles: Vec<&Stream> = report.streams_of(StreamKind::Subtitle).collect();
    if subtitles.is_empty() {
        rows.push("Subtitles", "None");
        return;
    }

    for (index, subtitle) in subtitles.iter().enumerate() {
        rows.push(
            format!("Subtitle track {}", index + 1),
            describe_subtitle(subtitle),
        );
    }
}

/// e.g. `SubRip subtitle (eng) - Forced`, dropping whichever parts the
/// file doesn't carry.
fn describe_subtitle(subtitle: &Stream) -> String {
    let mut description = subtitle.codec().unwrap_or("Unknown format").to_string();
    if let Some(language) = subtitle.tag("language") {
        description.push_str(&format!(" ({language})"));
    }
    if let Some(title) = subtitle.tag("title") {
        description.push_str(&format!(" - {title}"));
    }
    description
}

fn chapter_rows(rows: &mut Rows, report: &Report) {
    if report.chapters.is_empty() {
        return;
    }

    rows.push("Chapters", report.chapters.len().to_string());
    for (index, chapter) in report.chapters.iter().enumerate() {
        let number = index + 1;
        let title = chapter
            .title()
            .map(str::to_string)
            .unwrap_or_else(|| format!("Chapter {number}"));
        // The leading spaces indent each chapter under the count above --
        // in the raw-text view as well as in the row list.
        rows.push(
            format!("  {number}"),
            format!("{title} ({})", chapter_range(chapter)),
        );
    }
}

fn chapter_range(chapter: &Chapter) -> String {
    match (chapter.start_secs(), chapter.end_secs()) {
        (Some(start), Some(end)) => format!("{} - {}", human_duration(start), human_duration(end)),
        (Some(start), None) => human_duration(start),
        _ => "Unknown timing".to_string(),
    }
}

/// `HH:MM:SS`, dropping the hours for anything under an hour.
fn human_duration(secs: f64) -> String {
    let total = secs.round() as u64;
    let (hours, minutes, secs) = (total / 3600, (total % 3600) / 60, total % 60);
    if hours > 0 {
        format!("{hours:02}:{minutes:02}:{secs:02}")
    } else {
        format!("{minutes:02}:{secs:02}")
    }
}

/// The largest binary unit that keeps the number readable, e.g. `1.44 GB`.
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

/// ffprobe reports bitrates in bits per second; kb/s is what people
/// reading them expect.
fn human_bitrate(bits_per_sec: f64) -> String {
    format!("{:.0} kb/s", bits_per_sec / 1000.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_durations() {
        assert_eq!(human_duration(0.0), "00:00");
        assert_eq!(human_duration(61.4), "01:01");
        assert_eq!(human_duration(3599.6), "01:00:00");
        assert_eq!(human_duration(7284.0), "02:01:24");
    }

    #[test]
    fn formats_sizes() {
        assert_eq!(human_size(0), "0.00 B");
        assert_eq!(human_size(1023), "1023.00 B");
        assert_eq!(human_size(1024), "1.00 KB");
        assert_eq!(human_size(1_610_612_736), "1.50 GB");
    }

    #[test]
    fn formats_bitrates() {
        assert_eq!(human_bitrate(0.0), "0 kb/s");
        assert_eq!(human_bitrate(5_432_100.0), "5432 kb/s");
    }
}
