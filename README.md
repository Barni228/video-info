# Video Info

A small, modern alternative to MediaInfo. Video Info is a lightweight
desktop app that shows you the technical metadata of a video or audio
file: container format, video/audio/subtitle codecs, resolution, bitrate,
frame rate, duration, file size, and chapters.

It's cross-platform (macOS and Windows), built in Rust with
[Slint](https://slint.rs/) for the UI, and uses `ffprobe` under the
hood to read the file.

## Features

- Drag and drop a file onto the window, or use the native file picker
- Registers as an "Open With" handler for common video files (mp4, mkv,
  mov, avi, webm, m4v, mpg, mpeg, flv, wmv, gif) and audio files (mp3,
  wav, flac, aac, m4a, ogg, opus, wma, aiff)
- Container, video/audio/subtitle stream, and chapter details
- Shows only what's relevant to the file at hand — an audio file isn't
  asked about subtitles, a GIF isn't asked about its sound track, and
  album art is reported as cover art rather than as a video track
- Clear errors when a file can't be read: an unsupported file type, a
  damaged video, a folder, or an empty file each say so plainly, with
  `ffprobe`'s own words underneath
- Raw text view, and a Copy All button for pasting the report elsewhere
- Light, Dark, and System themes (System = automatically decide)
- Zoom controls (`Cmd`/`Ctrl` `+`/`-`/`0`) to scale the UI
- Tells you when a new release is out (it never installs anything
  itself — see [Updates](#updates))
- Analysis runs in the background so the UI never blocks

## Requirements

Released builds bundle `ffprobe` (part of [FFmpeg](https://ffmpeg.org/)),
so no separate install is needed. If you build from source and skip the
packaging step (`cargo run`), `ffprobe` must be installed and on your
`PATH` instead.

## Installation

Download the latest build for your platform from the
[GitHub Releases](../../releases) page (`.dmg` for macOS, `.exe`/`.msi`
for Windows).

> **macOS builds are Apple Silicon only.** The released `.dmg` won't run
> on an Intel Mac. Build from source on an Intel machine instead (see
> [Building from source](#building-from-source)).

### "App is damaged and can't be opened" (downloaded builds)

The `.dmg`/`.app` files on GitHub Releases aren't signed with a paid Apple
Developer ID or notarized by Apple. macOS adds `com.apple.quarantine` to
browser downloads, so Gatekeeper may show "is damaged and can't be opened".
The app isn't actually broken.

To run it anyway:

```sh
xattr -cr "/Applications/Video Info.app"
```

(or wherever you moved it), then open it normally.

## Settings

Open the Settings window from the View menu, or with `Cmd`/`Ctrl` + `,`.
Every change applies and saves immediately — there's no OK button — and
each setting shows a reset arrow once it differs from its default.

| Setting           | Options                            | Default |
| ----------------- | ---------------------------------- | ------- |
| Default font size | 12–24 px                           | 16 px   |
| Theme             | System, Light, Dark                | System  |
| Check for updates | Every launch, Daily, Weekly, Never | Daily   |
| Always on top     | on/off                             | off     |

Settings are stored in `settings.json`, under
`%APPDATA%\Video Info` on Windows and
`~/Library/Application Support/Video Info` on macOS.

## Updates

Video Info can check whether a newer release exists and show a notice
with a link to it. That's all it does: nothing is ever downloaded or
installed automatically, because the app ships unsigned.

The check asks the GitHub releases API for this repository and sends
nothing about you or your files. Set **Check for updates** to **Never**
to switch it off entirely.

## Building from source

1. Install Rust via the [getting-started guide](https://www.rust-lang.org/learn/get-started).
2. Clone this repository and change into it.
3. Build and run:
   ```sh
   cargo build --release
   cargo run --release
   ```
4. To package a `.app`/`.dmg`/installer with a bundled `ffprobe`, place a
   static `ffprobe` binary at `binaries/ffprobe-<target-triple>` (e.g.
   `binaries/ffprobe-aarch64-apple-darwin`, or `.exe` on Windows) — see
   [.github/workflows/release.yml](.github/workflows/release.yml) for
   where CI downloads these from — then run:
   ```sh
   cargo install cargo-packager --locked
   cargo packager --release
   ```

## License

Video Info is [MIT](LICENSE) licensed. Released builds bundle a separately
licensed `ffprobe` binary (GPL on macOS, LGPL on Windows — see
[.github/workflows/release.yml](.github/workflows/release.yml) for sources).
