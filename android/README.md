# LRC Companion (Android)

Companion app for the desktop LRC Open Player:

- Browse and search a music folder you grant access to (e.g. `Download/AUDIO`)
- Edit tags: title, artist, album, album artist, track, disc, year, genre, cover
- Search LRCLIB and browse every candidate (duration match, synced/plain, preview)
- Romaji view: Japanese lines are replaced by romaji, translations stay below
- Save lyrics as `Song.lrc` next to the song (the desktop player picks it up) and/or embed them in the tags
- Convert an LRC to romaji (timestamps and translation lines are kept)
- Standalone romaji converter for any pasted text or LRC

## Building the APK

1. Install Android Studio (https://developer.android.com/studio).
2. File > Open and select this `android` folder.
   - If Android Studio asks which Gradle JDK to use, choose "Use JDK 21".
3. Wait for the Gradle sync to finish. The first sync downloads Gradle, the Android SDK
   and the libraries, so it can take several minutes.
   - If Studio offers to upgrade the Android Gradle Plugin, you can postpone it.
4. Build the APK:
   - Newer Studio: Build > Generate App Bundles or APKs > Generate APKs
   - Older Studio: Build > Build Bundle(s) / APK(s) > Build APK(s)
5. The APK ends up in `app/build/outputs/apk/debug/app-debug.apk`.

## Installing on the phone

Either:

- Copy `app-debug.apk` to the phone, open it, and allow installing from that source when asked, or
- Enable Developer options and USB debugging on the phone, plug it in, and press Run in Android Studio.

## First run

1. Tap "Choose folder" and pick `Download/AUDIO` (the picker opens there if it exists), then "Use this folder" / "Allow".
2. Tap a song to edit it. "Find on LRCLIB" lists every match; tap one to preview it, then "Use synced lyrics".
3. "Save .lrc" writes the lyrics next to the song. "Save tags" writes the tags (and the lyrics too, if the embed switch is on).

The first romaji conversion takes a few seconds while the Japanese dictionary loads.

## Notes

- Tag editing works on a temporary copy and only overwrites the original after the edited copy is verified.
  Keep backups of anything irreplaceable anyway.
- Opus files can't be tag-edited (jaudiotagger has no Opus support), but their .lrc files work fine.
- Romaji uses Kuromoji (IPADIC). Stylized readings in lyrics (e.g. 刃 sung as "yaiba") can't be guessed by any analyzer.
