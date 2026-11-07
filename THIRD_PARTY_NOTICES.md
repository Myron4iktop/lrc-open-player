# Third-party software

These projects make the player and the companion app possible. Each is used under its own license; follow the links for the full terms.

## Player (Windows)

These are included in the built `.exe`.

| Project | Used for | License |
| --- | --- | --- |
| [PyQt6](https://www.riverbankcomputing.com/software/pyqt/) | Python bindings for Qt | GPL-3.0 |
| [Qt 6](https://www.qt.io/) | Window, audio playback | LGPL-3.0 |
| [FFmpeg](https://ffmpeg.org/) (inside Qt Multimedia) | Decoding audio | LGPL-2.1-or-later |
| [mutagen](https://github.com/quodlibet/mutagen) | Reading and writing tags | GPL-2.0-or-later |
| [cutlet](https://github.com/polm/cutlet) | Japanese to romaji | MIT |
| [fugashi](https://github.com/polm/fugashi) (with MeCab) | Splitting Japanese into words | MIT, BSD-3-Clause |
| [unidic-lite](https://github.com/polm/unidic-lite) (UniDic 2.1.2) | Japanese dictionary | MIT, BSD-3-Clause |
| [PyWinRT](https://github.com/pywinrt/pywinrt) | Windows media controls | MIT |

## Player, Rust build (in progress)

The Rust port in `desktop-rs/` is not released yet. These are the libraries it uses so far.

| Project | Used for | License |
| --- | --- | --- |
| [Slint](https://slint.dev/) | Window and user interface | GPL-3.0 (one of its license options) |
| [winit](https://github.com/rust-windowing/winit) | Window icon, title bar and dropped files | Apache-2.0 |
| [image](https://github.com/image-rs/image) | Reading the app icon and cover art | MIT or Apache-2.0 |
| [Lofty](https://github.com/Serial-ATA/lofty-rs) | Reading and writing tags | MIT or Apache-2.0 |
| [encoding_rs](https://github.com/hsivonen/encoding_rs) | Reading lyrics files in older code pages | (MIT or Apache-2.0) and BSD-3-Clause |
| [ureq](https://github.com/algesten/ureq) | Looking up lyrics on LRCLIB | MIT or Apache-2.0 |
| [rustls](https://github.com/rustls/rustls), [ring](https://github.com/briansmith/ring) | Secure connections (LRCLIB and the AI alignment client) | Apache-2.0, ISC or MIT; Apache-2.0 and ISC |
| [rustls-platform-verifier](https://github.com/rustls/rustls-platform-verifier) | Checking certificates with the system's own store | MIT or Apache-2.0 |
| [webpki-roots](https://github.com/rustls/webpki-roots) | Fallback list of trusted certificates | CDLA-Permissive-2.0 |
| [serde_json](https://github.com/serde-rs/json) | Reading LRCLIB's answers and the settings file | MIT or Apache-2.0 |
| [base64](https://github.com/marshallpierce/rust-base64) | Storing the AI keys in the settings file | MIT or Apache-2.0 |
| [regex](https://github.com/rust-lang/regex) | Matching timestamps and tags | MIT or Apache-2.0 |
| [caseless](https://github.com/unicode-rs/rust-caseless) | Comparing text without case, like Python's `casefold` | MIT |
| [fastrand](https://github.com/smol-rs/fastrand) | Shuffle order | MIT or Apache-2.0 |
| [tempfile](https://github.com/Stebalien/tempfile) | Writing lyrics into a song and the settings file safely, and the private temp folder for the media flyout's cover | MIT or Apache-2.0 |
| [Symphonia](https://github.com/pdeljanov/Symphonia) | Decoding audio | MPL-2.0 |
| [symphonia-adapter-libopus](https://github.com/aschey/symphonia-adapters) | Opus decoding for Symphonia | MIT or Apache-2.0 |
| [libopus](https://opus-codec.org/) (through [opusic-sys](https://github.com/DoumanAsh/opusic-sys)) | Decoding Opus | BSD-3-Clause |
| [cpal](https://github.com/RustAudio/cpal) | Audio output | Apache-2.0 |
| [Rubato](https://github.com/HEnquist/rubato) (with [audioadapter](https://github.com/HEnquist/audioadapter-rs) and [RealFFT](https://github.com/HEnquist/realfft)) | Resampling to the output's rate | MIT; MIT or Apache-2.0 |
| [RustFFT](https://github.com/ejmahler/RustFFT) | Resampling, and the visualizer's spectrum | MIT or Apache-2.0 |
| [tiny-skia](https://github.com/linebender/tiny-skia) | Drawing the visualizer | BSD-3-Clause |
| [rtrb](https://github.com/mgeier/rtrb) | Passing audio to the output without locks | MIT or Apache-2.0 |
| [rfd](https://github.com/PolyMeilex/rfd) | File dialogs | MIT |
| [windows](https://github.com/microsoft/windows-rs) | Windows APIs: title bar, media flyout and media keys, tray icon, encrypting the AI keys (DPAPI) | MIT or Apache-2.0 |
| [winresource](https://github.com/BenjaminRi/winresource) (build only) | Putting the icon into the `.exe` | MIT |

## LRC Companion (Android)

| Project | Used for | License |
| --- | --- | --- |
| [Jetpack Compose and AndroidX](https://developer.android.com/jetpack) | User interface | Apache-2.0 |
| [Kotlin coroutines](https://github.com/Kotlin/kotlinx.coroutines) | Background work | Apache-2.0 |
| [jaudiotagger](https://bitbucket.org/ijabz/jaudiotagger) | Reading and writing tags | LGPL-2.1 |
| [Kuromoji](https://github.com/atilika/kuromoji) (with IPADIC) | Japanese to romaji | Apache-2.0, IPADIC license |

## Online services

- [LRCLIB](https://lrclib.net), a free, community-run lyrics database, is used to search for lyrics.
- [OpenRouter](https://openrouter.ai) is used only if you choose it for AI alignment.
