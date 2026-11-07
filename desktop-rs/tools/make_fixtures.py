"""Expected outputs for the lrc-core tests, written by the Python player itself.

The Rust port must behave like desktop/lrcplayer.py. This script imports that file and
runs its functions on sample inputs, so the tests compare against what the Python app
really does rather than against someone's reading of it.

    python tools/make_fixtures.py pure [--check]
        Text, LRC, album-order, search-text and LRCLIB cases. Writes
        crates/lrc-core/tests/fixtures/pure.json, which is committed. --check only
        compares, and fails if the file is out of date.

    python tools/make_fixtures.py media OUT_DIR
        Makes tiny tagged songs in all six formats with ffmpeg and mutagen, and writes
        what the Python app reads from each to OUT_DIR/media.json. CI runs this; the
        songs are not committed.

    python tools/make_fixtures.py check-embedded OUT_DIR
        After `cargo test` wrote lyrics into copies of those songs (LRC_EMBED_OUT=OUT_DIR/
        embedded), checks that the Python app reads each one exactly as lrc-core does
        (rust.json), and that the lyrics fields are the ones mutagen would have written.

Needs PyQt6 and mutagen (pip install PyQt6 mutagen), and ffmpeg for `media`. Runs
headless: it uses Qt's offscreen platform.
"""

import base64
import contextlib
import io
import json
import os
import random
import subprocess
import sys
import types
from pathlib import Path

os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")
sys.dont_write_bytecode = True  # keep desktop/ free of __pycache__

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "desktop"))
import lrcplayer as L  # noqa: E402

from PyQt6.QtCore import QBuffer, QByteArray, QIODevice  # noqa: E402
from PyQt6.QtGui import QColor, QGuiApplication, QImage  # noqa: E402

PURE_JSON = ROOT / "desktop-rs" / "crates" / "lrc-core" / "tests" / "fixtures" / "pure.json"
EMBED_TEXT = "[00:00.50]Embedded line\n[00:00.50]Translation\n[00:01.20]Ünïcödé 日本語"


def quiet():
    """The Python app prints its errors; keep them out of the output."""
    return contextlib.redirect_stdout(io.StringIO())


def b64(data: bytes) -> str:
    return base64.b64encode(data).decode("ascii")


# ---------------------------------------------------------------------------------------
# pure
# ---------------------------------------------------------------------------------------
def decode_cases():
    real_getpref = L.locale.getpreferredencoding
    rng = random.Random(1234)
    samples = [
        b"",
        b"plain ascii\r\nline two",
        "UTF-8 ✓ café 日本語".encode("utf-8"),
        b"\xef\xbb\xbf" + "BOM UTF-8 ✓".encode("utf-8"),
        b"\xef\xbb\xbfbad \xff utf8 after bom",
        b"\xff\xfe" + "UTF-16 LE ✓".encode("utf-16-le"),
        b"\xfe\xff" + "UTF-16 BE ✓".encode("utf-16-be"),
        b"\xff\xfe" + "odd".encode("utf-16-le") + b"\x00",
        b"\xff\xfe\x00\xd8a\x00",  # lone surrogate
        b"\xff\xfe\x00\x00" + "UTF-32 LE ✓".encode("utf-32-le"),
        b"\x00\x00\xfe\xff" + "UTF-32 BE ✓".encode("utf-32-be"),
        b"\xff\xfe\x00\x00" + b"\x00\x00\x11\x00" + b"ab",  # out of range, then truncated
        "こんにちは 世界".encode("cp932"),
        "漢字だけ".encode("cp932"),
        "世界".encode("cp932"),  # no kana: falls through to the ANSI code page
        "café crème".encode("cp1252"),
        b"undefined \x81 \x8d \x8f \x90 \x9d in 1252",
        "Привет мир".encode("cp1251"),
        "Γειά σου".encode("cp1253"),
        "Merhaba dünya".encode("cp1254"),
        "שלום".encode("cp1255"),
        "مرحبا".encode("cp1256"),
        "Labas pasauli ąčę".encode("cp1257"),
        "Xin chào".encode("cp1258"),
        "สวัสดี".encode("cp874"),
        "Zażółć gęślą jaźń".encode("cp1250"),
        "你好世界".encode("gbk"),
        "안녕하세요".encode("cp949"),
        "你好世界".encode("cp950"),
        b"bad utf-8 \xc3\x28 here",
    ]
    samples += [bytes(rng.randrange(0x20, 0x100) for _ in range(40)) for _ in range(12)]
    # Mostly high bytes, so the double-byte code pages see many odd pairs.
    samples += [bytes(rng.randrange(0x80, 0x100) for _ in range(6)) for _ in range(60)]
    cases = []
    try:
        for cp in (1252, 1250, 1251, 1253, 1254, 1255, 1256, 1257, 1258, 874, 932, 936, 949, 950):
            L.locale.getpreferredencoding = lambda *_a, _cp=cp: f"cp{_cp}"
            for data in samples:
                cases.append({"data": b64(data), "code_page": cp, "text": L.decode_lyrics_bytes(data)})
    finally:
        L.locale.getpreferredencoding = real_getpref
    return cases


def parse_lyrics(content: str):
    """Runs the window's parse_and_display_lyrics on a stand-in window and lists its cards."""
    cards = []

    class Card:
        def __init__(self, time_ms, main, sub, parent):
            cards.append({"time_ms": time_ms, "main": main, "sub": sub})

        def set_display(self, *a):
            pass

        def update_style(self, **kw):
            pass

    win = types.SimpleNamespace(
        _clear_lyrics=lambda: None,
        isFullScreen=lambda: False,
        _display_for=lambda m, s: (m, s),
        lyrics_layout=types.SimpleNamespace(insertWidget=lambda *a: None),
        lyric_cards=[],
        is_synced=False,
        _reset_scroll_and_sync=lambda: None,
    )
    real_card, real_single = L.LyricCard, L.QTimer.singleShot
    L.LyricCard = Card
    L.QTimer.singleShot = staticmethod(lambda *a: None)
    try:
        L.AudioLRCPlayer.parse_and_display_lyrics(win, content)
    finally:
        L.LyricCard, L.QTimer.singleShot = real_card, real_single
    return {"synced": win.is_synced, "lines": cards}


LRC_SAMPLES = [
    "",
    "[ar:Artist]\n[ti:Title]\n[00:01.00]Hello\n[00:01.10]Bonjour\n[00:05.00]\n[00:03.00][00:07.00]Again\n",
    "﻿[00:00.00]a\n[00:00.10]b\n[00:00.20]c\n[00:00.151]d\n",
    "[00:02.00]Line\n[00:02.00]Translation\n[00:02.00]Third\n",
    "[1:02.5]x\n[01:02:34]y\n[01:02,345]z\n[001:02.123456]w\n[100:00]v\n",
    "First plain line\n\n   Second  \r\n[ar:x]\n third",
    "[00:01.00]a[00:02.00]b\n  [00:03.00]  spaced  \n[00:04.00] ♪ \n",
    "[00:01]one\r[00:02]two\r\n[00:03]three\x0bfour\x1c[00:05]five [00:06]six",
    "[００:０１.５０]full width\n[٠٠:٠٢]arabic digits\n",
    "[00:10.00]late\n[00:01.00]early\n[00:01.149]pair\n[00:01.300]alone\n",
    "no stamps [here]\n[x:y]\n",
    "[00:01.00]\n[00:01.00]\n",
    "[00:00.995]round\n[00:00.0005]tiny\n",
]


def stamp_cases():
    lines = ["[01:02]a", "[1:02.5]a", "[01:02:34]a", "[01:02,345]a", "[ar:Someone]", "[00:00.0005]",
             "[00:00.0015]", "[999:59.999999]", "[1000:00]x", "[01:2]a", "[0:00.1][0:00.12][0:00.123]x"]
    out = []
    for line in lines:
        matches = list(L._LRC_STAMP.finditer(line))
        out.append({"line": line, "ms": [L._stamp_to_ms(m) for m in matches],
                    "end": len(line[:matches[-1].end()].encode("utf-8")) if matches else 0})
    return out


QUERY_SAMPLES = [
    "01 Song", "01. Song", "01 - Song", "1-03 Song", "01_Song", "01.Song", "Song (2)", "01 - ",
    "1234 Song", "123 Song", "12-345 Song", "1.2 Song", "7 Rings", "99 Luftballons", "Song [FLAC]",
    "Song [hq]", "Song [Mp3] [WAV]", " 50 Cent [MP3] ", "01  -  Song", "01 -Song", "01- Song",
    "01　Song", "０１ Song", "01 (2)", "Song (2) (3)", "Song(2)", "Song (2)\n", "Song (a)",
    "01 - 02 Song", "1-1-1 Song", "01", "01 ", "-01 Song", "01 - Song (12)", "Song [FLAC",
    "Ünïcödé Song (2)", "",
]


def query_cases():
    return [{"text": t, "from_filename": f, "cleaned": L.clean_query_string(t, f)}
            for t in QUERY_SAMPLES for f in (True, False)]


def same_text_cases():
    pairs = [("Hello, World!", "hello world"), ("Straße", "STRASSE"), ("Hello", "Hallo"),
             ("a_b", "ab"), ("", "!!!"), ("日本語", "日本語。"), ("Ⅻ", "ⅻ"), ("naïve", "naive"),
             ("Mr. Brightside", "mr brightside"), ("ΣΊΣΥΦΟΣ", "σίσυφος"), ("½", "1/2"),
             ("रामायण", "रमयण"), ("ﬁne", "FINE"), ("x²", "x2")]
    return [{"a": a, "b": b, "same": L._same_text(a, b)} for a, b in pairs]


def album_order_cases():
    groups = []
    root = "/music/Album"
    songs = [
        (f"{root}/CD10/01 Song.mp3", {"title": None, "track": 1, "disc": None}),
        (f"{root}/CD2/02 Song.mp3", {"title": None, "track": 2, "disc": None}),
        (f"{root}/CD2/01 Song.mp3", {"title": None, "track": 1, "disc": None}),
        (f"{root}/b.flac", {"title": None, "track": None, "disc": 2}),
        (f"{root}/a.flac", {"title": None, "track": 10, "disc": 1}),
        (f"{root}/c.flac", {"title": None, "track": 9, "disc": 1}),
        (f"{root}/Track 10.ogg", {"title": None, "track": None, "disc": None}),
        (f"{root}/track 9.ogg", {"title": None, "track": None, "disc": None}),
        (f"{root}/Ärger.opus", {"title": None, "track": None, "disc": None}),
        (f"{root}/ärger 2.opus", {"title": None, "track": None, "disc": None}),
        (f"{root}/disc 1/x.m4a", {"title": None, "track": 3, "disc": 1}),
        (f"{root}/Disc 1/y.m4a", {"title": None, "track": 2, "disc": 1}),
        (f"{root}/007.wav", {"title": None, "track": None, "disc": None}),
        (f"{root}/7.wav", {"title": None, "track": None, "disc": None}),
        (f"{root}/6.wav", {"title": None, "track": None, "disc": None}),
    ]
    for use_root in (True, False):
        order = sorted(range(len(songs)),
                       key=lambda i: L.album_sort_key(songs[i][0], songs[i][1], root if use_root else ""))
        groups.append({"root": root if use_root else None,
                       "songs": [{"path": p, "track": t["track"], "disc": t["disc"]} for p, t in songs],
                       "order": order})
    loose = [
        ("/b/Album/02.mp3", {"title": None, "track": 2, "disc": None}),
        ("/a/Other/01.mp3", {"title": None, "track": 1, "disc": None}),
        ("/a/Other/10.mp3", {"title": None, "track": None, "disc": None}),
    ]
    order = sorted(range(len(loose)), key=lambda i: L.album_sort_key(loose[i][0], loose[i][1]))
    groups.append({"root": None,
                   "songs": [{"path": p, "track": t["track"], "disc": t["disc"]} for p, t in loose],
                   "order": order})
    return groups


def fmt_time_cases():
    return [{"ms": ms, "text": L._fmt_lrc_time(ms)}
            for ms in (0, -5, 4, 5, 6, 15, 25, 994, 995, 1005, 61_234, 61_235, 61_225, 59_999,
                       3_599_995, 6_000_000)]


def palette_cases():
    """Every color of every theme and accent (`build_palette`), as `QColor` computes them."""
    out = []
    for theme, _ in L.THEMES:
        for accent, _, _ in L.ACCENTS:
            out.append({"theme": theme, "accent": accent, "palette": L.build_palette(theme, accent)})
    # Unknown keys fall back the way the app does
    out.append({"theme": "dark", "accent": "nope", "palette": L.build_palette("dark", "nope")})
    return out


def window_text_cases():
    """The clock under the seek bar and the lyrics offset label."""
    times = [{"ms": ms, "text": L.AudioLRCPlayer._format_time(ms)}
             for ms in (0, 999, 1000, 59_999, 60_000, 61_500, 599_999, 3_599_999, 3_600_000,
                        3_725_000, 86_399_999, 86_400_000, 90_000_000)]
    offsets = [{"ms": ms, "text": f"{ms / 1000:+.1f} s"}
               for ms in (0, 100, -100, 1200, -30000, 30000, 50, -50, 150, 250, -250, 1)]
    return {"format_time": times, "offset": offsets}


class _FakeResponse:
    def __init__(self, status, body):
        self.status, self._body = status, body

    def read(self):
        return self._body

    def __enter__(self):
        return self

    def __exit__(self, *a):
        return False


LRCLIB_SCENARIOS = [
    ("exact synced", ("Song", "Artist", 215, True),
     {"syncedLyrics": "[00:01.00]a", "plainLyrics": "a", "duration": 215.5}, None),
    ("exact plain, no length", ("Song", "Artist", 0, True),
     {"syncedLyrics": None, "plainLyrics": "words", "duration": 200}, None),
    ("get 404, search match", ("01 - My Song (2)", "The Band", 200, False), None,
     [{"trackName": "Other", "artistName": "The Band", "duration": 200, "syncedLyrics": "[00:01.00]x"},
      {"trackName": "my song!", "artistName": "the band", "duration": 203.9, "syncedLyrics": "[00:01.00]y"}]),
    ("search other version", ("My Song", "The Band", 200, True), {"syncedLyrics": "", "plainLyrics": ""},
     [{"trackName": "My Song", "artistName": "The Band", "duration": 205, "syncedLyrics": "[00:01.00]x"}]),
    ("search plain trusted", ("My Song", "The Band", 200, True), None,
     ["junk", {"trackName": "My Song", "artistName": "The Band", "duration": "199", "plainLyrics": "w"}]),
    ("search plain untrusted", ("My Song", "Unknown Artist", 0, True), None,
     [{"trackName": "My Song", "duration": 0, "plainLyrics": "w"}]),
    ("nothing", ("Nope", "", 100, True), None, []),
    ("search not a list", ("Nope", "", 100, True), None, {"error": "x"}),
    ("search fails", ("Nope", "", 100, True), None, None),
    ("bad get duration", ("Song", "", 100, True), {"plainLyrics": "a", "duration": "x"},
     [{"trackName": "Song", "duration": 100, "plainLyrics": "from search"}]),
    ("bool duration", ("Song", "", 1, True), {"plainLyrics": "a", "duration": True}, None),
    ("track name not text", ("Song", "", 100, True), None,
     [{"trackName": 5, "duration": 100, "syncedLyrics": "[00:01.00]s"}]),
    ("synced not text", ("Song", "", 100, True), {"syncedLyrics": 5}, None),
    ("unmatched synced not text", ("Song", "", 100, True), None,
     [{"trackName": "Other", "duration": 1, "syncedLyrics": 7},
      {"trackName": "Song", "duration": 100, "plainLyrics": "p"}]),
    ("infinite duration", ("Song", "", 100, True), None,
     [{"trackName": "Song", "duration": "inf", "plainLyrics": "p"}]),
    ("artist mismatch", ("Song", "A", 100, True), None,
     [{"trackName": "Song", "artistName": "B", "duration": 100, "syncedLyrics": "[00:01.00]s"}]),
    ("unicode query", ("Café & Co", "A/B", 215, True), None, []),
]


def lrclib_cases():
    cases = []
    for name, (title, artist, duration, from_tags), get_body, search_body in LRCLIB_SCENARIOS:
        urls = []

        def fake_urlopen(req, timeout=None, _get=get_body, _search=search_body):
            urls.append(req.full_url)
            body = _get if "/api/get?" in req.full_url else _search
            if body is None:
                raise OSError("HTTP Error 404: Not Found")
            return _FakeResponse(200, json.dumps(body).encode("utf-8"))

        worker = L.LrcFetchWorker(title, artist, duration, from_tags)
        got = []
        worker.finished_signal.connect(lambda *a: got.append(a))
        real = L.urllib.request.urlopen
        L.urllib.request.urlopen = fake_urlopen
        try:
            with quiet():
                worker.run()
        finally:
            L.urllib.request.urlopen = real
        lyrics, message, trusted, lrc_duration = got[0]
        cases.append({
            "name": name, "title": title, "artist": artist, "duration_sec": duration,
            "title_from_tags": from_tags, "get": get_body, "search": search_body, "urls": urls,
            "query_title": worker.title, "query_artist": worker.artist,
            "result": {"lyrics": lyrics, "message": message, "trusted": trusted,
                       "lrclib_duration_sec": lrc_duration},
        })
    return cases


# ---------------------------------------------------------------------------------------
# Translation alignment, the AI server's texts and the two editors
# ---------------------------------------------------------------------------------------
ALIGN_LINES = [
    "[Verse]", "【サビ】", "(Verse 2)", "(Chorus x2)", "(x2)", "( × 3 )", "Chorus x2", "Repeat chorus:",
    "PRE-CHORUS", "Bridge 2:", "(come on)", "Bridge over water", "Intro", "rap", "[00:01.00]stamped",
    "[ti:Title]", "  spaced  ", "♪", "[00:02.00]♪", "I'm provoked by the sun, my body heats",
    "one, two, three, four", "a,b", "a、 b、 c", "x; y: z，w", "1, 2, 3, 4, 5, 6, 7, 8, 9",
    "a, b, c, d, e, f, g, h", "trailing,  ", ", leading", "sun,　moon", "tab,\tsep",
    "line break", "verse", "(spoken)", "(Verse 2", "[Chorus] text",
]

ALIGN_TRANSLATIONS = [
    "[Verse]", "I can hear your voice", "from beyond", "the distant sky",
    "The night is ending, and the light shines in", "one, two, three", "(x2)", "Last line",
]

ALIGN_ANSWERS = [
    "O1: T2\nO2: T3 T4\nO3: none\nO4: T5.1\nO5: T5.2\nO6: T2",
    "<think>let me see O1: T9</think>\nO1: T2\nO2: T3-T4\nO3: T6.1-3\nO4: T6.1-T6.2\nO5: T5\nO6: T1",
    "reasoning text</think>O1 → T2\n**O2**: T3, T4\n- O3 = n/a\n> O4 -> T5.2 (the rest)\nO5: T5.1 // note",
    "O1: T2\nO2: T3\nO3: <think>still",
    "Answer:\n`O1: T2`\n•  O2 – T3 to T4\nO 3 : T6.2\no4: t6.3\nO5: T6.1.\nO6: T99\nO7: T2\nO0: T2",
    "O1: T1.1-T1.999999999\nO2: T5.1-T5.999999999999999999999\nO3: T2-T9\nO4: T4-T2\nO5: T6.3-T6.1",
    "O1: T2 \"I can hear\"\nO2: T3 «from»\nO3: T4 # x\nO4: nothing\nO5: skip\nO6: —",
    "O1:\nT2\nO2: T3\r\nO3: T4\r\n",
    "O１: T２\nO2: T٣",
    "no answer at all",
    "",
    "O1: T5.1\nO2: T5.1\nO3: T6.2",
    "O1: T6.2\nO2: none\nO3: T6.3",
    "O1: T8 T7 T8\nO1: T3\nO2: T2.5\nO3: T5.3",
]

THINK_SAMPLES = [
    "<think>a</think>b", "<THINK>a</Think>b<think>c</think>d", "x</think>y</think>z", "pre<think>open",
    "İ</think>after", "plain", "<think>never closed", "a</THINK>b<THINK>c",
]

AI_URLS = [
    "localhost:1234", " http://127.0.0.1:1234/v1/chat/completions/ ", "http://x:11434/api",
    "http://x:11434/api/", "", "   ", "HTTPS://Host/v1/models?x=1", "http://h/v1/completions",
    "h:8080/v1/models/", "http://h/custom/path//", "http://h/chat/completions", "https://openrouter.ai/api/v1",
    "http://[::1]:1234", "http://h/v1#frag", "http://h?q=1", "ftp://h", "http://user@h:1/x/COMPLETIONS",
    "http://h/v1/chat/completions/models", "\thttp://h/v1",
]

HTTP_ERRORS = [
    (401, b'{"error": {"message": "bad key"}}', "/models", ""),
    (403, b"", "/models", ""),
    (402, b'{"error": {"message": "Insufficient credits"}}', "/chat/completions", "m"),
    (402, b"", "/chat/completions", "m"),
    (429, b"busy", "/chat/completions", "m"),
    (404, b'{"error": "model not found"}', "/chat/completions", "qwen"),
    (400, b'{"error": {"message": "Unknown MODEL x"}}', "/chat/completions", "x"),
    (400, b'{"error": {"message": "Unknown MODEL x"}}', "/chat/completions", ""),
    (404, b"Not Found", "/models", ""),
    (500, b'{"message": "boom"}', "/models", ""),
    (500, b"  " + b"x" * 300 + b"  ", "/models", ""),
    (500, b"[1, 2]", "/models", ""),
    (500, b'{"error": {"code": 5}}', "/models", ""),
    (503, b"", "/models", ""),
    (500, b'{"error": null}', "/models", ""),
    (500, b'"just text"', "/models", ""),
]


def align_cases():
    trans = ALIGN_TRANSLATIONS
    answers = []
    for text in ALIGN_ANSWERS:
        for n_rows in (6, 3):
            mapping = L.parse_alignment(text, n_rows, trans)
            answers.append({
                "text": text, "n_rows": n_rows, "count": L.count_answer_lines(text),
                "mapping": [[row, [list(r) for r in refs]] for row, refs in mapping.items()],
                "texts": L.alignment_to_texts(mapping, n_rows, trans),
            })
    texts_sets = [["I can hear your voice", "from beyond the distant sky", ""], ["", "", ""],
                  ["The night is ending,", "and the light shines in"], []]
    return {
        "lines": [{"line": l, "header": L.is_section_header(l), "clauses": L.split_clauses(l),
                   "used": [L.line_is_used(l, t) for t in texts_sets]} for l in ALIGN_LINES],
        "texts_sets": texts_sets,
        "clean": [{"text": t, "lines": L.clean_translation_lines(t)} for t in
                  ["\n".join(ALIGN_LINES), "[ar:x]\n[00:01.00][00:02.00] two \n\n♪\r\nlast", ""]],
        "match_in_order": [{"n": n, "lines": trans, "texts": L.match_in_order(n, trans)} for n in (0, 3, 6, 10)],
        "messages": [{"originals": o, "translations": t, "messages": L.build_alignment_messages(o, t)}
                     for o, t in [(["君の声", "遠い空", "ラララ"], trans), ([], []), (["a"], ["x, y"])]],
        "translations": trans,
        "answers": answers,
        "strip_thinking": [{"text": t, "out": L.strip_thinking(t)} for t in THINK_SAMPLES],
    }


class _FakeLabel:
    def __init__(self):
        self.text, self.name = "", ""

    def setText(self, t):
        self.text = t

    def objectName(self):
        return self.name

    def setObjectName(self, n):
        self.name = n

    def style(self):
        return types.SimpleNamespace(unpolish=lambda w: None, polish=lambda w: None)


def ai_cases():
    models = ["qwen/qwen3-32b", "qwen/qwen3-235b", "meta/llama:free", "Paid/Big", "odd"]
    pricing = {"qwen/qwen3-32b": (1e-7, 3e-7), "qwen/qwen3-235b": (2e-6, 6e-6), "meta/llama:free": (0.0, 0.0),
               "Paid/Big": (0.000003333, 0.0000111)}
    costs = []
    for typed in ["", "  ", "qwen", "QWEN3-32", "qwen/qwen3-32b", "qwen/qwen3-235b", "meta/llama:free",
                  "Paid/Big", "odd", "zzz", " qwen/qwen3-32b "]:
        lbl = _FakeLabel()
        fake = types.SimpleNamespace(ed_or_model=types.SimpleNamespace(text=lambda t=typed: t),
                                     _provider=lambda: "openrouter", or_models=models, or_pricing=pricing,
                                     lbl_cost=lbl)
        L.AiSettingsDialog._update_cost(fake)
        costs.append({"typed": typed, "text": lbl.text, "error": lbl.name == "syncError"})
    return {
        "urls": [{"url": u, "out": L.normalize_ai_url(u)} for u in AI_URLS],
        "http_errors": [{"status": s, "body": b64(b), "endpoint": e, "model": m,
                         "text": L._ai_http_error(s, b, e, m)} for s, b, e, m in HTTP_ERRORS],
        "models": models, "pricing": {k: list(v) for k, v in pricing.items()}, "costs": costs,
    }


SYNC_TEXTS = [
    "one\n\ntwo\n  three  ",
    "[ar:Artist]\n[00:05.00]five\n[00:01.00]one\n[00:01.10]uno\n[00:03.00]\n[00:04.00]♪\n[00:06.00][00:08.00]twice",
    "[00:01.00]a\n[00:01.00]b\n[00:01.00]c\nplain line",
    "[00:02.00]line\n[00:02.20]not paired\n[00:02.30]paired",
    "plain\n[00:01.00]stamped\n[00:00.50]earlier",
    "[00:01.00]\n[00:01.05]sub after break",
    "[01:02:34]colon\n[1:02,5]comma\n[00:00.0005]tiny",
    "",
]


def editor_cases():
    sync = []
    for text in SYNC_TEXTS:
        entries = L.LyricsSyncDialog._parse_text(text)
        ns = types.SimpleNamespace(entries=entries)
        lrc, missing = L.LyricsSyncDialog.build_lrc(ns)
        sync.append({"text": text, "entries": entries, "as_text": L.LyricsSyncDialog._entries_to_text(ns),
                     "lrc": lrc, "missing": missing})
    cards_sets = [
        [(1000, "Hello", ""), (2000, "♪", ""), (3000, "World", "Monde"), (4000, "Spin", "")],
        [(0, "a", ""), (61_250, "b", ""), (3_599_990, "c", "")],
    ]
    texts_sets = [["Bonjour", "Monde!", "spin"], ["", " x ", "C"]]
    trans = []
    for cards, texts in zip(cards_sets, texts_sets):
        row_cards = [i for i, (_, m, _) in enumerate(cards) if m and m != "♪"]
        ns = types.SimpleNamespace(cards=cards, row_cards=row_cards, texts=texts)
        trans.append({"cards": cards, "texts": texts, "lrc": L.TranslationDialog.build_lrc(ns)})
    times = [{"ms": ms, "text": f"{ms // 60000}:{(ms % 60000) / 1000:04.1f}"}
             for ms in (0, 1000, 59_950, 59_949, 61_250, 61_350, 600_000, 3_599_999, 25, 75)]
    return {"sync": sync, "translation": trans, "row_times": times}


def build_pure():
    return {
        "_comment": "Written by tools/make_fixtures.py pure from desktop/lrcplayer.py. Do not edit.",
        "decode": decode_cases(),
        "parse": [{"content": c, **parse_lyrics(c)} for c in LRC_SAMPLES],
        "stamps": stamp_cases(),
        "fmt_lrc_time": fmt_time_cases(),
        "clean_query": query_cases(),
        "same_text": same_text_cases(),
        "album_order": album_order_cases(),
        "lrclib": lrclib_cases(),
        "palette": palette_cases(),
        "window_text": window_text_cases(),
        "align": align_cases(),
        "ai": ai_cases(),
        "editor": editor_cases(),
    }


def cmd_pure(check: bool):
    text = json.dumps(build_pure(), ensure_ascii=False, indent=1) + "\n"
    if check:
        if not PURE_JSON.exists() or PURE_JSON.read_text(encoding="utf-8") != text:
            sys.exit(f"{PURE_JSON} is out of date: run tools/make_fixtures.py pure")
        print(f"{PURE_JSON.name} is up to date")
        return
    PURE_JSON.parent.mkdir(parents=True, exist_ok=True)
    PURE_JSON.write_text(text, encoding="utf-8")
    print(f"wrote {PURE_JSON}")


# ---------------------------------------------------------------------------------------
# media
# ---------------------------------------------------------------------------------------
def image_bytes(fmt: str, w: int, h: int, color: str) -> bytes:
    img = QImage(w, h, QImage.Format.Format_RGB32)
    img.fill(QColor(color))
    buf = QBuffer()
    buf.open(QIODevice.OpenModeFlag.WriteOnly)
    img.save(buf, fmt)
    return bytes(buf.data())


GIF_1PX = (b"GIF89a\x01\x00\x01\x00\x80\x00\x00\x00\x00\x00\xff\xff\xff!\xf9\x04\x01\x00\x00\x00"
           b"\x00,\x00\x00\x00\x00\x01\x00\x01\x00\x00\x02\x02D\x01\x00;")
SVG = (b'<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10">'
       b'<rect width="10" height="10" fill="red"/></svg>')
SYNCED = "[00:00.50]First line\n[00:00.50]Première ligne\n[00:01.50]Second line"
PLAIN = "Just words\nno stamps here"


def ffmpeg(out: Path, codec_args, seconds=2.5, rate=44100, channels=2):
    subprocess.run(
        ["ffmpeg", "-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi",
         "-i", f"sine=frequency=440:sample_rate={rate}:duration={seconds}",
         "-ac", str(channels), "-map_metadata", "-1", *codec_args, str(out)],
        check=True)


def flac_picture(data: bytes, kind: int, mime: str, desc: str = "", w=0, h=0):
    from mutagen.flac import Picture
    pic = Picture()
    pic.type, pic.mime, pic.desc, pic.data = kind, mime, desc, data
    pic.width, pic.height, pic.depth = w, h, 24 if w else 0
    return pic


def make_media(out: Path):
    from mutagen.flac import FLAC
    from mutagen.id3 import (APIC, COMM, ID3, TALB, TCON, TDRC, TIT2, TPE1, TPE2, TPOS, TRCK, TXXX,
                             TYER, USLT, PRIV)
    from mutagen.mp4 import MP4, MP4Cover, MP4FreeForm
    from mutagen.oggopus import OggOpus
    from mutagen.oggvorbis import OggVorbis
    from mutagen.wave import WAVE

    png = image_bytes("PNG", 12, 8, "#3366cc")
    jpg = image_bytes("JPEG", 20, 10, "#cc6633")
    gif = GIF_1PX  # Qt reads GIF but can't write it
    bmp = image_bytes("BMP", 5, 7, "#aa00aa")

    files = []

    def add(name):
        files.append(name)
        return out / name

    # FLAC: everything, a FLAC picture block behind a non-front picture, multi-value artist.
    p = add("full.flac")
    ffmpeg(p, ["-c:a", "flac", "-sample_fmt", "s16"])
    a = FLAC(p)
    a["TITLE"] = "Flac Title"
    a["ARTIST"] = ["Artist One", "Artist Two"]
    a["ALBUM"] = "The Album"
    a["ALBUMARTIST"] = "Album Artist"
    a["TRACKNUMBER"] = "5/12"
    a["DISCNUMBER"] = "1/2"
    a["DATE"] = "2021-03-04"
    a["GENRE"] = "Pop"
    a["UNSYNCEDLYRICS"] = PLAIN
    a["LYRICS"] = SYNCED
    a.add_picture(flac_picture(bmp, 4, "image/bmp", "back", 5, 7))
    a.add_picture(flac_picture(png, 3, "image/png", "front", 12, 8))
    a.save()

    # FLAC 24-bit, 48 kHz, no tags at all; a cover.jpg next to it is the fallback.
    (out / "folder_cover").mkdir(exist_ok=True)
    p = add("folder_cover/hires.flac")
    ffmpeg(p, ["-c:a", "flac", "-sample_fmt", "s32", "-bits_per_raw_sample", "24"], rate=48000, channels=1)
    (out / "folder_cover" / "cover.jpg").write_bytes(jpg)

    # FLAC whose only picture is SVG: refused, and folder.png is used instead.
    (out / "svg_cover").mkdir(exist_ok=True)
    p = add("svg_cover/svg.flac")
    ffmpeg(p, ["-c:a", "flac", "-sample_fmt", "s16"])
    a = FLAC(p)
    a["title"] = "  spaced title  "
    a["tracknumber"] = "07"
    a.add_picture(flac_picture(SVG, 3, "image/png", "fake png"))
    a.save()
    (out / "svg_cover" / "folder.png").write_bytes(png)
    (out / "svg_cover" / "cover.png").write_bytes(SVG)  # checked first, refused

    # MP3, ID3v2.4: USLT lyrics, a COMM, TXXX, APIC front + other, PRIV.
    p = add("full.mp3")
    ffmpeg(p, ["-c:a", "libmp3lame", "-b:a", "128k"])
    t = ID3()
    t.add(TIT2(encoding=3, text=["Mp3 Title"]))
    t.add(TPE1(encoding=3, text=["Mp3 Artist"]))
    t.add(TALB(encoding=3, text=["Mp3 Album"]))
    t.add(TPE2(encoding=3, text=["Mp3 Album Artist"]))
    t.add(TRCK(encoding=3, text=["03/10"]))
    t.add(TPOS(encoding=3, text=["2"]))
    t.add(TDRC(encoding=3, text=["1999-12-31"]))
    t.add(TCON(encoding=3, text=["Rock"]))
    t.add(COMM(encoding=3, lang="eng", desc="", text=["a comment"]))
    t.add(TXXX(encoding=3, desc="MOOD", text=["calm"]))
    t.add(PRIV(owner="x", data=b"[00:12 not lyrics"))
    t.add(USLT(encoding=3, lang="eng", desc="", text=SYNCED))
    t.add(APIC(encoding=3, mime="image/gif", type=0, desc="other", data=gif))
    t.add(APIC(encoding=3, mime="image/jpeg", type=3, desc="front", data=jpg))
    t.save(p, v2_version=4)

    # MP3, ID3v2.3 with TYER and lyrics in a TXXX frame.
    p = add("v23.mp3")
    ffmpeg(p, ["-c:a", "libmp3lame", "-b:a", "96k"], rate=32000, channels=1)
    t = ID3()
    t.add(TIT2(encoding=1, text=["Old Title"]))
    t.add(TYER(encoding=1, text=["1987"]))
    t.add(TXXX(encoding=1, desc="LYRICS", text=[PLAIN]))
    t.add(TRCK(encoding=1, text=["0"]))
    t.update_to_v23()
    t.save(p, v2_version=3)

    # MP3 with only an ID3v1 tag.
    p = add("v1only.mp3")
    ffmpeg(p, ["-c:a", "libmp3lame", "-b:a", "64k"])
    v1 = bytearray(b"TAG" + b"V1 Title".ljust(30, b"\0") + b"V1 Artist".ljust(30, b"\0") +
                   b"V1 Album".ljust(30, b"\0") + b"2001" + b"v1 comment".ljust(28, b"\0") + b"\0\x07" +
                   bytes([17]))
    with open(p, "ab") as f:
        f.write(v1)

    # MP3 with ID3v2 and ID3v1: v1 fills what v2 lacks; a COMM with a timestamp is "lyrics".
    p = add("v1v2.mp3")
    ffmpeg(p, ["-c:a", "libmp3lame", "-b:a", "128k"])
    t = ID3()
    t.add(TIT2(encoding=3, text=["V2 Title"]))
    t.add(COMM(encoding=3, lang="eng", desc="note", text=["see [01:23] for the solo"]))
    t.save(p, v2_version=4)
    with open(p, "ab") as f:
        f.write(v1)

    # MP3 without any tag: the title is the file name.
    p = add("01 - No Tags.mp3")
    ffmpeg(p, ["-c:a", "libmp3lame", "-b:a", "128k"])

    # M4A: track and disc pairs, ©lyr, a freeform atom, a PNG cover.
    p = add("full.m4a")
    ffmpeg(p, ["-c:a", "aac", "-b:a", "128k"])
    a = MP4(p)
    a["\xa9nam"] = ["M4a Title"]
    a["\xa9ART"] = ["M4a Artist"]
    a["\xa9alb"] = ["M4a Album"]
    a["aART"] = ["M4a Album Artist"]
    a["trkn"] = [(5, 12)]
    a["disk"] = [(1, 0)]
    a["\xa9day"] = ["2010"]
    a["\xa9gen"] = ["Jazz"]
    a["\xa9lyr"] = [PLAIN]
    a["cpil"] = True
    a["tmpo"] = [120]
    a["----:com.apple.iTunes:MOOD"] = [MP4FreeForm(b"happy")]
    a["covr"] = [MP4Cover(png, imageformat=MP4Cover.FORMAT_PNG)]
    a.save()

    # M4A with a track number of 0 and no cover.
    p = add("zero.m4a")
    ffmpeg(p, ["-c:a", "aac", "-b:a", "96k"], rate=48000)
    a = MP4(p)
    a["trkn"] = [(0, 9)]
    a.save()

    # OGG Vorbis: synced lyrics under UNSYNCEDLYRICS, a front cover in METADATA_BLOCK_PICTURE.
    p = add("full.ogg")
    ffmpeg(p, ["-c:a", "libvorbis", "-q:a", "3"])
    a = OggVorbis(p)
    a["title"] = "Ogg Title"
    a["artist"] = "Ogg Artist"
    a["tracknumber"] = "2"
    a["year"] = "2005"
    a["unsyncedlyrics"] = SYNCED
    pic = flac_picture(jpg, 3, "image/jpeg", "", 20, 10)
    a["metadata_block_picture"] = [base64.b64encode(pic.write()).decode("ascii")]
    a.save()

    # Opus: plain LYRICS, performer instead of artist.
    p = add("full.opus")
    ffmpeg(p, ["-c:a", "libopus", "-b:a", "64k"], rate=48000)
    a = OggOpus(p)
    a["TITLE"] = "Opus Title"
    a["PERFORMER"] = "Opus Performer"
    a["DISCNUMBER"] = "3"
    a["LYRICS"] = PLAIN
    a.save()

    # WAV with an ID3 chunk.
    p = add("full.wav")
    ffmpeg(p, ["-c:a", "pcm_s16le"], seconds=1.5)
    a = WAVE(p)
    a.add_tags()
    a.tags.add(TIT2(encoding=3, text=["Wav Title"]))
    a.tags.add(TPE1(encoding=3, text=["Wav Artist"]))
    a.tags.add(TRCK(encoding=3, text=["4"]))
    a.tags.add(APIC(encoding=3, mime="image/png", type=3, desc="", data=png))
    a.save()

    # WAV without tags, 24-bit.
    p = add("plain24.wav")
    ffmpeg(p, ["-c:a", "pcm_s24le"], seconds=1.25, rate=96000)

    # Not a song at all.
    p = add("broken.mp3")
    p.write_text("this is a text file renamed to .mp3", encoding="utf-8")

    return files


def describe(path: Path, root: Path):
    lyrics, cover, info = L.extract_metadata_and_cover(str(path))
    info = dict(info)
    info["file_path"] = path.relative_to(root).as_posix()
    tags = L.read_track_tags(str(path))
    return {
        "file": path.relative_to(root).as_posix(),
        "track_tags": tags,
        "lyrics": lyrics,
        "cover": [cover.width(), cover.height()] if cover else None,
        "info": info,
    }


def cmd_media(out: Path):
    out.mkdir(parents=True, exist_ok=True)
    files = make_media(out)
    with quiet():
        entries = [describe(out / f, out) for f in files]
        scan = L.scan_album_folder(str(out), tuple(L.AudioLRCPlayer.SUPPORTED_EXTS))
    data = {
        "_comment": "Written by tools/make_fixtures.py media from desktop/lrcplayer.py.",
        "embed_text": EMBED_TEXT,
        "files": entries,
        "scan": [[Path(p).relative_to(out).as_posix(), t] for p, t in scan],
    }
    (out / "media.json").write_text(json.dumps(data, ensure_ascii=False, indent=1), encoding="utf-8")
    print(f"wrote {len(entries)} songs and media.json to {out}")


def cmd_check_embedded(out: Path):
    data = json.loads((out / "media.json").read_text(encoding="utf-8"))
    embedded = out / "embedded"
    rust = json.loads((embedded / "rust.json").read_text(encoding="utf-8"))
    failures, checked = [], 0
    for entry in data["files"]:
        name = entry["file"]
        target = embedded / name
        if not target.exists():
            continue  # Rust refused this one (the broken file); its test checks that
        checked += 1
        with quiet():
            got = describe(target, embedded)
        want = rust.get(name)
        if want is None:
            failures.append(f"{name}: missing from rust.json")
            continue
        for key in ("track_tags", "lyrics", "cover", "info"):
            if got[key] != want[key]:
                failures.append(f"{name}: {key}\n    python {got[key]}\n    rust   {want[key]}")
        if name.endswith((".mp3", ".wav")):
            from mutagen.id3 import ID3
            from mutagen.wave import WAVE
            tags = WAVE(target).tags if name.endswith(".wav") else ID3(target)
            uslt = tags.getall("USLT")
            if (tags.version[:2] != (2, 4) or len(uslt) != 1 or uslt[0].lang != "eng"
                    or uslt[0].desc != "" or uslt[0].encoding != 3):
                failures.append(f"{name}: expected one UTF-8 USLT (eng, no description) in ID3v2.4, "
                                f"got v{tags.version} {uslt!r}")
        if name.endswith((".flac", ".ogg", ".opus")):
            import mutagen
            tags = mutagen.File(target).tags
            if "UNSYNCEDLYRICS" in tags:
                failures.append(f"{name}: UNSYNCEDLYRICS is still there")
        if name.endswith(".m4a"):
            from mutagen.mp4 import MP4
            if MP4(target).tags.get("\xa9lyr") != [data["embed_text"]]:
                failures.append(f"{name}: ©lyr is not the embedded text")
    if not checked:
        sys.exit(f"nothing to check in {embedded}: run cargo test with LRC_EMBED_OUT={embedded}")
    if failures:
        sys.exit("mutagen disagrees with what lofty wrote:\n  " + "\n  ".join(failures))
    print(f"mutagen reads back all {checked} songs that lrc-core embedded lyrics into")


def main():
    app = QGuiApplication.instance() or QGuiApplication([])  # QPixmap needs one  # noqa: F841
    args = sys.argv[1:]
    if args[:1] == ["pure"]:
        cmd_pure("--check" in args)
    elif args[:1] == ["media"] and len(args) == 2:
        cmd_media(Path(args[1]).resolve())
    elif args[:1] == ["check-embedded"] and len(args) == 2:
        cmd_check_embedded(Path(args[1]).resolve())
    else:
        sys.exit(__doc__)


if __name__ == "__main__":
    main()
