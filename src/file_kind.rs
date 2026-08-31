//! What a file's name suggests it holds.
//!
//! Only ever a guess -- ffprobe is what actually decides whether a file is
//! readable. It is used for the file picker's filters, and to word errors:
//! a `.mp4` ffprobe rejects is a damaged video, whereas a `.pdf` it
//! rejects was simply never video in the first place.

use std::path::Path;

/// Extensions the file picker offers to filter by. Deliberately a superset
/// of the `file-associations` in Cargo.toml (which decide what the app
/// registers itself as an "Open With" handler for): the picker can list
/// formats without claiming to own them.
pub const VIDEO_EXTENSIONS: &[&str] = &[
    "mp4", "mkv", "mov", "avi", "webm", "flv", "wmv", "m4v", "mpg", "mpeg", "ts", "3gp", "ogv",
    "gif",
];
pub const AUDIO_EXTENSIONS: &[&str] = &[
    "mp3", "wav", "flac", "aac", "m4a", "ogg", "opus", "wma", "aiff",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileKind {
    Video,
    Audio,
    /// An extension this app doesn't associate with media -- or none at
    /// all. Plenty of readable files land here; it only means the name
    /// gives nothing away.
    Unknown,
}

impl FileKind {
    /// How to refer to the file in a sentence, e.g. "This **video file**
    /// appears to be damaged."
    pub fn describe(self) -> &'static str {
        match self {
            Self::Video => "video file",
            Self::Audio => "audio file",
            Self::Unknown => "file",
        }
    }
}

/// What `path`'s extension suggests it is.
pub fn of(path: &Path) -> FileKind {
    match extension(path) {
        Some(ext) if VIDEO_EXTENSIONS.contains(&ext.as_str()) => FileKind::Video,
        Some(ext) if AUDIO_EXTENSIONS.contains(&ext.as_str()) => FileKind::Audio,
        _ => FileKind::Unknown,
    }
}

/// `path`'s extension, lowercased, without the dot.
pub fn extension(path: &Path) -> Option<String> {
    let ext = path.extension()?.to_string_lossy().to_lowercase();
    (!ext.is_empty()).then_some(ext)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn recognizes_media_extensions_case_insensitively() {
        assert_eq!(of(&PathBuf::from("clip.MP4")), FileKind::Video);
        assert_eq!(of(&PathBuf::from("song.flac")), FileKind::Audio);
    }

    #[test]
    fn treats_anything_else_as_unknown() {
        assert_eq!(of(&PathBuf::from("paper.pdf")), FileKind::Unknown);
        assert_eq!(of(&PathBuf::from("README")), FileKind::Unknown);
    }

    #[test]
    fn extracts_lowercased_extensions() {
        assert_eq!(extension(&PathBuf::from("a/b/Clip.MOV")), Some("mov".into()));
        assert_eq!(extension(&PathBuf::from("noext")), None);
    }
}
