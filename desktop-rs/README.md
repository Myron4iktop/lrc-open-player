# LRC Open Player, Rust port

A native Rust rewrite of the Windows player in `desktop/`. It is not finished yet: the Python player keeps shipping until this one passes the testing checklist in the port plan.

## Layout

- `crates/lrc-core`: lyrics files, LRC parsing, tags and covers, embedding lyrics, album order and the LRCLIB lookup. No UI and no Windows APIs, so it is tested on Linux.
- `crates/lrc-romaji`: Japanese to romaji (from M5).
- `crates/lrc-player`: the `.exe`, with the Slint UI in `ui/` and Windows-only code behind `#[cfg(windows)]`.

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
```

The `lrc-core` tests compare against results written by the Python player itself, through `tools/make_fixtures.py` (see `tools/README.md`). CI also makes tagged test songs in all six formats with ffmpeg and checks that mutagen reads back the lyrics `lrc-core` writes. The LRCLIB test needs the internet, so it only runs by hand: `cargo test -p lrc-core -- --ignored`.
