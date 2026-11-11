# LRC Open Player, Rust port

A native Rust rewrite of the Windows player in `desktop/`. It is not finished yet: the Python player keeps shipping until this one passes the testing checklist in the port plan.

## Layout

- `crates/lrc-core`: lyrics files, LRC parsing, tags and covers, embedding lyrics, album order, the LRCLIB lookup, and the rules for the settings and session. No UI and no Windows APIs, so it is tested on Linux.
- `crates/lrc-romaji`: Japanese to romaji (from M5).
- `crates/lrc-player`: the `.exe`, with the Slint UI in `ui/` and Windows-only code behind `#[cfg(windows)]`. `src/audio/` is the audio engine: Symphonia decodes, Rubato resamples, cpal plays, and the clock counts the frames actually played. `src/app.rs` runs the main window (`ui/main.slint`), `src/platform.rs` holds what it needs from the system: the title bar color, the taskbar icon and dropped files, and `src/session.rs` keeps the settings file. Debug builds open a song or album folder given in `LRC_DEV_OPEN` at start, instead of the last session.

## Building

You need [Rust](https://rustup.rs/) (stable). On Windows, also the Visual Studio C++ build tools that rustup asks for.

```
cd desktop-rs
cargo build --release -p lrc-player
```

The finished program is `desktop-rs/target/release/LRCOpenPlayer.exe`. Every pull request also builds it on GitHub: open the run under **Actions → desktop-rs** and download the `LRCOpenPlayer` artifact.

Checks that CI runs:

```
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test -p lrc-core
cargo test -p lrc-player
```

The `lrc-core` tests compare against results written by the Python player itself, through `tools/make_fixtures.py` (see `tools/README.md`). CI also makes tagged test songs in all six formats with ffmpeg and checks that mutagen reads back the lyrics `lrc-core` writes. The LRCLIB test needs the internet, so it only runs by hand: `cargo test -p lrc-core -- --ignored`.

On Linux, building the player needs the ALSA headers (`libasound2-dev`) and CMake for libopus. The `lrc-player` tests decode the CI test songs (set `LRC_MEDIA_FIXTURES` as for `lrc-core`). One more test plays through a real sound device, so it only runs by hand: `cargo test -p lrc-player -- --ignored`.

## Settings

The Rust build keeps its settings in `%APPDATA%\LRCOpenPlayer\settings.json`, with the same key names as the Python app (`ui/theme`, `playback/last_track`, …). The first time it starts, it copies the Python app's settings from `HKEY_CURRENT_USER\Software\LRCPlayer\MinimalistLRCPlayer` (all but the window position, a Qt blob). After that the two apps keep their own settings, and the Rust build never writes to the registry. Delete `settings.json` to import again.

One key is new in the Rust build: `ui/smooth_animation` (View → Smooth animation). When it is `true`, the window is drawn with the GPU (Slint's FemtoVG renderer over OpenGL) instead of the software renderer, from the next start; if the GPU can't be used, the software renderer is used after all.
