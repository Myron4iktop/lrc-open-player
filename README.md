<p align="center"><img src="assets/icon.png" width="128" alt=""></p>

# LRC Open Player

A clean Windows music player built around synced lyrics. It finds lyrics for your songs, shows Japanese lyrics as romaji with a translation underneath, and gives you the tools to sync lyrics and line up translations yourself.

The repository also contains **LRC Companion**, an Android app for fixing tags and lyrics on your phone.

## Features

### Player (Windows)

- **Synced lyrics** that follow the song, from a `.lrc` file next to it, from the file's own tags, or found online on [LRCLIB](https://lrclib.net)
- **Romaji** for Japanese lyrics (press `R`), with a translation shown under each line
- **Sync lyrics** (`Ctrl+T`): play the song and press Enter as each line starts, and plain lyrics become a synced `.lrc`
- **Add translation** (`Ctrl+Shift+T`): pair a translation with synced lyrics, either line by line or with optional AI alignment
- Save lyrics as a `.lrc` file or embed them in the audio file, so they travel with it
- Albums, shuffle, repeat (off / track / album) and drag and drop
- Dark and light themes, seven accent colors and an optional visualizer
- Windows media keys and lock-screen controls, a tray icon, and it remembers where you left off
- Plays FLAC, MP3, M4A, OGG, Opus and WAV

Press `F1` in the player for every keyboard shortcut.

### LRC Companion (Android)

- Browse and search a music folder, and edit tags and cover art
- Search LRCLIB and compare every match (length, synced or plain) before choosing one
- Preview and convert Japanese lyrics to romaji, and save `.lrc` files the player picks up

## Download

Get **LRCOpenPlayer.exe** from [Releases](https://github.com/Myron4iktop/lrc-open-player/releases) and run it; there is nothing to install. Windows SmartScreen may say "Windows protected your PC" because the program isn't signed with a paid certificate: click **More info**, then **Run anyway**.

## Building the player

The player is written in Rust, in `desktop-rs/`. You need [Rust](https://rustup.rs/) (stable) and, on Windows, the Visual Studio C++ build tools that rustup asks for.

```
cd desktop-rs
cargo build --release -p lrc-player
```

The finished program is `desktop-rs/target/release/LRCOpenPlayer.exe`. See [desktop-rs/README.md](desktop-rs/README.md) for the tests and the code layout.

### The previous Python version

The original player, in `desktop/`, still runs. It needs Python 3.11 or newer:

```
cd desktop
pip install -r requirements.txt
python lrcplayer.py
```

## Building the Android app

See [android/README.md](android/README.md). In short: open the `android` folder in [Android Studio](https://developer.android.com/studio) and use Build → Generate APKs.

## AI alignment (optional)

When a translation splits or joins lines differently from the song, matching it line by line doesn't work. A language model can work out which translated line belongs to which sung line instead. It only matches line numbers; it never writes or changes the translation, and you review the result before saving.

Turn it on in **View → AI alignment…** and choose one of:

- **On this computer:** any server with an OpenAI-compatible API, such as LM Studio, Ollama, llama.cpp or KoboldCpp
- **OpenRouter (online):** paste an API key from [openrouter.ai](https://openrouter.ai/keys) and pick a model. You pay OpenRouter per use; the window shows an estimate per song.

API keys are stored encrypted for your Windows account.

## Privacy

There are no accounts, ads or telemetry. The player only goes online for:

- **Lyrics search:** the song's title, artist and length are sent to LRCLIB.
- **AI alignment, if you turn it on:** the lyrics and translation are sent to the server you chose. That's your own computer, or OpenRouter and the company running the model you picked.

## Third-party software

The player and the Android app are built on open-source libraries; see [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).

## License

LRC Open Player and LRC Companion are free software under the [GNU General Public License v3.0](LICENSE). You can use, study, share and change them. If you share a changed version, you have to share its full source code under the same license, so the player can't be turned into a closed, paid program.

The name "LRC Open Player" and the app icon aren't covered by the license. Please use a different name and icon for your own versions.
