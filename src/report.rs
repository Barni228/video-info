//! Turning an ffprobe report into the flat label/value rows the UI shows.
//!
//! The row order here is the display order, top to bottom: file and
//! container facts, then the video and audio tracks, then subtitles and
//! chapters.
//!
//! Which of those sections appear depends on what the file turned out to
//! be -- see [`Media`]. Reporting an absence is only worth a row where
//! the thing could plausibly have been there: "No audio stream" says
//! something about a movie, and nothing at all about a GIF.

use std::path::Path;

use slint::SharedString;

use crate::InfoRow;
use crate::ffprobe::{Chapter, Report, Stream, StreamKind};

/// What the file turned out to be, which is what decides the sections
/// worth showing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Media {
    /// Moving video, with or without a sound track: the full report.
    Movie,
    /// Sound only. Any picture in it is cover art, not a video track.
    Audio,
    /// A picture format -- a still image, or a silent animation like a
    /// GIF. It has frames and nothing else. `animated` separates the two:
    /// a run of frames has a duration and a rate, a single still doesn't.
    Image { animated: bool },
}

impl Media {
    fn of(report: &Report) -> Self {
        let image_container = report.format.as_ref().is_some_and(|format| format.is_image());
        let video = report.video_stream();
        if image_container {
            return Self::Image {
                animated: video.and_then(animated_frame_count).is_some(),
            };
        }
        match video {
            Some(_) => Self::Movie,
            // No moving picture and not a picture file: whatever sound
            // track got this far is the whole story.
            None => Self::Audio,
        }
    }

    fn is_image(self) -> bool {
        matches!(self, Self::Image { .. })
    }

    /// A single frame, so anything measured over time -- duration, frame
    /// rate, bitrate -- is either absent or invented.
    fn is_still_image(self) -> bool {
        self == Self::Image { animated: false }
    }
}

/// Builds the rows shown for `path`, whose analysis produced `report`.
pub fn rows(path: &Path, report: &Report) -> Vec<InfoRow> {
    let mut rows = Rows::default();
    let media = Media::of(report);

    // ffprobe reports the stream sizes, not the file's, so ask the OS.
    if let Ok(metadata) = std::fs::metadata(path) {
        rows.push("File size", human_size(metadata.len()));
    }

    if let Some(format) = &report.format {
        rows.push_some("Container", format.container());
        // A single still has no duration to speak of; ffprobe reports one
        // anyway ("00:00"), along with a bitrate derived from it.
        if !media.is_still_image() {
            rows.push_some("Duration", format.duration_secs().map(human_duration));
            rows.push_some("Overall bitrate", format.bitrate_bps().map(human_bitrate));
        }
    }

    picture_rows(&mut rows, report, media);
    audio_rows(&mut rows, report, media);
    subtitle_rows(&mut rows, report, media);
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

/// The moving-picture section. For an audio file this is skipped
/// entirely -- bar a line for its cover art, which is the one picture
/// worth mentioning there.
fn picture_rows(rows: &mut Rows, report: &Report, media: Media) {
    if media == Media::Audio {
        if let Some(cover) = report.cover_art() {
            rows.push_some("Cover art", describe_cover_art(cover));
        }
        return;
    }

    let Some(video) = report.video_stream() else {
        // Only a movie can be missing its video: a picture file that got
        // this far has frames by definition.
        rows.push("Video", "No video stream found");
        return;
    };

    // A still image has no video track to speak of, so its frames are
    // labelled for what they are.
    let codec_label = if media.is_image() {
        "Image format"
    } else {
        "Video codec"
    };
    rows.push_some(codec_label, video.codec());
    rows.push_some("Profile", video.profile.as_deref());
    rows.push_some("Resolution", video.resolution());
    rows.push_some("Aspect ratio", video.display_aspect_ratio.as_deref());
    if media.is_image() {
        rows.push_some("Frames", animated_frame_count(video).map(|n| n.to_string()));
    }
    // ffprobe invents a nominal 25 fps for a single still, so the rate is
    // only worth showing once there is something to animate.
    if !media.is_still_image() {
        rows.push_some(
            "Frame rate",
            video.frame_rate().map(|fps| format!("{fps:.2} fps")),
        );
    }
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

/// The frame count of a stream that actually moves, or `None` for a
/// single still.
fn animated_frame_count(video: &Stream) -> Option<i64> {
    video.frame_count().filter(|&frames| frames > 1)
}

/// e.g. `600 x 600 (MJPEG)`, for the picture embedded in an audio file.
fn describe_cover_art(cover: &Stream) -> Option<String> {
    match (cover.resolution(), cover.codec()) {
        (Some(resolution), Some(codec)) => Some(format!("{resolution} ({codec})")),
        (Some(resolution), None) => Some(resolution),
        (None, codec) => codec.map(str::to_string),
    }
}

fn audio_rows(rows: &mut Rows, report: &Report, media: Media) {
    let Some(audio) = report.first_stream(StreamKind::Audio) else {
        // A picture format has no sound track to be missing.
        if media == Media::Movie {
            rows.push("Audio", "No audio stream found");
        }
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

fn subtitle_rows(rows: &mut Rows, report: &Report, media: Media) {
    let subtitles: Vec<&Stream> = report.streams_of(StreamKind::Subtitle).collect();
    if subtitles.is_empty() {
        // Worth saying for a movie, where subtitles might have been
        // expected. An audio file or a GIF can't carry them at all.
        if media == Media::Movie {
            rows.push("Subtitles", "None");
        }
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

    /// Builds a report from ffprobe-shaped JSON, so these tests exercise
    /// the same deserialization the real thing goes through.
    fn report(json: serde_json::Value) -> Report {
        serde_json::from_value(json).expect("test report should deserialize")
    }

    fn labels(report: &Report) -> Vec<String> {
        let media = Media::of(report);
        let mut rows = Rows::default();
        picture_rows(&mut rows, report, media);
        audio_rows(&mut rows, report, media);
        subtitle_rows(&mut rows, report, media);
        rows.into_vec()
            .into_iter()
            .map(|row| row.label.to_string())
            .collect()
    }

    fn value_of(report: &Report, label: &str) -> Option<String> {
        let media = Media::of(report);
        let mut rows = Rows::default();
        picture_rows(&mut rows, report, media);
        audio_rows(&mut rows, report, media);
        rows.into_vec()
            .into_iter()
            .find(|row| row.label == label)
            .map(|row| row.value.to_string())
    }

    #[test]
    fn a_movie_reports_what_it_is_missing() {
        let movie = report(serde_json::json!({
            "format": { "format_name": "mov,mp4,m4a,3gp,3g2,mj2" },
            "streams": [{ "codec_type": "video", "codec_name": "h264" }],
        }));
        assert_eq!(Media::of(&movie), Media::Movie);
        let labels = labels(&movie);
        assert!(labels.contains(&"Audio".to_string()));
        assert!(labels.contains(&"Subtitles".to_string()));
    }

    #[test]
    fn a_gif_is_not_asked_about_sound_or_subtitles() {
        let gif = report(serde_json::json!({
            "format": { "format_name": "gif" },
            "streams": [{
                "codec_type": "video", "codec_name": "gif", "nb_frames": "30",
            }],
        }));
        assert_eq!(Media::of(&gif), Media::Image { animated: true });
        let labels = labels(&gif);
        assert!(!labels.contains(&"Audio".to_string()));
        assert!(!labels.contains(&"Subtitles".to_string()));
        // It does move, so its rate is worth reporting.
        assert!(labels.contains(&"Frames".to_string()));
    }

    #[test]
    fn a_still_image_has_no_invented_frame_rate() {
        let still = report(serde_json::json!({
            "format": { "format_name": "png_pipe" },
            "streams": [{
                "codec_type": "video", "codec_name": "png", "r_frame_rate": "25/1",
            }],
        }));
        assert_eq!(Media::of(&still), Media::Image { animated: false });
        let labels = labels(&still);
        assert!(!labels.contains(&"Frame rate".to_string()));
        assert!(!labels.contains(&"Frames".to_string()));
        assert!(labels.contains(&"Image format".to_string()));
    }

    #[test]
    fn an_audio_file_is_not_asked_about_video_or_subtitles() {
        let audio = report(serde_json::json!({
            "format": { "format_name": "mp3" },
            "streams": [{ "codec_type": "audio", "codec_name": "mp3" }],
        }));
        assert_eq!(Media::of(&audio), Media::Audio);
        let labels = labels(&audio);
        assert!(!labels.contains(&"Video".to_string()));
        assert!(!labels.contains(&"Subtitles".to_string()));
        assert!(labels.contains(&"Audio codec".to_string()));
    }

    #[test]
    fn cover_art_is_a_picture_not_a_video_track() {
        let with_cover = report(serde_json::json!({
            "format": { "format_name": "mp3" },
            "streams": [
                { "codec_type": "audio", "codec_name": "mp3" },
                {
                    "codec_type": "video", "codec_name": "mjpeg",
                    "width": 600, "height": 600, "r_frame_rate": "90000/1",
                    "disposition": { "attached_pic": 1 },
                },
            ],
        }));
        assert_eq!(Media::of(&with_cover), Media::Audio);
        let labels = labels(&with_cover);
        assert!(!labels.contains(&"Video codec".to_string()));
        assert!(!labels.contains(&"Frame rate".to_string()));
        assert_eq!(
            value_of(&with_cover, "Cover art").as_deref(),
            Some("600 x 600 (mjpeg)")
        );
    }

    #[test]
    fn subtitles_are_listed_wherever_they_actually_exist() {
        let subtitled = report(serde_json::json!({
            "format": { "format_name": "matroska,webm" },
            "streams": [
                { "codec_type": "video", "codec_name": "h264" },
                {
                    "codec_type": "subtitle", "codec_long_name": "SubRip subtitle",
                    "tags": { "language": "eng" },
                },
            ],
        }));
        let labels = labels(&subtitled);
        assert!(labels.contains(&"Subtitle track 1".to_string()));
        assert!(!labels.contains(&"Subtitles".to_string()));
    }

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

