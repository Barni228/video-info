# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

Video Info: a small Rust desktop app built with [Slint](https://slint.rs/) that shells out to `ffprobe` to display metadata (container, codecs, resolution, bitrate, chapters, subtitles) for a video file. The user picks a file via drag-and-drop onto the window or a native file-picker button.
It has to be cross-platform (works with Mac and Windows)

## Commands

- Build: `cargo build`
- Run: `cargo run`
- Release build: `cargo build --release`
- Check without building: `cargo check`
- Package as `.app`/`.dmg` (macOS): `cargo packager --release` (uses `[package.metadata.packager]` in [Cargo.toml](Cargo.toml); runs `cargo build --release` first via `before-packaging-command`)

There are no automated tests in this repo currently.

## Architecture

The app is intentionally a two-file program:

- **[src/main.rs](src/main.rs)** — all application logic:

  - `main()` sets up the Slint window, selects the `winit` backend explicitly (required for native OS drag-and-drop via `WindowEvent::DroppedFile`), wires up the browse button, and handles the "opened via file association" case (`std::env::args().nth(1)`; noted as not yet working on macOS).
  - `analyze_file()` is the entry point triggered by drag-drop, the browse dialog, or a CLI arg. It immediately updates UI state (filename, "Analyzing..." status) then spawns a background `std::thread` so `ffprobe` doesn't block the UI thread; the result is marshalled back via `slint::invoke_from_event_loop`.
  - `run_ffprobe()` shells out to `ffprobe` (currently hardcoded to `/opt/homebrew/bin/ffprobe` rather than relying on `PATH` — see the commented-out line above it), parses the JSON output into `FfprobeOutput`/`StreamInfo`/`FormatInfo`/`ChapterInfo` structs via `serde`, and builds a flat `Vec<InfoRow>` (label/value pairs) for display. Video, audio, subtitle streams and chapters are each handled by dedicated blocks appending rows.
  - Small formatting helpers: `human_duration`, `human_size`, `parse_fraction` (for parsing ffprobe's `"num/den"` frame-rate strings).

- **[ui/app.slint](ui/app.slint)** — the entire UI in one Slint component (`AppWindow`). State is exposed as `in-out property` (`has-file`, `is-drag-hover`, `file-name`, `status-text`, `is-error`, `info-rows: [InfoRow]`) that Rust sets directly (`app.set_*`), plus one `callback browse-clicked()` invoked from Slint and handled in Rust. The `InfoRow` struct (`label`, `value`) is defined in `.slint` and shared with Rust via `slint::include_modules!()` in `main.rs`, which compiles the file at build time ([build.rs](build.rs) calls `slint_build::compile`).

There is no separate "backend"/"frontend" split beyond this Rust/Slint boundary — all business logic (ffprobe invocation, parsing, formatting) lives in `main.rs`, and `app.slint` is purely presentational, driven by the properties above.

### Editing the UI

Use the Slint LSP extension (`Slint.slint`, listed in [.vscode/extensions.json](.vscode/extensions.json)) for `.slint` editing. Any new UI property or callback added in `app.slint` must be mirrored with corresponding `app.set_*`/`app.on_*` calls in `main.rs`.
