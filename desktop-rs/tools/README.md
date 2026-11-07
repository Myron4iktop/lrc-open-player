# Tools

`make_fixtures.py` writes the expected results for the `lrc-core` tests by running the Python player itself (`desktop/lrcplayer.py`), so the Rust tests check against what the Python app really does.

It needs Python 3.13, the packages in `requirements.txt` and, for `media`, ffmpeg. It runs headless.

```
pip install -r tools/requirements.txt
python tools/make_fixtures.py pure                     # rewrite crates/lrc-core/tests/fixtures/pure.json
python tools/make_fixtures.py pure --check             # fail if that file is out of date
python tools/make_fixtures.py media /tmp/media         # tagged songs in all six formats, and media.json
python tools/make_fixtures.py check-embedded /tmp/media
```

- **pure**: lyrics decoding in every Windows code page, LRC parsing (run through the window's own `parse_and_display_lyrics`), timestamps, search text, album order, `_same_text` and the LRCLIB rules (run through `LrcFetchWorker` with a fake network); the AI alignment prompt, answer parsing and translation clean-up (`align`); the AI settings rules, addresses, error texts and cost lines (`ai`); and the sync and translation editors' LRC output (`editor`). The JSON is committed; rerun it when the Python code changes.
- **media**: makes 15 tiny songs with ffmpeg and mutagen, and records what `read_track_tags`, `extract_metadata_and_cover` and `scan_album_folder` give for each. CI makes them fresh on every run; they are not committed.
- **check-embedded**: after `cargo test` wrote lyrics into copies of those songs, checks that the Python app reads each copy exactly as `lrc-core` does.

To run the song tests locally:

```
python tools/make_fixtures.py media /tmp/media
LRC_MEDIA_FIXTURES=/tmp/media LRC_EMBED_OUT=/tmp/media/embedded cargo test -p lrc-core
python tools/make_fixtures.py check-embedded /tmp/media
```

Without `LRC_MEDIA_FIXTURES`, the song tests are skipped. CI sets `LRC_REQUIRE_MEDIA_FIXTURES`, which turns a missing folder into a failure.
