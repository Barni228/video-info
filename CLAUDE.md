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
- **[settings.rs](src/settings.rs)** — the persisted `Settings` (font size, theme, always-on-top), where the JSON file lives per-platform, and `SettingsStore`, which holds them in memory and writes through to disk on every change. `ThemeSetting::ALL` is the single source of truth for the theme dropdown's contents and ordering — the Settings window gets its list from Rust rather than hardcoding one.
- **[ffprobe.rs](src/ffprobe.rs)** — locating the `ffprobe` binary (preferring the copy bundled next to the executable, falling back to `PATH`), running it, and deserializing its JSON into `Report`/`Stream`/`Format`/`Chapter`. Raw fields mirror ffprobe's output; derived values (codec name, frame rate, bitrates) are accessor methods. `ffprobe::Error`'s `Display` text is what the UI shows the user.
- **[report.rs](src/report.rs)** — turns a `Report` into the flat `Vec<InfoRow>` the UI displays, in display order, plus `as_text` for the raw-text view and Copy All. The `human_*` formatting helpers live here.
- **[ui.rs](src/ui.rs)** — everything that touches the Slint windows: pushing stored settings in, applying the theme to both windows' `Theme` global, wiring drag-and-drop/browse/copy/settings callbacks, and `analyze_file`, which spawns the background probe and marshals the result back via `slint::invoke_from_event_loop`.
- **[macos_open.rs](src/macos_open.rs)** — macOS-only. Receives files from Finder "Open With" by adding `application:openURLs:` to winit's existing `NSApplicationDelegate` class at runtime (winit crashes if the delegate is replaced). Raw libobjc FFI; read the file header before touching it.

### Slint ([ui/](ui/))

[build.rs](build.rs) compiles [ui/app.slint](ui/app.slint), which is only a set of re-exports — anything Rust needs to reach must be exported from there.

- **[ui/theme.slint](ui/theme.slint)** — the `Theme` global (Catppuccin Mocha/Latte palette) and `ThemeMode` enum. Globals are per top-level component, so Rust must set `mode`/`system-is-dark` on _both_ windows (`ui::apply_theme` does). It also holds `ThemeBridge`, a zero-sized element each Window places once: it copies `Theme.is-dark` into the std-widgets `Palette.color-scheme`, so `Button`, `ComboBox`, `Slider` and friends — which draw from that palette, not from `Theme` — follow the chosen theme instead of the OS's.
- **[ui/widgets.slint](ui/widgets.slint)** — `SelectableText`, the read-only `TextInput` used wherever text should be selectable/copyable.
- **[ui/main-window.slint](ui/main-window.slint)** — `AppWindow` and the `InfoRow` struct. State comes in as `in-out property`s set from Rust; user actions go out as callbacks. `zoom` (View menu) scales layout metrics, `font-scale` scales the font sizes hardcoded against the 16px baseline.
- **[ui/settings-window.slint](ui/settings-window.slint)** — `SettingsWindow`, plus its `ResetButton` and `SettingRow` components. Defaults for the reset buttons and the theme list are supplied by Rust so they aren't duplicated here.

### Editing the UI

Use the Slint LSP extension (`Slint.slint`, listed in [.vscode/extensions.json](.vscode/extensions.json)) for `.slint` editing. Any new UI property or callback must be mirrored with corresponding `app.set_*`/`app.on_*` calls in [src/ui.rs](src/ui.rs).
