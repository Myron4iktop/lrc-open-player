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
