# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

Video Info: a small Rust desktop app built with [Slint](https://slint.rs/) that shells out to `ffprobe` to display metadata (container, codecs, resolution, bitrate, chapters, subtitles) for a video / audio file.
It has to be cross-platform (works with Mac and Windows)

## Commands

- Build: `cargo build`
- Run: `cargo run`
- Release build: `cargo build --release`
- Check without building: `cargo check`
- Test: `cargo test` (unit tests for the pure formatting/parsing helpers only — there is no UI test harness)
- Lint: `cargo clippy --all-targets`
- Package as `.app`/`.dmg` (macOS): `cargo packager --release` (uses `[package.metadata.packager]` in [Cargo.toml](Cargo.toml); runs `cargo build --release` first via `before-packaging-command`)

## Architecture

Data flows in one direction: a file arrives (drag-drop, file picker, "Open With", or CLI arg) → `ffprobe` is run on a background thread → its JSON is deserialized → that is flattened into label/value rows → the rows are pushed into the Slint UI.

### Rust ([src/](src/))

- **[main.rs](src/main.rs)** — startup path only: select the `winit` backend (required for native OS drag-and-drop via `WindowEvent::DroppedFile`), create both windows, call `ui::wire`, handle the "opened via CLI arg" case, run the event loop.
- **[file_kind.rs](src/file_kind.rs)** — the video/audio extension lists, used both for the file picker's filters and to word errors: an unreadable `.mp4` is a damaged video, an unreadable `.pdf` was never media at all.
- **[settings.rs](src/settings.rs)** — the persisted `Settings` (font size, theme, always-on-top, update interval), where the JSON file lives per-platform, and `SettingsStore`, which holds them in memory and writes through to disk on every change. `ThemeSetting::ALL` and `UpdateInterval::ALL` are the single source of truth for their dropdowns' contents and ordering — the Settings window gets both lists from Rust rather than hardcoding them. The store is a `Mutex` rather than a `RefCell` because the update check records its result from the background thread it runs on.
- **[ffprobe.rs](src/ffprobe.rs)** — locating the `ffprobe` binary (preferring the copy bundled next to the executable, falling back to `PATH`), running it, and deserializing its JSON into `Report`/`Stream`/`Format`/`Chapter`. Raw fields mirror ffprobe's output; derived values (codec name, frame rate, bitrates) are accessor methods. It also owns every user-facing failure message: `run` stats the path first (missing, folder, empty, unreadable) and classifies a non-zero ffprobe exit from its stderr into an `Error` variant, whose `headline`/`detail` are the two lines the UI shows. `tidy_stderr` strips the demuxer tags and full paths out of ffprobe's own wording before it is shown. A file that reads but makes ffprobe complain comes back as `Probe { report, warning }` rather than an error.
- **[report.rs](src/report.rs)** — turns a `Report` into the flat `Vec<InfoRow>` the UI displays, in display order, plus `as_text` for the raw-text view and Copy All. The `human_*` formatting helpers live here. Its `Media` enum (movie / audio / image, the last split by whether it animates) decides which sections appear: an absence is only worth a row where the thing could have been there, so "No audio stream" shows for a movie but not a GIF, and a still image gets no duration or frame rate.
- **[update.rs](src/update.rs)** — asks the GitHub releases API whether a newer version exists, compares it against `CARGO_PKG_VERSION`, and opens the releases page in the browser. It only ever *notifies*: nothing is downloaded or installed, because the app ships unsigned. Every network failure is `None` — an update check isn't worth an error message.
- **[ui.rs](src/ui.rs)** — everything that touches the Slint windows: pushing stored settings in, applying the theme to both windows' `Theme` global, wiring drag-and-drop/browse/copy/settings callbacks, and `analyze_file`, which spawns the background probe and marshals the result back via `slint::invoke_from_event_loop`.
- **[macos_open.rs](src/macos_open.rs)** — macOS-only. Receives files from Finder "Open With" by adding `application:openURLs:` to winit's existing `NSApplicationDelegate` class at runtime (winit crashes if the delegate is replaced). Raw libobjc FFI; read the file header before touching it.

### Slint ([ui/](ui/))

[build.rs](build.rs) compiles [ui/app.slint](ui/app.slint), which is only a set of re-exports — anything Rust needs to reach must be exported from there.

- **[ui/theme.slint](ui/theme.slint)** — the `Theme` global (Catppuccin Mocha/Latte palette) and `ThemeMode` enum. Globals are per top-level component, so Rust must set `mode`/`system-is-dark` on _both_ windows (`ui::apply_theme` does). It also holds `ThemeBridge`, a zero-sized element each Window places once: it copies `Theme.is-dark` into the std-widgets `Palette.color-scheme`, so `Button`, `ComboBox`, `Slider` and friends — which draw from that palette, not from `Theme` — follow the chosen theme instead of the OS's.
- **[ui/widgets.slint](ui/widgets.slint)** — `SelectableText`, the read-only `TextInput` used wherever text should be selectable/copyable.
- **[ui/main-window.slint](ui/main-window.slint)** — `AppWindow` and the `InfoRow` struct. State comes in as `in-out property`s set from Rust; user actions go out as callbacks. `zoom` (View menu) scales layout metrics, `font-scale` scales the font sizes hardcoded against the 16px baseline.
- **[ui/settings-window.slint](ui/settings-window.slint)** — `SettingsWindow`, plus its `ResetButton` and `SettingRow` components. Defaults for the reset buttons and the theme/update-interval lists are supplied by Rust so they aren't duplicated here. Its `preferred-height` does **not** decide the size it opens at: the content is in a `ScrollView`, which reports a small preferred height of its own and wins, so the window would open at `min-height`. `ui::size_settings_window` sets the starting size from the `content-height` property instead, leaving `min-height` as the floor the user can shrink to.

### Editing the UI

Use the Slint LSP extension (`Slint.slint`, listed in [.vscode/extensions.json](.vscode/extensions.json)) for `.slint` editing. Any new UI property or callback must be mirrored with corresponding `app.set_*`/`app.on_*` calls in [src/ui.rs](src/ui.rs).
