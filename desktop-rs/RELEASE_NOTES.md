LRC Open Player 1.1 lets you build a tracklist from songs anywhere on your computer, and choose which version of a song's lyrics to use when [LRCLIB](https://lrclib.net) has several.

## What's new

- **Add songs to the tracklist that is playing.** Open ▾ → Add songs to tracklist… (`Ctrl+Shift+A`), the **+** button above the tracklist, or hold `Shift` while dropping songs or folders onto the window. The music carries on, the new songs go to the end, and the list is still there the next time you start the player.
- **Choose the version of the lyrics.** Turn on Lyrics ▾ → Let me choose the version, and Lyrics ▾ → Search online lists every version LRCLIB has, with its length next to your file's. Pick one and it shows in the player straight away, so you can watch whether the lines keep time with the song. "Use these lyrics" keeps it; Cancel puts the old lyrics back.
- Open ▾ → Audio files… can open several songs at once.

## Download

Download **LRCOpenPlayer.exe** below and run it. It needs Windows 10 or 11 (64-bit).

**"Windows protected your PC"**: the program isn't signed with a paid certificate, so Windows SmartScreen warns about it the first few times anyone downloads it. Click **More info**, then **Run anyway**. To check the file is the one built here, compare its SHA-256 with `SHA256SUMS.txt`: in PowerShell, `Get-FileHash .\LRCOpenPlayer.exe`.

## What it does

- Synced lyrics that follow the song, from a `.lrc` file next to it, from the file's own tags, or found online
- Romaji for Japanese lyrics (press `R`), with a translation under each line
- Sync lyrics (`Ctrl+T`) and add a translation (`Ctrl+Shift+T`), line by line or with optional AI alignment
- Save lyrics as a `.lrc` file or embed them in the audio file
- Lyrics a little early or late? `[` and `]` fix that song, and Lyrics ▾ → Use this offset for all songs sets a default (for Bluetooth headphones, say)
- Albums and tracklists of your own, shuffle, repeat, drag and drop, dark and light themes, an optional visualizer
- Windows media keys and the media flyout, a tray icon, and it remembers where you left off
- Plays FLAC, MP3, M4A (AAC and ALAC), OGG, Opus and WAV

Press `F1` in the player for every keyboard shortcut.

## Coming from the Python version

The first time it starts, it copies your settings from the Python version (theme, volume, last song and so on). After that the two keep separate settings, so you can keep both for a while. Settings are in `%APPDATA%\LRCOpenPlayer\settings.json`.

## Privacy

No accounts, ads or telemetry. It only goes online to look up lyrics on LRCLIB (title, artist and length), and, if you turn AI alignment on, to the server you chose.

## License

Free software under the [GNU GPL v3.0](https://github.com/Myron4iktop/lrc-open-player/blob/main/LICENSE). The licenses of the libraries inside the `.exe` are in `THIRD_PARTY_LICENSES.html`.
