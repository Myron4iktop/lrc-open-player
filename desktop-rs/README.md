# LRC Open Player, Rust port

A native Rust rewrite of the Windows player in `desktop/`. It is not finished yet: the Python player keeps shipping until this one passes the testing checklist in the port plan.

## Layout

- `crates/lrc-core`: lyrics files, LRC parsing, tags and covers, embedding lyrics, album order, the LRCLIB lookup, the visualizer's analysis, and the rules for the settings, session, tray and media flyout. No UI and no Windows APIs, so it is tested on Linux.
- `crates/lrc-romaji`: Japanese to romaji. It ports the Python app's cutlet rules and splits words with Lindera, which embeds its Japanese dictionary in the `.exe`: UniDic by default (cutlet's dictionary), or IPADIC (the Android app's) with `--no-default-features --features ipadic`.
- `crates/lrc-player`: the `.exe`, with the Slint UI in `ui/` and Windows-only code behind `#[cfg(windows)]`. `src/audio/` is the audio engine: Symphonia decodes, Rubato resamples, cpal plays, and the clock counts the frames actually played. `src/app.rs` runs the main window (`ui/main.slint`), `src/platform.rs` holds what it needs from the system: the title bar color, the taskbar icon and dropped files, `src/session.rs` keeps the settings file, `src/visualizer.rs` draws the spectrum behind the lyrics, and `src/errors.rs` shows unexpected errors instead of closing the app. `src/win/` holds the Windows media flyout and media keys (SystemMediaTransportControls) and the tray icon (`Shell_NotifyIcon`). Debug builds open a song or album folder given in `LRC_DEV_OPEN` at start, instead of the last session.

## Building

You need [Rust](https://rustup.rs/) (stable). On Windows, also the Visual Studio C++ build tools that rustup asks for.

```
cd desktop-rs
cargo build --release -p lrc-player
```

The finished program is `desktop-rs/target/release/LRCOpenPlayer.exe`. Every pull request also builds it on GitHub: open the run under **Actions → desktop-rs** and download the `LRCOpenPlayer` artifact.

Lindera downloads its dictionary from lindera.dev while building `lrc-romaji` (and so `lrc-player`), so that build needs the internet. `cargo test -p lrc-core` doesn't.

Checks that CI runs:

```
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test -p lrc-core
cargo test -p lrc-player
cargo test -p lrc-romaji
cargo test -p lrc-romaji --no-default-features --features ipadic
```

To see how close the romaji is to the Python app's on your own lyrics, run the Python app's converter over a folder of `.lrc` files, then compare:

```
python tools/make_fixtures.py romaji "D:\Music" romaji.json
cargo run -p lrc-romaji --release --example compare -- romaji.json
cargo run -p lrc-romaji --release --no-default-features --features ipadic --example compare -- romaji.json
```

The `lrc-core` tests compare against results written by the Python player itself, through `tools/make_fixtures.py` (see `tools/README.md`). CI also makes tagged test songs in all six formats with ffmpeg and checks that mutagen reads back the lyrics `lrc-core` writes. The LRCLIB test needs the internet, so it only runs by hand: `cargo test -p lrc-core -- --ignored`.

On Linux, building the player needs the ALSA headers (`libasound2-dev`) and CMake for libopus. The `lrc-player` tests decode the CI test songs (set `LRC_MEDIA_FIXTURES` as for `lrc-core`). One more test plays through a real sound device, so it only runs by hand: `cargo test -p lrc-player -- --ignored`.

## Settings

The Rust build keeps its settings in `%APPDATA%\LRCOpenPlayer\settings.json`, with the same key names as the Python app (`ui/theme`, `playback/last_track`, …). The first time it starts, it copies the Python app's settings from `HKEY_CURRENT_USER\Software\LRCPlayer\MinimalistLRCPlayer` (all but the window position, a Qt blob). After that the two apps keep their own settings, and the Rust build never writes to the registry. Delete `settings.json` to import again.

One key is new in the Rust build: `ui/smooth_animation` (View → Smooth animation). When it is `true`, the window is drawn with the GPU (Slint's FemtoVG renderer over OpenGL) instead of the software renderer, from the next start; if the GPU can't be used, the software renderer is used after all.
