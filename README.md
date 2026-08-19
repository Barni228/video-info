# Video Info

A small, modern alternative to MediaInfo. Video Info is a lightweight
desktop app that shows you the technical metadata of a video file:
container format, video/audio/subtitle codecs, resolution, bitrate,
frame rate, duration, file size, and chapters.

It's cross-platform (macOS and Windows), built in Rust with
[Slint](https://slint.rs/) for the UI, and uses `ffprobe` under the
hood to read the file.

## Features

- Drag and drop a video file onto the window, or use the native file
  picker
- Open file with "Open With" option for common video files
  (mp4, mkv, mov, avi, webm, m4v, mpg, mpeg, flv, wmv)
- Container, video/audio/subtitle stream, and chapter details
- Zoom controls (`Cmd`/`Ctrl` +/-/0) to scale the UI
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

### "App is damaged and can't be opened" (downloaded builds)

The `.dmg`/`.app` files on GitHub Releases aren't signed with a paid Apple
Developer ID or notarized by Apple. macOS adds `com.apple.quarantine` to
browser downloads, so Gatekeeper may show "is damaged and can't be opened".
The app isn't actually broken

To run it anyway:

```sh
xattr -cr "/Applications/Video Info.app"
```

(or wherever you moved it), then open it normally.

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
