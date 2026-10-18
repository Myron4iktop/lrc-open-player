import os
import re
import sys
import json
import math
import time
import array
import base64
import cmath
import types
import ctypes
import random
import socket
import tempfile
import http.client
from collections import deque
import urllib.request
import urllib.parse
from datetime import timedelta
from pathlib import Path
from PyQt6.QtCore import (
    Qt, QUrl, QTime, QSize, QThread, QObject, QSettings, pyqtSignal, QTimer,
    QPropertyAnimation, QEasingCurve, pyqtProperty, QPointF, QRectF
)
from PyQt6.QtWidgets import (
    QApplication, QMainWindow, QWidget, QVBoxLayout, QHBoxLayout,
    QPushButton, QSlider, QLabel, QFileDialog, QScrollArea, QFrame,
    QSizePolicy, QDialog, QGridLayout, QGraphicsOpacityEffect, QListWidget, QListWidgetItem,
    QMessageBox, QMenu, QStackedWidget, QPlainTextEdit, QSystemTrayIcon,
    QLineEdit, QCheckBox, QTableWidget, QTableWidgetItem, QHeaderView, QProgressBar,
    QAbstractItemView, QStyledItemDelegate, QCompleter, QButtonGroup
)
from PyQt6.QtGui import (
    QFont, QPixmap, QImage, QIcon, QPainter, QColor, QPolygonF, QKeySequence,
    QPen, QPainterPath, QLinearGradient, QActionGroup, QDesktopServices
)
from PyQt6.QtMultimedia import QMediaPlayer, QAudioOutput, QMediaDevices
try:  # Qt 6.8+: lets the visualizer see the decoded audio
    from PyQt6.QtMultimedia import QAudioBufferOutput, QAudioFormat
except ImportError:
    QAudioBufferOutput = None
    QAudioFormat = None


# ---------------------------------------------------------
# Metadata & Track Helpers
# ---------------------------------------------------------
def get_track_sort_key(file_path: str):
    """Sorts album files by (Disc Number, Track Number, Filename)."""
    disc_no = 1
    track_no = 999
    try:
        import mutagen
        audio = mutagen.File(file_path)
        if audio:
            def parse_num(keys):
                for k in keys:
                    v = audio.get(k) or (audio.tags.get(k) if hasattr(audio, "tags") and audio.tags else None)
                    if v:
                        raw = str(v[0] if isinstance(v, list) else getattr(v, "text", str(v)))
                        m = re.match(r"^(\d+)", raw.strip())
                        if m:
                            return int(m.group(1))
                return None

            d = parse_num(["discnumber", "tpos", "disk"])
            if d is not None:
                disc_no = d
            t = parse_num(["tracknumber", "trck", "trkn"])
            if t is not None:
                track_no = t
    except Exception:
        pass
    return (disc_no, track_no, os.path.basename(file_path).lower())


def extract_metadata_and_cover(file_path: str):
    """Safely extracts audio specifications, tags, lyrics, and album art with isolated error handling."""
    lyrics = None
    cover_pixmap = None
    info = {
        "title": os.path.splitext(os.path.basename(file_path))[0],
        "artist": "Unknown Artist",
        "album": "Unknown Album",
        "album_artist": "Unknown Album Artist",
        "track_number": "-",
        "disc_number": "-",
        "genre": "Unknown Genre",
        "year": "-",
        "duration_sec": 0,
        "duration_str": "0:00",
        "mime_type": "audio/unknown",
        "bitrate_kbps": 0,
        "sample_rate_khz": 0.0,
        "bits_per_sample": 16,
        "channels": 2,
        "is_hd": False,
        "file_path": file_path
    }

    try:
        import mutagen
        audio = mutagen.File(file_path)
    except Exception as e:
        print(f"Error opening audio file: {e}")
        return lyrics, cover_pixmap, info

    if audio is None:
        return lyrics, cover_pixmap, info

    # 1. Technical Stream Information
    try:
        if hasattr(audio, "info") and audio.info:
            ainfo = audio.info
            sr = getattr(ainfo, "sample_rate", 44100)
            info["sample_rate_khz"] = round(sr / 1000.0, 1)
            info["channels"] = getattr(ainfo, "channels", 2)

            bps = getattr(ainfo, "bits_per_sample", 0)
            if bps == 0:
                ext = os.path.splitext(file_path)[1].lower()
                bps = 16 if ext in (".mp3", ".ogg", ".m4a") else 24
            info["bits_per_sample"] = bps

            br = getattr(ainfo, "bitrate", 0)
            if br and br > 0:
                info["bitrate_kbps"] = int(br / 1000)
            else:
                length = getattr(ainfo, "length", 0)
                if length > 0:
                    file_bytes = os.path.getsize(file_path)
                    info["bitrate_kbps"] = int((file_bytes * 8) / (length * 1000))

            length_sec = int(getattr(ainfo, "length", 0))
            info["duration_sec"] = length_sec
            m, s = divmod(length_sec, 60)
            info["duration_str"] = f"{m}:{s:02d}"

            if bps >= 24 or sr >= 48000 or info["bitrate_kbps"] >= 1000:
                info["is_hd"] = True

        ext = os.path.splitext(file_path)[1].lower()
        mime_map = {
            ".flac": "audio/flac",
            ".mp3": "audio/mpeg",
            ".m4a": "audio/mp4",
            ".ogg": "audio/ogg",
            ".opus": "audio/opus",
            ".wav": "audio/wav"
        }
        info["mime_type"] = mime_map.get(ext, "audio/unknown")
    except Exception as e:
        print(f"Stream specs parsing error: {e}")

    # 2. Extract Raw Tag Pairs
    tag_pairs = []
    try:
        if hasattr(audio, "tags") and audio.tags is not None:
            for k in list(audio.tags.keys()):
                try:
                    val = audio.tags[k]
                    if isinstance(val, list):
                        text_val = "\n".join(str(x) for x in val if x is not None)
                    elif hasattr(val, "text"):
                        text_val = "\n".join(str(x) for x in val.text) if isinstance(val.text, list) else str(val.text)
                    else:
                        text_val = str(val)
                    tag_pairs.append((str(k).lower(), text_val))
                except Exception:
                    continue
        elif hasattr(audio, "keys"):
            for k in list(audio.keys()):
                try:
                    val = audio[k]
                    text_val = "\n".join(str(x) for x in val if x is not None) if isinstance(val, list) else str(val)
                    tag_pairs.append((str(k).lower(), text_val))
                except Exception:
                    continue
    except Exception as e:
        print(f"Tag collection error: {e}")

    def find_tag_value(possible_keys):
        for p_key in possible_keys:
            p_key_lower = p_key.lower()
            for k, val in tag_pairs:
                if k == p_key_lower:
                    clean_val = val.strip()
                    if clean_val:
                        return clean_val
        return None

    t_val = find_tag_value(["title", "tit2", "\xa9nam"])
    if t_val: info["title"] = t_val

    a_val = find_tag_value(["artist", "tpe1", "\xa9art", "performer"])
    if a_val: info["artist"] = a_val

    alb_val = find_tag_value(["album", "talb", "\xa9alb"])
    if alb_val: info["album"] = alb_val

    aa_val = find_tag_value(["albumartist", "album artist", "tpe2", "aart"])
    if aa_val: info["album_artist"] = aa_val

    tr_val = find_tag_value(["tracknumber", "trck", "trkn"])
    if tr_val: info["track_number"] = tr_val

    disc_val = find_tag_value(["discnumber", "tpos", "disk"])
    if disc_val: info["disc_number"] = disc_val

    g_val = find_tag_value(["genre", "tcon", "\xa9gen"])
    if g_val: info["genre"] = g_val

    y_val = find_tag_value(["date", "year", "tdrc", "tyer", "\xa9day"])
    if y_val: info["year"] = y_val[:4]

    # 3. Synchronized Lyrics Extraction
    try:
        timestamp_pattern = re.compile(r"\[\d{1,2}:\d{2}")
        for _, text in tag_pairs:
            if timestamp_pattern.search(text):
                lyrics = text
                break

        if not lyrics:
            for k, text in tag_pairs:
                if "lyric" in k or "\xa9lyr" in k or "unsynced" in k:
                    lyrics = text
                    break
    except Exception as e:
        print(f"Lyrics extraction error: {e}")

    # 4. Embedded Cover Art Extraction
    try:
        if hasattr(audio, "pictures") and audio.pictures:
            for pic in audio.pictures:
                qimg = QImage()
                if qimg.loadFromData(pic.data):
                    cover_pixmap = QPixmap.fromImage(qimg)
                    if getattr(pic, "type", None) == 3:
                        break
        elif hasattr(audio, "tags") and audio.tags:
            for frame in audio.tags.values():
                if getattr(frame, "FrameID", "") == "APIC":
                    qimg = QImage()
                    if qimg.loadFromData(frame.data):
                        cover_pixmap = QPixmap.fromImage(qimg)
                        break
        elif hasattr(audio, "get") and audio.get("covr"):
            covers = audio.get("covr")
            if covers:
                qimg = QImage()
                if qimg.loadFromData(bytes(covers[0])):
                    cover_pixmap = QPixmap.fromImage(qimg)
    except Exception as e:
        print(f"Cover art extraction error: {e}")

    if not cover_pixmap:
        folder = os.path.dirname(file_path)
        for name in ("cover.jpg", "cover.png", "folder.jpg", "folder.png"):
            path = os.path.join(folder, name)
            if os.path.exists(path):
                cover_pixmap = QPixmap(path)
                break

    return lyrics, cover_pixmap, info


# ---------------------------------------------------------
# LRCLIB Network Fetcher (Worker Thread)
# ---------------------------------------------------------
def clean_query_string(text: str) -> str:
    cleaned = re.sub(r"^\d+[\s.-]+", "", text)
    cleaned = re.sub(r"\s*\(\d+\)$", "", cleaned)
    cleaned = re.sub(r"\[(FLAC|MP3|WAV|HQ)\]", "", cleaned, flags=re.IGNORECASE)
    return cleaned.strip()


# ---------------------------------------------------------
# Kanji / kana -> romaji (optional: pip install "cutlet" "unidic-lite")
# ---------------------------------------------------------
_JP_CHARS = re.compile(r"[\u3040-\u30ff\u3400-\u4dbf\u4e00-\u9fff\uff66-\uff9f]")
_KANJI_CHARS = re.compile(r"[\u3400-\u4dbf\u4e00-\u9fff]")


def _same_text(a: str, b: str) -> bool:
    """True if two lines only differ in case, spacing or punctuation."""
    norm = lambda t: re.sub(r"[\W_]+", "", t.casefold())
    return norm(a) == norm(b)


class RomajiConverter:
    """Context-aware Japanese -> Hepburn romaji using cutlet (fugashi + UniDic).

    A morphological analyzer is required because kanji readings depend on
    context (e.g. 生 = sei / nama / i / u). Loaded lazily: the dictionary is
    large and the import is slow, so nothing happens until first use.
    """

    def __init__(self):
        self._katsu = None
        self._cache = {}
        self.error = ""

    def load(self) -> bool:
        if self._katsu is not None:
            return True
        try:
            import cutlet
            self._katsu = cutlet.Cutlet()  # Hepburn; は->wa, を->o, long vowels handled
            self._katsu.use_foreign_spelling = False
            self.error = ""
            return True
        except Exception as e:
            self.error = str(e) or e.__class__.__name__
            return False

    @staticmethod
    def has_japanese(text: str) -> bool:
        return bool(_JP_CHARS.search(text))

    def convert(self, text: str) -> str:
        """Romaji for a lyric line, or "" if it has no Japanese / conversion fails."""
        if not self.has_japanese(text) or not self.load():
            return ""
        if text in self._cache:
            return self._cache[text]
        try:
            result = self._katsu.romaji(text).strip()
        except Exception:
            result = ""
        if result == text:
            result = ""
        self._cache[text] = result
        return result


class LrcFetchWorker(QThread):
    finished_signal = pyqtSignal(str, str)

    def __init__(self, title: str, artist: str, duration_sec: int, save_path: str = ""):
        super().__init__()
        self.title = clean_query_string(title)
        self.artist = "" if artist == "Unknown Artist" else clean_query_string(artist)
        self.duration_sec = duration_sec
        self.save_path = save_path

    def run(self):
        headers = {"User-Agent": "LRCOpenPlayer/3.0 (Desktop; Python/PyQt6)"}

        params = {"track_name": self.title}
        if self.artist:
            params["artist_name"] = self.artist
        if self.duration_sec > 0:
            params["duration"] = self.duration_sec

        url = f"https://lrclib.net/api/get?{urllib.parse.urlencode(params)}"
        req = urllib.request.Request(url, headers=headers)

        try:
            with urllib.request.urlopen(req, timeout=6) as response:
                if response.status == 200:
                    data = json.loads(response.read().decode("utf-8"))
                    synced = data.get("syncedLyrics")
                    if synced:
                        self._save_cache(synced)
                        self.finished_signal.emit(synced, "Synced lyrics downloaded from LRCLIB.")
                        return
                    plain = data.get("plainLyrics")
                    if plain:
                        self.finished_signal.emit(plain, "Plain lyrics downloaded from LRCLIB.")
                        return
        except Exception:
            pass

        search_terms = f"{self.title} {self.artist}".strip()
        search_url = f"https://lrclib.net/api/search?{urllib.parse.urlencode({'q': search_terms})}"
        req_search = urllib.request.Request(search_url, headers=headers)

        try:
            with urllib.request.urlopen(req_search, timeout=7) as response:
                if response.status == 200:
                    results = json.loads(response.read().decode("utf-8"))
                    if results and isinstance(results, list):
                        for item in results:
                            if item.get("syncedLyrics"):
                                if self.duration_sec > 0 and abs(item.get("duration", 0) - self.duration_sec) <= 4:
                                    self._save_cache(item["syncedLyrics"])
                                    self.finished_signal.emit(item["syncedLyrics"], "Synced lyrics matched on LRCLIB.")
                                    return
                        for item in results:
                            if item.get("syncedLyrics"):
                                self._save_cache(item["syncedLyrics"])
                                self.finished_signal.emit(item["syncedLyrics"], "Synced lyrics matched on LRCLIB.")
                                return
                        for item in results:
                            if item.get("plainLyrics"):
                                self.finished_signal.emit(item["plainLyrics"], "Plain lyrics found on LRCLIB.")
                                return
        except Exception as e:
            print(f"LRCLIB Search error: {e}")

        self.finished_signal.emit("", "No lyrics found online.")

    def _save_cache(self, lyrics_text: str):
        if self.save_path:
            try:
                with open(self.save_path, "w", encoding="utf-8") as f:
                    f.write(lyrics_text)
            except Exception as e:
                print(f"Could not cache LRC file: {e}")


# ---------------------------------------------------------
# Windows System Media Transport Controls (SMTC)
# ---------------------------------------------------------
class WindowsMediaControls(QObject):
    """
    Bridges the player to the Windows media flyout, lock screen and hardware /
    headset media keys via SystemMediaTransportControls.

    Requires (Windows only):
        pip install winrt-runtime winrt-Windows.Foundation winrt-Windows.Foundation.Collections
                    winrt-Windows.Media winrt-Windows.Media.Playback winrt-Windows.Storage.Streams

    If the packages are missing or the platform isn't Windows, every method is a
    harmless no-op and `available` is False.

    WinRT invokes our handlers on a thread-pool thread, so they only emit Qt
    signals; the window connects those to slots that run on the GUI thread.
    """
    play_requested = pyqtSignal()
    pause_requested = pyqtSignal()
    next_requested = pyqtSignal()
    previous_requested = pyqtSignal()
    seek_requested = pyqtSignal(int)  # milliseconds

    def __init__(self, parent=None):
        super().__init__(parent)
        self.available = False
        self._smtc = None
        self._media_player = None
        self._winrt = None
        self._handler_tokens = []
        self._last_timeline_push = 0.0
        self._cover_file = None
        self._cover_counter = 0

        if sys.platform != "win32":
            return

        try:
            from winrt.windows.media import (
                MediaPlaybackStatus, MediaPlaybackType,
                SystemMediaTransportControlsButton,
                SystemMediaTransportControlsTimelineProperties,
            )
            from winrt.windows.media.playback import MediaPlayer
            from winrt.windows.storage.streams import RandomAccessStreamReference
            from winrt.windows.foundation import Uri
        except ImportError:
            print("SMTC disabled: install the winrt packages (see WindowsMediaControls docstring).")
            return

        self._winrt = types.SimpleNamespace(
            Status=MediaPlaybackStatus,
            MediaType=MediaPlaybackType,
            Button=SystemMediaTransportControlsButton,
            Timeline=SystemMediaTransportControlsTimelineProperties,
            StreamRef=RandomAccessStreamReference,
            Uri=Uri,
        )

        try:
            # A MediaPlayer is only used as a vehicle to obtain an SMTC instance
            # in a desktop (non-UWP) app. Audio is still played by QMediaPlayer.
            self._media_player = MediaPlayer()
            self._media_player.command_manager.is_enabled = False

            smtc = self._media_player.system_media_transport_controls
            smtc.is_enabled = True
            smtc.is_play_enabled = True
            smtc.is_pause_enabled = True
            smtc.is_next_enabled = False
            smtc.is_previous_enabled = False
            smtc.playback_status = self._winrt.Status.CLOSED

            self._handler_tokens.append(
                ("button_pressed", smtc.add_button_pressed(self._on_button_pressed)))
            self._handler_tokens.append(
                ("playback_position_change_requested",
                 smtc.add_playback_position_change_requested(self._on_position_change_requested)))

            self._smtc = smtc
            self.available = True
        except Exception as e:
            print(f"SMTC initialisation failed: {e}")
            self._smtc = None
            self.available = False

    # ---- WinRT -> Qt (runs on a non-GUI thread; only emit signals) ----
    def _on_button_pressed(self, sender, args):
        try:
            b = args.button
            B = self._winrt.Button
            if b == B.PLAY:
                self.play_requested.emit()
            elif b in (B.PAUSE, B.STOP):
                self.pause_requested.emit()
            elif b == B.NEXT:
                self.next_requested.emit()
            elif b == B.PREVIOUS:
                self.previous_requested.emit()
        except Exception as e:
            print(f"SMTC button handler error: {e}")

    def _on_position_change_requested(self, sender, args):
        try:
            ms = int(args.requested_playback_position.total_seconds() * 1000)
            self.seek_requested.emit(max(0, ms))
        except Exception as e:
            print(f"SMTC seek handler error: {e}")

    # ---- Qt -> Windows ----
    @staticmethod
    def _clean(value, placeholder):
        return "" if not value or value == placeholder else str(value)

    def set_track(self, info: dict, cover_pixmap, has_previous: bool, has_next: bool):
        if not self.available:
            return
        try:
            W = self._winrt
            updater = self._smtc.display_updater
            updater.type = W.MediaType.MUSIC

            props = updater.music_properties
            props.title = info.get("title") or ""
            props.artist = self._clean(info.get("artist"), "Unknown Artist")
            props.album_title = self._clean(info.get("album"), "Unknown Album")
            props.album_artist = self._clean(info.get("album_artist"), "Unknown Album Artist")
            m = re.match(r"\d+", str(info.get("track_number", "")))
            props.track_number = int(m.group()) if m else 0

            previous_cover = self._cover_file
            cover_path = self._write_cover(cover_pixmap)
            if cover_path:
                updater.thumbnail = W.StreamRef.create_from_uri(W.Uri(Path(cover_path).as_uri()))
            else:
                updater.thumbnail = None
            updater.update()

            if previous_cover and previous_cover != cover_path:
                self._delete_file(previous_cover)

            self._smtc.is_previous_enabled = has_previous
            self._smtc.is_next_enabled = has_next
        except Exception as e:
            print(f"SMTC set_track error: {e}")

    def set_navigation(self, has_previous: bool, has_next: bool):
        if not self.available:
            return
        try:
            self._smtc.is_previous_enabled = has_previous
            self._smtc.is_next_enabled = has_next
        except Exception as e:
            print(f"SMTC navigation error: {e}")

    def set_playback_state(self, state):
        if not self.available:
            return
        try:
            S = self._winrt.Status
            PS = QMediaPlayer.PlaybackState
            self._smtc.playback_status = {
                PS.PlayingState: S.PLAYING,
                PS.PausedState: S.PAUSED,
            }.get(state, S.STOPPED)
        except Exception as e:
            print(f"SMTC playback state error: {e}")

    def update_timeline(self, position_ms: int, duration_ms: int, force: bool = False):
        """Push position/duration. Throttled to ~1 Hz unless forced (seek, pause, new track)."""
        if not self.available:
            return
        now = time.monotonic()
        if not force and now - self._last_timeline_push < 1.0:
            return
        self._last_timeline_push = now
        try:
            duration_ms = max(0, int(duration_ms))
            position_ms = min(max(0, int(position_ms)), duration_ms)
            tl = self._winrt.Timeline()
            zero = timedelta(0)
            end = timedelta(milliseconds=duration_ms)
            tl.start_time = zero
            tl.min_seek_time = zero
            tl.end_time = end
            tl.max_seek_time = end
            tl.position = timedelta(milliseconds=position_ms)
            self._smtc.update_timeline_properties(tl)
        except Exception as e:
            print(f"SMTC timeline error: {e}")

    def shutdown(self):
        if not self.available:
            return
        try:
            for name, token in self._handler_tokens:
                getattr(self._smtc, f"remove_{name}")(token)
            self._handler_tokens.clear()
            self._smtc.playback_status = self._winrt.Status.CLOSED
            self._smtc.is_enabled = False
            self._media_player.close()
        except Exception as e:
            print(f"SMTC shutdown error: {e}")
        finally:
            self.available = False
            if self._cover_file:
                self._delete_file(self._cover_file)
                self._cover_file = None

    # ---- helpers ----
    def _write_cover(self, pixmap):
        """SMTC thumbnails are loaded from a URI; write the cover to a unique temp PNG."""
        if pixmap is None or pixmap.isNull():
            self._cover_file = None
            return None
        try:
            scaled = pixmap.scaled(
                512, 512, Qt.AspectRatioMode.KeepAspectRatio, Qt.TransformationMode.SmoothTransformation
            )
            self._cover_counter += 1
            path = os.path.join(
                tempfile.gettempdir(), f"lrcplayer_cover_{os.getpid()}_{self._cover_counter}.png"
            )
            if scaled.save(path, "PNG"):
                self._cover_file = path
                return path
        except Exception as e:
            print(f"SMTC cover write error: {e}")
        self._cover_file = None
        return None

    @staticmethod
    def _delete_file(path):
        try:
            os.remove(path)
        except OSError:
            pass


# ---------------------------------------------------------
# UI Components
# ---------------------------------------------------------
# ---------------------------------------------------------
# Themes: every color in the app comes from PAL
# ---------------------------------------------------------
ACCENTS = [
    ("coral", "Coral", "#E05A47"),
    ("rose", "Rose", "#E2577F"),
    ("amber", "Amber", "#E3A33B"),
    ("jade", "Jade", "#3FB27F"),
    ("aqua", "Aqua", "#34B4C8"),
    ("azure", "Azure", "#5B8FF0"),
    ("violet", "Violet", "#9B74E8"),
]
ACCENT_HEX = {key: hex_ for key, _, hex_ in ACCENTS}
THEMES = [("dark", "Dark"), ("light", "Light")]

PAL = {}  # the active palette; filled by set_palette()


def _hsl(h: float, s: float, l: float) -> str:
    return QColor.fromHslF((h % 360) / 360.0, max(0.0, min(1.0, s)), max(0.0, min(1.0, l))).name()


def build_palette(theme: str, accent: str) -> dict:
    """All UI colors for a theme + accent.

    Neutrals carry a faint tint of the accent's hue, so each accent feels like one
    coherent theme (with Coral this reproduces the original warm look).
    """
    base = QColor(ACCENT_HEX.get(accent, ACCENT_HEX["coral"]))
    h = max(0.0, base.hslHueF()) * 360
    sat = base.hslSaturationF()
    lig = base.lightnessF()
    T = lambda s_, l_: _hsl(h, s_, l_)  # same hue for tinted neutrals and accent shades

    if theme == "light":
        pl = min(lig, 0.44)  # deep enough for white text on primary buttons
        return dict(
            dark=False,
            bg=T(0.20, 0.962), dialog_bg=T(0.22, 0.972),
            surface=T(0.30, 0.990), surface2=T(0.16, 0.925), surface_border=T(0.14, 0.86),
            btn=T(0.30, 0.992), btn_hover=T(0.22, 0.935), btn_border=T(0.13, 0.84),
            btn_disabled=T(0.15, 0.945), btn_disabled_border=T(0.12, 0.895),
            menu=T(0.30, 0.992), menu_border=T(0.13, 0.84), divider=T(0.13, 0.89),
            hover=T(0.20, 0.935), groove=T(0.13, 0.855), badge_bg=T(0.18, 0.925),
            text=T(0.14, 0.10), text_soft=T(0.12, 0.17), title=T(0.20, 0.08),
            list_text=T(0.10, 0.26), muted=T(0.08, 0.42), dim=T(0.07, 0.47), faint=T(0.06, 0.56),
            faint2=T(0.06, 0.55), text_disabled=T(0.06, 0.70),
            handle="#FFFFFF", handle_border=T(0.12, 0.74),
            icon=T(0.14, 0.20), icon_muted=T(0.07, 0.52),
            accent=_hsl(h, sat, min(lig, 0.50)),
            artist=T(0.35, 0.40), badge_text=T(0.22, 0.38),
            sel=T(0.55, 0.905), sel_text=T(0.20, 0.10),
            toggle_bg=T(0.70, 0.90), toggle_border=T(0.45, 0.72), toggle_text=T(0.60, 0.30),
            primary=_hsl(h, sat * 0.9, pl), primary_hover=_hsl(h, sat * 0.9, pl - 0.05),
            primary_border=_hsl(h, sat * 0.9, pl - 0.08), primary_text="#FFFFFF",
            primary_disabled=T(0.25, 0.88), primary_disabled_text=T(0.06, 0.60),
            primary_disabled_border=T(0.20, 0.82),
            lyric_active=T(0.20, 0.08), lyric_active_sub=T(0.60, 0.40),
            lyric_inactive=T(0.12, 0.17), lyric_inactive_sub=T(0.25, 0.35),
            lyric_plain=T(0.12, 0.20), inactive_opacity=0.40,
            time_ok=T(0.55, 0.38), status=T(0.30, 0.38),
            error="#C7362B", warn_text="#8A1F16", warn_bg="#FBE3DF",
            overlay="rgba(255, 255, 255, 215)", viz_alpha=0.26,
        )

    return dict(
        dark=True,
        bg=T(0.11, 0.071), dialog_bg=T(0.13, 0.063),
        surface=T(0.13, 0.090), surface2=T(0.11, 0.108), surface_border=T(0.14, 0.155),
        btn=T(0.10, 0.118), btn_hover=T(0.10, 0.175), btn_border=T(0.09, 0.194),
        btn_disabled=T(0.11, 0.092), btn_disabled_border=T(0.12, 0.130),
        menu=T(0.13, 0.108), menu_border=T(0.14, 0.200), divider=T(0.12, 0.180),
        hover=T(0.14, 0.137), groove=T(0.13, 0.157), badge_bg=T(0.14, 0.114),
        text="#FFFFFF", text_soft=T(0.22, 0.895), title=T(1.0, 0.967),
        list_text=T(0.17, 0.80), muted=T(0.10, 0.61), dim=T(0.08, 0.51), faint=T(0.09, 0.45),
        faint2=T(0.07, 0.52), text_disabled=T(0.09, 0.34),
        handle="#FFFFFF", handle_border="#FFFFFF",
        icon=T(0.33, 0.92), icon_muted=T(0.08, 0.52),
        accent=base.name(),
        artist=T(0.28, 0.69), badge_text=T(0.20, 0.69),
        sel=T(0.22, 0.185), sel_text=T(1.0, 0.967),
        toggle_bg=T(0.35, 0.26), toggle_border=T(0.34, 0.40), toggle_text=T(1.0, 0.94),
        primary=T(0.49, 0.32), primary_hover=T(0.47, 0.38), primary_border=T(0.43, 0.44),
        primary_text=T(1.0, 0.97),
        primary_disabled=T(0.22, 0.185), primary_disabled_text=T(0.08, 0.52),
        primary_disabled_border=T(0.18, 0.24),
        lyric_active="#FFFFFF", lyric_active_sub=T(1.0, 0.87),
        lyric_inactive=T(0.22, 0.895), lyric_inactive_sub=T(0.20, 0.69),
        lyric_plain=T(0.18, 0.87), inactive_opacity=0.32,
        time_ok=T(0.58, 0.79), status=T(0.25, 0.72),
        error="#FF7A6E", warn_text="#FFB4A8", warn_bg="#3A1F1C",
        overlay="rgba(20, 16, 16, 225)", viz_alpha=0.34,
    )


def set_palette(theme: str, accent: str):
    PAL.clear()
    PAL.update(build_palette(theme, accent))
    PAL["theme"], PAL["accent_key"] = theme, accent


set_palette("dark", "coral")


def app_stylesheet(p: dict) -> str:
    return f"""
        QMainWindow, QWidget {{
            background-color: {p['bg']};
            color: {p['text']};
            font-family: 'Segoe UI', -apple-system, sans-serif;
        }}
        QPushButton {{
            background-color: {p['btn']};
            border: 1px solid {p['btn_border']};
            border-radius: 6px;
            padding: 7px 10px;
            font-size: 13px;
            font-weight: 600;
            color: {p['text']};
        }}
        QPushButton:hover {{ background-color: {p['btn_hover']}; }}
        QPushButton::menu-indicator {{ image: none; width: 0px; }}
        QPushButton#iconButton {{ padding: 0px; }}
        QPushButton#toggle:checked, QPushButton#iconToggle:checked {{
            background-color: {p['toggle_bg']};
            border: 1px solid {p['toggle_border']};
            color: {p['toggle_text']};
        }}
        QPushButton#iconToggle {{ padding: 0px; }}
        QPushButton#iconFlat {{ padding: 0px; background: transparent; border: none; }}
        QPushButton#iconFlat:hover {{ background-color: {p['btn_hover']}; }}
        QPushButton:disabled {{
            color: {p['text_disabled']};
            background-color: {p['btn_disabled']};
            border: 1px solid {p['btn_disabled_border']};
        }}
        QMenu {{
            background-color: {p['menu']};
            border: 1px solid {p['menu_border']};
            padding: 6px;
        }}
        QMenu::item {{
            padding: 7px 28px 7px 14px;
            border-radius: 5px;
            color: {p['text_soft']};
            font-size: 13px;
        }}
        QMenu::item:selected {{ background-color: {p['sel']}; color: {p['sel_text']}; }}
        QMenu::item:disabled {{ color: {p['text_disabled']}; }}
        QMenu::separator {{ height: 1px; background: {p['divider']}; margin: 5px 8px; }}
        QToolTip {{
            background-color: {p['menu']};
            color: {p['text_soft']};
            border: 1px solid {p['menu_border']};
            padding: 4px 6px;
        }}
        QSlider::groove:horizontal {{ height: 5px; background: {p['groove']}; border-radius: 2px; }}
        QSlider::sub-page:horizontal {{ background: {p['accent']}; border-radius: 2px; }}
        QSlider::handle:horizontal {{
            background: {p['handle']};
            border: 1px solid {p['handle_border']};
            width: 12px;
            margin: -5px 0;
            border-radius: 7px;
        }}
        QScrollArea {{ border: none; background-color: transparent; }}
        QWidget#lyricsViewport {{ background: transparent; }}
        QListWidget {{
            background-color: {p['surface']};
            border: 1px solid {p['surface_border']};
            border-radius: 8px;
            outline: none;
            padding: 4px;
        }}
        QListWidget::item {{ padding: 8px 10px; border-radius: 6px; color: {p['list_text']}; }}
        QListWidget::item:hover {{ background-color: {p['hover']}; }}
        QListWidget::item:selected {{ background-color: {p['sel']}; color: {p['sel_text']}; font-weight: bold; }}
        QLabel#trackTitle {{ font-size: 16px; font-weight: bold; color: {p['title']}; }}
        QLabel#trackArtist {{ font-size: 13px; font-weight: 500; color: {p['artist']}; }}
        QLabel#timeLabel {{ font-size: 12px; color: {p['muted']}; }}
        QLabel#dimLabel {{ font-size: 12px; color: {p['dim']}; }}
        QLabel#offsetValue {{ font-size: 12px; font-weight: 600; color: {p['text_soft']}; }}
        QLabel#panelTitle {{ font-size: 14px; font-weight: bold; color: {p['title']}; }}
        QLabel#specsBadge {{
            font-size: 11px;
            font-weight: 600;
            color: {p['badge_text']};
            background-color: {p['badge_bg']};
            border-radius: 4px;
            padding: 4px 8px;
        }}
        QLabel#coverArt {{
            background-color: {p['surface2']};
            border-radius: 14px;
            border: 1px solid {p['surface_border']};
            color: {p['muted']};
        }}
        QLabel#lyricsStatus {{ color: {p['artist']}; font-size: 16px; font-weight: 500; }}
        QLabel#lyricsEmpty {{ color: {p['faint']}; font-size: 16px; }}
        QLabel#dropOverlay {{
            background-color: {p['overlay']};
            border: 2px dashed {p['accent']};
            border-radius: 18px;
            color: {p['title']};
            font-size: 22px;
            font-weight: 600;
        }}
    """


_CHECK_IMAGES = {}


def _check_image(color: str) -> str:
    """A small check mark PNG for checkbox styling (style sheets can only use image files)."""
    if color not in _CHECK_IMAGES:
        pm = QPixmap(48, 48)
        pm.fill(Qt.GlobalColor.transparent)
        painter = QPainter(pm)
        painter.setRenderHint(QPainter.RenderHint.Antialiasing)
        pen = QPen(QColor(color), 6.0)
        pen.setCapStyle(Qt.PenCapStyle.RoundCap)
        pen.setJoinStyle(Qt.PenJoinStyle.RoundJoin)
        painter.setPen(pen)
        path = QPainterPath(QPointF(12, 25))
        path.lineTo(20, 33)
        path.lineTo(36, 15)
        painter.drawPath(path)
        painter.end()
        file = os.path.join(tempfile.gettempdir(), f"lrcplayer_check_{color.strip('#')}.png")
        try:
            pm.save(file, "PNG")
        except Exception:
            file = ""
        _CHECK_IMAGES[color] = file.replace("\\", "/")
    return _CHECK_IMAGES[color]


def dialog_stylesheet(p: dict) -> str:
    """Styles for the sync, translation and local-AI windows (buttons etc. come from app_stylesheet)."""
    return f"""
        QDialog {{ background-color: {p['dialog_bg']}; }}
        QStackedWidget, QStackedWidget > QWidget, QWidget#plainBox {{ background: transparent; }}
        QPlainTextEdit {{
            background-color: {p['surface']};
            border: 1px solid {p['surface_border']};
            border-radius: 8px;
            color: {p['text_soft']};
            font-size: 14px;
            padding: 8px;
            selection-background-color: {p['sel']};
            selection-color: {p['sel_text']};
        }}
        QLineEdit {{
            background-color: {p['surface']};
            border: 1px solid {p['surface_border']};
            border-radius: 6px;
            color: {p['text_soft']};
            font-size: 13px;
            padding: 6px 8px;
            selection-background-color: {p['sel']};
            selection-color: {p['sel_text']};
        }}
        QPlainTextEdit:focus, QLineEdit:focus {{ border: 1px solid {p['accent']}; }}
        QLineEdit:disabled {{ color: {p['text_disabled']}; background-color: {p['btn_disabled']}; }}
        QCheckBox {{ color: {p['text_soft']}; spacing: 9px; font-size: 13px; background: transparent; }}
        QCheckBox::indicator {{
            width: 16px;
            height: 16px;
            border: 1px solid {p['btn_border']};
            border-radius: 4px;
            background-color: {p['surface']};
        }}
        QCheckBox::indicator:hover {{ border-color: {p['accent']}; }}
        QCheckBox::indicator:checked {{
            background-color: {p['primary']};
            border-color: {p['primary_border']};
            image: url("{_check_image(p['primary_text'])}");
        }}
        QTableWidget {{
            background-color: {p['surface']};
            border: 1px solid {p['surface_border']};
            border-radius: 8px;
            color: {p['text_soft']};
            font-size: 13px;
            outline: none;
            selection-background-color: {p['sel']};
            selection-color: {p['sel_text']};
        }}
        QTableWidget::item {{ padding: 6px 8px; border-bottom: 1px solid {p['divider']}; }}
        QTableWidget::item:selected {{ background-color: {p['sel']}; color: {p['sel_text']}; }}
        QTableWidget QLineEdit {{ border-radius: 0px; padding: 2px 6px; }}
        QHeaderView::section {{
            background-color: {p['surface']};
            color: {p['muted']};
            border: none;
            border-bottom: 1px solid {p['surface_border']};
            padding: 7px 8px;
            font-size: 12px;
            font-weight: 600;
        }}
        QTableCornerButton::section {{ background-color: {p['surface']}; border: none; }}
        QPushButton#segment {{ padding: 7px 16px; }}
        QPushButton#segment:checked {{
            background-color: {p['toggle_bg']};
            border: 1px solid {p['toggle_border']};
            color: {p['toggle_text']};
        }}
        QPushButton#link {{ background: transparent; border: none; color: {p['accent']}; padding: 4px 6px; }}
        QPushButton#link:hover {{ background-color: {p['btn_hover']}; }}
        QProgressBar {{ background-color: {p['groove']}; border: none; border-radius: 3px; }}
        QProgressBar::chunk {{ background-color: {p['accent']}; border-radius: 3px; }}
        #syncRow, #syncRow QLabel {{ background: transparent; }}
        QListWidget#syncList::item {{ padding: 0px; margin: 1px 0px; }}
        QLabel {{ background: transparent; }}
        QLabel#dlgHeading {{ font-size: 16px; font-weight: 600; color: {p['title']}; }}
        QLabel#syncMuted {{ color: {p['muted']}; }}
        QLabel#syncProgress {{ color: {p['muted']}; padding: 0 12px; }}
        QLabel#syncTime {{ color: {p['text_soft']}; padding-left: 8px; }}
        QLabel#syncHelp {{ color: {p['faint']}; font-size: 11px; }}
        QLabel#syncStatus {{ color: {p['status']}; }}
        QLabel#syncError {{ color: {p['error']}; }}
        QLabel#syncWarn {{
            color: {p['warn_text']};
            background-color: {p['warn_bg']};
            border-radius: 6px;
            padding: 8px;
        }}
        QPushButton#primary {{
            background-color: {p['primary']};
            border: 1px solid {p['primary_border']};
            color: {p['primary_text']};
            font-weight: bold;
            padding: 9px 18px;
        }}
        QPushButton#primary:hover {{ background-color: {p['primary_hover']}; }}
        QPushButton#primary:disabled {{
            background-color: {p['primary_disabled']};
            color: {p['primary_disabled_text']};
            border-color: {p['primary_disabled_border']};
        }}
    """


def set_role(widget, name: str):
    """Changes a widget's style role (objectName) and re-applies the stylesheet to it."""
    if widget.objectName() != name:
        widget.setObjectName(name)
        widget.style().unpolish(widget)
        widget.style().polish(widget)


def _icon_canvas(size: int):
    scale = 3  # draw at 3x, Qt scales down smoothly (sharp on high-DPI screens too)
    pm = QPixmap(size * scale, size * scale)
    pm.fill(Qt.GlobalColor.transparent)
    p = QPainter(pm)
    p.setRenderHint(QPainter.RenderHint.Antialiasing)
    p.scale(size * scale / 24.0, size * scale / 24.0)  # shapes use a 24x24 grid
    return pm, p, scale


def make_media_icon(kind: str, color: str = "", size: int = 18) -> QIcon:
    """Small flat transport icons drawn with QPainter, so they look the same on every Windows setup."""
    pm, p, scale = _icon_canvas(size)
    color = color or PAL["icon"]
    p.setPen(Qt.PenStyle.NoPen)
    p.setBrush(QColor(color))

    def tri(*pts):
        p.drawPolygon(QPolygonF([QPointF(x, y) for x, y in pts]))

    if kind == "play":
        tri((7, 4), (20, 12), (7, 20))
    elif kind == "pause":
        p.drawRoundedRect(QRectF(6, 4, 4.5, 16), 1.2, 1.2)
        p.drawRoundedRect(QRectF(13.5, 4, 4.5, 16), 1.2, 1.2)
    elif kind == "prev":
        p.drawRoundedRect(QRectF(5, 5, 2.6, 14), 1, 1)
        tri((19, 5), (8.5, 12), (19, 19))
    elif kind == "next":
        p.drawRoundedRect(QRectF(16.4, 5, 2.6, 14), 1, 1)
        tri((5, 5), (15.5, 12), (5, 19))
    elif kind in ("volume", "mute"):
        tri((3, 9.5), (6.8, 9.5), (11.5, 5.2), (11.5, 18.8), (6.8, 14.5), (3, 14.5))
    if kind in ("shuffle", "repeat", "repeat_one", "volume", "mute"):
        pen = QPen(QColor(color), 2.0)
        pen.setCapStyle(Qt.PenCapStyle.RoundCap)
        pen.setJoinStyle(Qt.PenJoinStyle.RoundJoin)
        p.setPen(pen)
        p.setBrush(Qt.BrushStyle.NoBrush)

        def line(*pts):
            path = QPainterPath(QPointF(*pts[0]))
            for pt in pts[1:]:
                path.lineTo(QPointF(*pt))
            p.drawPath(path)

        if kind == "shuffle":
            a = QPainterPath(QPointF(3.5, 7))
            a.lineTo(6.5, 7)
            a.cubicTo(11.5, 7, 11, 17, 16, 17)
            a.lineTo(19.5, 17)
            b = QPainterPath(QPointF(3.5, 17))
            b.lineTo(6.5, 17)
            b.cubicTo(11.5, 17, 11, 7, 16, 7)
            b.lineTo(19.5, 7)
            p.drawPath(a)
            p.drawPath(b)
            line((17, 4.5), (19.5, 7), (17, 9.5))
            line((17, 14.5), (19.5, 17), (17, 19.5))
        elif kind in ("repeat", "repeat_one"):
            top = QPainterPath(QPointF(4, 12.5))
            top.lineTo(4, 10)
            top.quadTo(4, 7, 7, 7)
            top.lineTo(18.5, 7)
            bottom = QPainterPath(QPointF(20, 11.5))
            bottom.lineTo(20, 14)
            bottom.quadTo(20, 17, 17, 17)
            bottom.lineTo(5.5, 17)
            p.drawPath(top)
            p.drawPath(bottom)
            line((16, 4.5), (18.5, 7), (16, 9.5))
            line((8, 14.5), (5.5, 17), (8, 19.5))
            if kind == "repeat_one":
                font = QFont("Segoe UI")
                font.setPixelSize(8)
                font.setBold(True)
                p.setFont(font)
                p.drawText(QRectF(7, 7.6, 10, 9), Qt.AlignmentFlag.AlignCenter, "1")
        elif kind == "volume":
            p.drawArc(QRectF(10, 8.5, 6, 7), -55 * 16, 110 * 16)
            p.drawArc(QRectF(8.5, 5, 12, 14), -55 * 16, 110 * 16)
        elif kind == "mute":
            line((15, 9.5), (20, 14.5))
            line((20, 9.5), (15, 14.5))
    p.end()
    pm.setDevicePixelRatio(scale)
    return QIcon(pm)


def make_swatch_icon(hex_color: str, selected: bool, size: int = 16) -> QIcon:
    """Accent color dot for the menu; the selected one gets a ring around it."""
    pm, p, scale = _icon_canvas(size)
    p.setPen(Qt.PenStyle.NoPen)
    p.setBrush(QColor(hex_color))
    if selected:
        p.drawEllipse(QRectF(6.5, 6.5, 11, 11))
        ring = QPen(QColor(PAL["text_soft"]), 2.0)
        p.setPen(ring)
        p.setBrush(Qt.BrushStyle.NoBrush)
        p.drawEllipse(QRectF(2.5, 2.5, 19, 19))
    else:
        p.drawEllipse(QRectF(4, 4, 16, 16))
    p.end()
    pm.setDevicePixelRatio(scale)
    return QIcon(pm)


SHORTCUTS_HTML = """
<table cellspacing="6">
<tr><td><b>Space</b></td><td>Play / pause</td></tr>
<tr><td><b>Ctrl+← / Ctrl+→</b></td><td>Previous / next track</td></tr>
<tr><td><b>F11</b> or double-click</td><td>Fullscreen (Esc to leave)</td></tr>
<tr><td><b>[ / ]</b></td><td>Show lyrics 0.1 s earlier / later</td></tr>
<tr><td><b>\\</b></td><td>Reset lyrics offset</td></tr>
<tr><td><b>R</b></td><td>Romaji on / off</td></tr>
<tr><td><b>S</b></td><td>Shuffle on / off</td></tr>
<tr><td><b>L</b></td><td>Repeat: off → track → album</td></tr>
<tr><td><b>M</b></td><td>Mute / unmute</td></tr>
<tr><td><b>V</b></td><td>Visualizer: off → bars → wave</td></tr>
<tr><td><b>Ctrl+O</b></td><td>Open audio file</td></tr>
<tr><td><b>Ctrl+Shift+O</b></td><td>Open album folder</td></tr>
<tr><td><b>Ctrl+L</b></td><td>Load LRC file</td></tr>
<tr><td><b>Ctrl+T</b></td><td>Sync lyrics (tap-to-time editor)</td></tr>
<tr><td><b>Ctrl+Shift+T</b></td><td>Add translation</td></tr>
<tr><td><b>Ctrl+I</b></td><td>Track details</td></tr>
<tr><td><b>F1</b></td><td>This list</td></tr>
</table>
<p>Drop songs, album folders or an .lrc file onto the window to open them.</p>
"""


class MetadataDialog(QDialog):
    def __init__(self, parent, info, cover_pixmap):
        super().__init__(parent)
        self.setWindowTitle("Track Details")
        self.resize(500, 680)
        self.setStyleSheet(f"""
            QDialog {{
                background-color: {PAL['dialog_bg']};
                color: {PAL['text']};
                font-family: 'Segoe UI', -apple-system, sans-serif;
            }}
            QLabel {{ font-size: 13px; background: transparent; }}
        """)

        layout = QVBoxLayout(self)
        layout.setContentsMargins(24, 24, 24, 24)
        layout.setSpacing(16)

        lbl_head = QLabel("Track Details")
        lbl_head.setStyleSheet(f"font-size: 18px; font-weight: bold; color: {PAL['title']};")
        layout.addWidget(lbl_head)

        if cover_pixmap and not cover_pixmap.isNull():
            lbl_cover_prev = QLabel()
            lbl_cover_prev.setFixedSize(140, 140)
            lbl_cover_prev.setStyleSheet(f"border-radius: 10px; border: 1px solid {PAL['surface_border']};")
            scaled = cover_pixmap.scaled(140, 140, Qt.AspectRatioMode.KeepAspectRatioByExpanding, Qt.TransformationMode.SmoothTransformation)
            lbl_cover_prev.setPixmap(scaled)
            lbl_cover_prev.setAlignment(Qt.AlignmentFlag.AlignCenter)
            layout.addWidget(lbl_cover_prev, alignment=Qt.AlignmentFlag.AlignCenter)

        scroll = QScrollArea()
        scroll.setWidgetResizable(True)
        scroll.setStyleSheet("background: transparent; border: none;")
        scroll_content = QWidget()
        grid = QGridLayout(scroll_content)
        grid.setSpacing(12)

        fields = [
            ("Title", info.get("title", "-")),
            ("Artist", info.get("artist", "-")),
            ("Album", info.get("album", "-")),
            ("Album Artist", info.get("album_artist", "-")),
            ("Track Number", info.get("track_number", "-")),
            ("Disc Number", info.get("disc_number", "-")),
            ("Bitrate", f"{info.get('bitrate_kbps', 0)} kbps"),
            ("Sample Rate", f"{info.get('sample_rate_khz', 0.0)} kHz"),
            ("Bit Depth", f"{info.get('bits_per_sample', 16)} bit"),
            ("Channels", f"{info.get('channels', 2)} ch"),
            ("Genre", info.get("genre", "-")),
            ("Year", info.get("year", "-")),
            ("Duration", info.get("duration_str", "-")),
            ("MIME-type", info.get("mime_type", "-")),
            ("File Path", info.get("file_path", "-")),
        ]

        for row, (label, val) in enumerate(fields):
            lbl_name = QLabel(label)
            lbl_name.setStyleSheet(f"color: {PAL['dim']}; font-weight: 600;")
            lbl_val = QLabel(str(val))
            lbl_val.setStyleSheet(f"color: {PAL['text_soft']};")
            lbl_val.setWordWrap(True)

            grid.addWidget(lbl_name, row, 0, Qt.AlignmentFlag.AlignTop)
            grid.addWidget(lbl_val, row, 1, Qt.AlignmentFlag.AlignTop)

        grid.setColumnStretch(1, 1)
        scroll.setWidget(scroll_content)
        layout.addWidget(scroll, 1)

        btn_close = QPushButton("Close")
        btn_close.clicked.connect(self.accept)
        btn_close.setStyleSheet("QPushButton { padding: 8px 16px; font-weight: bold; }")
        layout.addWidget(btn_close, alignment=Qt.AlignmentFlag.AlignRight)


class LyricCard(QFrame):
    def __init__(self, time_ms: int, main_text: str, sub_text: str = "", player_ref=None):
        super().__init__()
        self.time_ms = time_ms
        # Original LRC text; what is shown can differ (romaji mode), see set_display()
        self.main_text = main_text
        self.sub_text = sub_text
        self.player_ref = player_ref
        self.is_active = False
        self._padding_y = 6

        self.setSizePolicy(QSizePolicy.Policy.Expanding, QSizePolicy.Policy.Minimum)
        self.setCursor(Qt.CursorShape.PointingHandCursor)

        self.opacity_effect = QGraphicsOpacityEffect(self)
        self.setGraphicsEffect(self.opacity_effect)
        self.opacity_anim = QPropertyAnimation(self.opacity_effect, b"opacity")
        self.opacity_anim.setDuration(300)
        self.opacity_anim.setEasingCurve(QEasingCurve.Type.OutCubic)

        self.pad_anim = QPropertyAnimation(self, b"padding_y")
        self.pad_anim.setDuration(300)
        self.pad_anim.setEasingCurve(QEasingCurve.Type.OutCubic)

        self.layout = QVBoxLayout(self)
        self.layout.setContentsMargins(16, self._padding_y, 16, self._padding_y)
        self.layout.setSpacing(6)

        self.lbl_main = QLabel(self.main_text)
        self.lbl_main.setWordWrap(True)
        self.lbl_main.setSizePolicy(QSizePolicy.Policy.Expanding, QSizePolicy.Policy.Minimum)
        self.layout.addWidget(self.lbl_main)

        self.lbl_sub = QLabel(self.sub_text)
        self.lbl_sub.setWordWrap(True)
        self.lbl_sub.setSizePolicy(QSizePolicy.Policy.Expanding, QSizePolicy.Policy.Minimum)
        if not self.sub_text:
            self.lbl_sub.hide()
        self.layout.addWidget(self.lbl_sub)

        self.update_style(is_active=False, is_fullscreen=False, is_synced=True, animate=False)

    def set_display(self, main: str, sub: str):
        """Text actually shown: the original lines, or romaji + translation in romaji mode."""
        self.lbl_main.setText(main)
        self.lbl_sub.setText(sub)
        self.lbl_sub.setVisible(bool(sub))

    @pyqtProperty(int)
    def padding_y(self):
        return self._padding_y

    @padding_y.setter
    def padding_y(self, val):
        self._padding_y = val
        self.layout.setContentsMargins(16, val, 16, val)

    def mousePressEvent(self, event):
        if event.button() == Qt.MouseButton.LeftButton and self.player_ref and self.player_ref.is_synced:
            self.player_ref.seek_to(max(0, self.time_ms + self.player_ref.lyrics_offset_ms))
        super().mousePressEvent(event)

    def update_style(self, is_active: bool, is_fullscreen: bool, is_synced: bool = True, animate: bool = True):
        self.is_active = is_active

        active_main_size = 26 if is_fullscreen else 22
        active_sub_size = 17 if is_fullscreen else 15
        inactive_main_size = 19 if is_fullscreen else 16
        inactive_sub_size = 14 if is_fullscreen else 12

        target_padding = (30 if is_fullscreen else 22) if is_active else (8 if is_fullscreen else 5)
        target_opacity = 1.0 if is_active else PAL["inactive_opacity"]

        if not is_synced:
            self.opacity_anim.stop()
            self.pad_anim.stop()
            self.padding_y = 6
            self.opacity_effect.setOpacity(1.0)
            self.lbl_main.setFont(QFont("Segoe UI", 16, QFont.Weight.Normal))
            self.lbl_main.setStyleSheet(f"color: {PAL['lyric_plain']};")
            self.lbl_sub.hide()
            self.setStyleSheet("LyricCard { background-color: transparent; }")
            return

        if is_active:
            self.lbl_main.setFont(QFont("Segoe UI", active_main_size, QFont.Weight.Bold))
            self.lbl_main.setStyleSheet(f"color: {PAL['lyric_active']};")
            self.lbl_sub.setFont(QFont("Segoe UI", active_sub_size, QFont.Weight.DemiBold))
            self.lbl_sub.setStyleSheet(f"color: {PAL['lyric_active_sub']};")
            self.layout.setSpacing(8)
            self.setStyleSheet("LyricCard { background-color: transparent; }")
        else:
            self.lbl_main.setFont(QFont("Segoe UI", inactive_main_size, QFont.Weight.Normal))
            self.lbl_main.setStyleSheet(f"color: {PAL['lyric_inactive']};")
            self.lbl_sub.setFont(QFont("Segoe UI", inactive_sub_size, QFont.Weight.Normal))
            self.lbl_sub.setStyleSheet(f"color: {PAL['lyric_inactive_sub']};")
            self.layout.setSpacing(4)
            self.setStyleSheet("LyricCard { background-color: transparent; }")

        if animate:
            self.opacity_anim.stop()
            self.opacity_anim.setStartValue(self.opacity_effect.opacity())
            self.opacity_anim.setEndValue(target_opacity)
            self.opacity_anim.start()

            self.pad_anim.stop()
            self.pad_anim.setStartValue(self._padding_y)
            self.pad_anim.setEndValue(target_padding)
            self.pad_anim.start()
        else:
            self.opacity_anim.stop()
            self.pad_anim.stop()
            self.opacity_effect.setOpacity(target_opacity)
            self.padding_y = target_padding


# ---------------------------------------------------------
# Visualizer (drawn behind the lyrics)
# ---------------------------------------------------------
def prime_qt_virtuals(signal):
    """Settle, on the GUI thread, that this object has no Python connectNotify/disconnectNotify.

    For QObjects created from Python, PyQt asks the interpreter whether those methods are
    overridden the first time Qt calls them, and caches the answer. Qt Multimedia's
    audio threads connect/disconnect from the player objects; if they are the first to
    ask, they need Python's lock while holding Qt's, which can freeze the app.
    """
    def _noop(*_args):
        pass
    signal.connect(_noop)
    signal.disconnect(_noop)


def _make_fft(n: int):
    """Iterative radix-2 FFT in plain Python: about 1 ms for 1024 points, so numpy isn't needed."""
    levels = n.bit_length() - 1
    rev = [int(format(i, f"0{levels}b")[::-1], 2) for i in range(n)]
    twiddles = [cmath.exp(-2j * math.pi * k / n) for k in range(n // 2)]

    def fft(x):
        a = [complex(x[r]) for r in rev]
        size = 2
        while size <= n:
            half = size // 2
            step = n // size
            for start in range(0, n, size):
                k = 0
                for j in range(start, start + half):
                    t = twiddles[k] * a[j + half]
                    u = a[j]
                    a[j] = u + t
                    a[j + half] = u - t
                    k += step
            size *= 2
        return a

    return fft


class VisualizerBackdrop(QWidget):
    """A subtle spectrum painted behind the lyrics (the lyrics view sits on top of it).

    Decoded audio arrives through QAudioBufferOutput. Every buffer carries its position
    in the song, so the picture follows what is heard, not what the decoder has
    already read ahead. Nothing is captured or computed while it is switched off.
    """
    MODES = ("off", "bars", "wave")
    FFT_SIZE = 1024
    RATE = 22050
    BANDS = 44
    F_LO, F_HI = 55.0, 9500.0
    HEIGHT_SHARE = 0.36   # bottom part of the lyrics area that the spectrum may use

    def __init__(self, player, parent=None):
        super().__init__(parent)
        self.player = player
        self.mode = "off"
        self.available = QAudioBufferOutput is not None and hasattr(player, "setAudioBufferOutput")
        self._output = None
        self._chunks = deque()   # (start_ms, sample_rate, array of mono floats)
        self._levels = [0.0] * self.BANDS
        self._ref_db = -30.0
        self._fft = _make_fft(self.FFT_SIZE)
        n = self.FFT_SIZE
        self._window = [0.5 - 0.5 * math.cos(2 * math.pi * i / (n - 1)) for i in range(n)]
        self._bands = self._band_bins()
        # Music has less energy up high; lift the treble so the bars look even
        self._tilt = [10.0 * i / (self.BANDS - 1) for i in range(self.BANDS)]
        self._timer = QTimer(self)
        self._timer.setInterval(33)
        self._timer.timeout.connect(self._tick)
        player.sourceChanged.connect(lambda _url: self._chunks.clear())
        player.playbackStateChanged.connect(lambda _state: self._wake())

    # ---------------------------------------------------------------- control
    def set_mode(self, mode: str):
        if mode not in self.MODES or (mode != "off" and not self.available):
            mode = "off"
        self.mode = mode
        if mode == "off":
            self._detach()
            self._timer.stop()
            self._chunks.clear()
            self._levels = [0.0] * self.BANDS
        else:
            self._attach()
            self._timer.start()
        self.update()

    def _wake(self):
        if self.mode != "off" and not self._timer.isActive():
            self._timer.start()

    def _attach(self):
        if self._output is not None:
            return
        fmt = QAudioFormat()
        fmt.setSampleRate(self.RATE)       # mono floats at 22 kHz: cheap, and plenty for a picture
        fmt.setChannelCount(1)
        fmt.setSampleFormat(QAudioFormat.SampleFormat.Float)
        self._output = QAudioBufferOutput(fmt, self)
        prime_qt_virtuals(self._output.audioBufferReceived)
        self._output.audioBufferReceived.connect(self._on_buffer)
        self.player.setAudioBufferOutput(self._output)

    def _detach(self):
        if self._output is None:
            return
        self.player.setAudioBufferOutput(None)
        self._output.deleteLater()
        self._output = None

    # ---------------------------------------------------------------- audio in
    def _on_buffer(self, buf):
        try:
            size = buf.byteCount()
            if size <= 0:
                return
            fmt = buf.format()
            raw = buf.constData().asstring(size)
            SF = QAudioFormat.SampleFormat
            kind = fmt.sampleFormat()
            channels = max(1, fmt.channelCount())
            if kind == SF.Float:
                data = array.array("f")
                data.frombytes(raw)
                if channels > 1:
                    data = data[::channels]
            else:  # only if Qt ignored the requested format
                code, scale, offset = {SF.Int16: ("h", 1 / 32768, 0), SF.Int32: ("i", 1 / 2147483648, 0),
                                       SF.UInt8: ("B", 1 / 128, 128)}.get(kind, (None, 0, 0))
                if code is None:
                    return
                ints = array.array(code)
                ints.frombytes(raw)
                data = array.array("f", ((v - offset) * scale for v in ints[::channels]))
            start_ms = buf.startTime() / 1000.0
            chunks = self._chunks
            if chunks and start_ms < chunks[-1][0] - 500:
                chunks.clear()  # jumped backwards (seek / new song)
            chunks.append((start_ms, fmt.sampleRate() or self.RATE, data))
            while len(chunks) > 2 and chunks[-1][0] - chunks[0][0] > 3000:
                chunks.popleft()
        except Exception as e:
            print(f"Visualizer buffer error: {e}")

    def _window_at(self, pos_ms: float):
        """FFT_SIZE samples ending at pos_ms, or None if that part of the song isn't buffered."""
        chunks = self._chunks
        idx = None
        for i in range(len(chunks) - 1, -1, -1):
            if chunks[i][0] <= pos_ms:
                idx = i
                break
        if idx is None:
            return None
        start, rate, data = chunks[idx]
        end = int((pos_ms - start) * rate / 1000)
        if end > len(data) + rate // 4:
            return None  # nothing decoded for this moment (just seeked, or the song ended)
        end = min(end, len(data))
        need = self.FFT_SIZE
        out = data[max(0, end - need):end]
        j = idx - 1
        while len(out) < need and j >= 0:
            p_start, p_rate, p_data = chunks[j]
            if abs(p_start + len(p_data) * 1000.0 / p_rate - start) > 60:
                break  # not continuous with the newer chunk
            out = p_data[-(need - len(out)):] + out
            start = p_start
            j -= 1
        if len(out) < need:
            out = array.array("f", bytes(4 * (need - len(out)))) + out
        return out

    # ---------------------------------------------------------------- analysis
    def _band_bins(self):
        bin_hz = self.RATE / self.FFT_SIZE
        ratio = self.F_HI / self.F_LO
        edges = [self.F_LO * ratio ** (i / self.BANDS) for i in range(self.BANDS + 1)]
        half = self.FFT_SIZE // 2
        bands = []
        for i in range(self.BANDS):
            lo = max(1, int(edges[i] / bin_hz))
            hi = min(half, max(lo + 1, int(math.ceil(edges[i + 1] / bin_hz))))
            bands.append((lo, hi))
        return bands

    def _analyze(self, samples):
        n = self.FFT_SIZE
        w = self._window
        spectrum = self._fft([samples[i] * w[i] for i in range(n)])
        norm = 4.0 / n  # a full-scale sine wave reads as 1.0
        power = [(abs(c) * norm) ** 2 for c in spectrum[: n // 2]]
        dbs = [10.0 * math.log10(max(power[lo:hi]) + 1e-12) + self._tilt[i]
               for i, (lo, hi) in enumerate(self._bands)]
        # Auto-gain: follow loud passages quickly, relax slowly, never amplify silence
        self._ref_db = max(max(dbs), self._ref_db - 0.25, -34.0)
        span = 46.0
        floor = self._ref_db - span
        return [min(1.0, max(0.0, (db - floor) / span)) ** 1.7 for db in dbs]

    def _tick(self):
        playing = self.player.playbackState() == QMediaPlayer.PlaybackState.PlayingState
        target = None
        if playing:
            window = self._window_at(self.player.position())
            if window is not None:
                target = self._analyze(window)
        if target is None:
            target = [0.0] * self.BANDS
        levels = self._levels
        for i, t in enumerate(target):
            v = levels[i]
            levels[i] = v + (t - v) * (0.55 if t > v else 0.14)  # quick rise, gentle fall
        if not playing and max(levels) < 0.002:
            self._levels = [0.0] * self.BANDS
            self._timer.stop()  # paused or stopped: nothing to animate
        self._update_band_area()

    def _update_band_area(self):
        area = int(self.height() * self.HEIGHT_SHARE) + 12
        self.update(0, self.height() - area, self.width(), area)

    # ---------------------------------------------------------------- drawing
    def paintEvent(self, event):
        if self.mode == "off":
            return
        lv = self._levels
        if max(lv) < 0.002:
            return
        n = len(lv)
        # gentle smoothing across neighbours looks calmer than raw bins
        disp = [(lv[max(0, i - 1)] + 2 * lv[i] + lv[min(n - 1, i + 1)]) / 4 for i in range(n)]
        W, H = self.width(), self.height()
        area_h = H * self.HEIGHT_SHARE
        base_y = H - 4.0

        accent = QColor(PAL["accent"])
        alpha = PAL["viz_alpha"]
        grad = QLinearGradient(0, base_y - area_h, 0, base_y)
        for stop, a in ((0.0, 0.0), (0.55, alpha * 0.45), (1.0, alpha)):
            c = QColor(accent)
            c.setAlphaF(a)
            grad.setColorAt(stop, c)

        p = QPainter(self)
        p.setRenderHint(QPainter.RenderHint.Antialiasing)
        p.setPen(Qt.PenStyle.NoPen)
        p.setBrush(grad)
        if self.mode == "bars":
            slot = W / n
            bw = slot * 0.6
            radius = min(bw / 2, 4.0)
            for i, v in enumerate(disp):
                h = max(3.0, v * area_h)
                p.drawRoundedRect(QRectF(i * slot + (slot - bw) / 2, base_y - h, bw, h), radius, radius)
        else:
            pts = [QPointF(i * W / (n - 1), base_y - v * area_h) for i, v in enumerate(disp)]

            def curve(path):
                for i in range(n - 1):  # Catmull-Rom through the points
                    p0, p1, p2 = pts[max(0, i - 1)], pts[i], pts[i + 1]
                    p3 = pts[min(n - 1, i + 2)]
                    path.cubicTo(p1 + (p2 - p0) / 6.0, p2 - (p3 - p1) / 6.0, p2)

            fill = QPainterPath(QPointF(0, base_y))
            fill.lineTo(pts[0])
            curve(fill)
            fill.lineTo(W, base_y)
            fill.closeSubpath()
            p.drawPath(fill)

            crest = QPainterPath(pts[0])
            curve(crest)
            line = QColor(accent)
            line.setAlphaF(min(1.0, alpha * 1.35))
            p.setPen(QPen(line, 2.0))
            p.setBrush(Qt.BrushStyle.NoBrush)
            p.drawPath(crest)
        p.end()


# ---------------------------------------------------------
# Translation alignment (matching a translation to synced lyrics)
# ---------------------------------------------------------
_HEADER_WORDS = (r"(?:verse|chorus|pre-?chorus|post-?chorus|bridge|intro|outro|hook|refrain|"
                 r"interlude|instrumental|break|repeat|coda|spoken|rap)")
_HDR_SQUARE = re.compile(r"^\s*[\[【].*[\]】]\s*$")
_HDR_PAREN = re.compile(
    rf"^\s*[(（][^)）]*\b{_HEADER_WORDS}\b[^)）]*[)）]\s*$|^\s*[(（]\s*[x×]\s*\d+\s*[)）]\s*$", re.I)
_HDR_BARE = re.compile(rf"^\s*(?:repeat\s+)?{_HEADER_WORDS}(?:\s*\d+)?(?:\s*[x×]\s*\d+)?\s*:?\s*$", re.I)


def is_section_header(line: str) -> bool:
    """[Chorus], (Verse 2), Chorus x2 … : labels in a translation, not sung lines.

    Plain parentheses are left alone, since "(come on)" is usually a backing vocal.
    """
    return bool(_HDR_SQUARE.match(line) or _HDR_PAREN.match(line) or _HDR_BARE.match(line))


def clean_translation_lines(text: str):
    """Translation text -> non-empty lines, without LRC timestamps or tags (section labels are kept)."""
    out = []
    for raw in text.splitlines():
        line = raw.strip()
        if not line or _LRC_META.match(line):
            continue
        m = _LRC_LEADING.match(line)
        if m:
            line = m.group(2).strip()
        if line and line != "♪":
            out.append(line)
    return out


_CLAUSE_BREAK = re.compile(r"(?<=[,;:，、；：])\s+")
_TRAILING_PUNCT = ",;:，、；："


def split_clauses(line: str):
    """'I'm provoked by the sun, my body heats' -> ["I'm provoked by the sun,", 'my body heats'].

    Long translation lines are shown to the model in parts, so when one translated line
    covers two sung lines, each sung line can get its own part (without any text being written).
    """
    if is_section_header(line):
        return [line]
    parts = [p.strip() for p in _CLAUSE_BREAK.split(line.strip()) if p.strip()]
    return parts if 2 <= len(parts) <= 8 else [line]


def line_is_used(line: str, texts) -> bool:
    """True if every part of a translation line appears in some row (it may be split over rows)."""
    return all(any(part.rstrip(_TRAILING_PUNCT) in t for t in texts) for part in split_clauses(line))


def match_in_order(n_rows: int, lines):
    """The simple method: n-th sung line gets the n-th translation line (labels skipped)."""
    content = [l for l in lines if not is_section_header(l)]
    return [content[i] if i < len(content) else "" for i in range(n_rows)]


def build_alignment_messages(originals, translations):
    """Chat messages asking the model to match line numbers, never to write text itself."""
    o_block = "\n".join(f"O{i}: {t}" for i, t in enumerate(originals, 1))
    t_rows = []
    for i, t in enumerate(translations, 1):
        parts = split_clauses(t)
        if len(parts) == 1:
            t_rows.append(f"T{i}: {t}")
        else:
            t_rows.extend(f"T{i}.{k}: {p}" for k, p in enumerate(parts, 1))
    t_block = "\n".join(t_rows)
    n = len(originals)
    system = ("You align song lyrics with a translation of them. You never translate, rewrite "
              "or comment; you only match line numbers.")
    user = f"""Below are the lines of a song as they are sung (O1, O2, …) and a translation of the same song (T1, T2, …). The translation may split or join lines differently, leave out repeated parts, add section labels like [Chorus], or say "(repeat)".

For every sung line, give the translation line(s) that say the same thing.

Rules:
- Answer with exactly one line per sung line, from O1 to O{n}, in order, and nothing else.
- Format: O<number>: T<number>   or   O<number>: T<number> T<number>   or   O<number>: none
- Use several T numbers, in reading order, when one sung line is translated across several translation lines.
- Longer translation lines are shown in parts (T5.1, T5.2 …). When the whole line belongs to one sung line, just write T5. Only when one translation line covers two or more sung lines, give each sung line the part(s) that translate it.
- The same T line may be used for several sung lines, for example a chorus that is sung twice but translated once.
- Use "none" when nothing fits: vocalisations, ad-libs, or lines the translation leaves out.
- Never give section labels or notes such as [Chorus] or (x2) as a translation.

Example
Sung:
O1: 君の声が聞こえる
O2: 遠い空の向こうから
O3: ラララ
O4: 夜が明けて
O5: 光が差す
O6: 君の声が聞こえる
Translation:
T1: [Verse]
T2: I can hear your voice
T3: from beyond
T4: the distant sky
T5.1: The night is ending,
T5.2: and the light shines in
Answer:
O1: T2
O2: T3 T4
O3: none
O4: T5.1
O5: T5.2
O6: T2

Now the real song.
Sung:
{o_block}
Translation:
{t_block}
Answer:"""
    return [{"role": "system", "content": system}, {"role": "user", "content": user}]


_THINK_BLOCK = re.compile(r"<think>.*?</think>", re.S | re.I)
_ANSWER_LINE = re.compile(r"^[\s*>`•\-]*O\s*(\d+)\s*\**\s*(?:[:=→>]|-+>?|–)\s*(.*)$", re.I | re.M)
_T_REF = re.compile(r"T\s*(\d+)(?:\.(\d+))?(?:\s*(?:-|–|—|to)\s*T?\s*(\d+)(?:\.(\d+))?)?", re.I)
_NONE_WORDS = re.compile(r"^\W*(?:none|nothing|n/?a|no match|skip|empty|—|-|–|∅)\W*$", re.I)


def strip_thinking(text: str) -> str:
    """Removes <think>…</think> reasoning that some models write before their answer."""
    text = _THINK_BLOCK.sub("", text)
    low = text.lower()
    if "</think>" in low:  # the opening tag was part of the prompt template
        text = text[low.rindex("</think>") + len("</think>"):]
        low = text.lower()
    if "<think>" in low:  # cut off while still thinking
        text = text[:low.index("<think>")]
    return text


def count_answer_lines(text: str) -> int:
    return len(_ANSWER_LINE.findall(strip_thinking(text)))


def parse_alignment(text: str, n_rows: int, translations) -> dict:
    """The model's answer -> {row: [(line, part)]}, 1-based; part 0 means the whole line.

    Invalid numbers are dropped; a part number a line doesn't have counts as the whole line.
    """
    parts_of = [split_clauses(t) for t in translations]
    n_trans = len(translations)

    def ref(line, part):
        if not 1 <= line <= n_trans:
            return None
        return (line, part if part and 1 <= part <= len(parts_of[line - 1]) and len(parts_of[line - 1]) > 1 else 0)

    result = {}
    for m in _ANSWER_LINE.finditer(strip_thinking(text)):
        row = int(m.group(1))
        if not 1 <= row <= n_rows:
            continue
        rest = re.split(r"[(\"“«#]|//", m.group(2), maxsplit=1)[0].strip().strip("*` ")
        refs = []

        def add(r):
            if r is not None and r not in refs:
                refs.append(r)

        if not _NONE_WORDS.match(rest):
            for tm in _T_REF.finditer(rest):
                a_line, a_part = int(tm.group(1)), int(tm.group(2) or 0)
                if tm.group(3) is None:
                    add(ref(a_line, a_part))
                    continue
                b_line, b_part = int(tm.group(3)), int(tm.group(4) or 0)
                if b_line == a_line and a_part and b_part and b_part >= a_part:   # T5.1-T5.3
                    for k in range(a_part, b_part + 1):
                        add(ref(a_line, k))
                elif (a_part and tm.group(4) is None and 1 <= a_line <= n_trans
                      and a_part < b_line <= len(parts_of[a_line - 1])):            # T5.1-3
                    for k in range(a_part, b_line + 1):
                        add(ref(a_line, k))
                elif a_line < b_line <= a_line + 6:                               # T2-T4
                    add(ref(a_line, a_part))
                    for line in range(a_line + 1, b_line):
                        add(ref(line, 0))
                    add(ref(b_line, b_part))
                else:
                    add(ref(a_line, a_part))
        result[row] = refs
    return result


def alignment_to_texts(mapping: dict, n_rows: int, translations):
    """Mapping -> translation text per row ("" = no translation, None = the model skipped the row).

    Parts of a line the model left unused join the row that holds a neighbouring part of the
    same line, so a translation line is never cut short by accident.
    """
    parts_of = [split_clauses(t) for t in translations]
    rows = {row: list(refs) for row, refs in mapping.items()}

    used = set()
    for refs in rows.values():
        for line, part in refs:
            used.update((line, k) for k in range(1, len(parts_of[line - 1]) + 1)) if part == 0 else used.add((line, part))
    for line, parts in enumerate(parts_of, 1):
        if len(parts) < 2 or not any((line, k) in used for k in range(1, len(parts) + 1)):
            continue
        for k in range(1, len(parts) + 1):
            if (line, k) in used:
                continue
            placed = False
            for row in sorted(rows):
                refs = rows[row]
                if (line, k - 1) in refs:
                    refs.insert(refs.index((line, k - 1)) + 1, (line, k))
                    placed = True
                elif (line, k + 1) in refs:
                    refs.insert(refs.index((line, k + 1)), (line, k))
                    placed = True
                if placed:
                    used.add((line, k))
                    break

    out = []
    for row in range(1, n_rows + 1):
        if row not in rows:
            out.append(None)
            continue
        pieces, last = [], None
        for line, part in rows[row]:
            if is_section_header(translations[line - 1]):
                continue
            pieces.append(translations[line - 1] if part == 0 else parts_of[line - 1][part - 1])
            last = (line, part)
        text = " ".join(pieces)
        if last and last[1] and last[1] < len(parts_of[last[0] - 1]):
            text = text.rstrip(_TRAILING_PUNCT)  # ends mid-line: drop the dangling comma
        out.append(text)
    return out


# ---------------------------------------------------------
# AI server: a local one (LM Studio, Ollama, llama.cpp, KoboldCpp, vLLM …) or OpenRouter, both OpenAI-compatible
# ---------------------------------------------------------
OPENROUTER_BASE = "https://openrouter.ai/api/v1"
OPENROUTER_KEYS_URL = "https://openrouter.ai/keys"
APP_TITLE = "LRC Open Player"


class AiError(Exception):
    pass


def _dpapi(data: bytes, protect: bool) -> bytes:
    """Windows' per-user encryption (the same one browsers use for saved passwords)."""
    from ctypes import wintypes

    class DATA_BLOB(ctypes.Structure):
        _fields_ = [("cbData", wintypes.DWORD), ("pbData", ctypes.POINTER(ctypes.c_char))]

    buf = ctypes.create_string_buffer(data, len(data))
    blob_in = DATA_BLOB(len(data), ctypes.cast(buf, ctypes.POINTER(ctypes.c_char)))
    blob_out = DATA_BLOB()
    crypt32 = ctypes.windll.crypt32
    fn = crypt32.CryptProtectData if protect else crypt32.CryptUnprotectData
    fn.argtypes = [ctypes.POINTER(DATA_BLOB), ctypes.c_void_p, ctypes.c_void_p, ctypes.c_void_p,
                   ctypes.c_void_p, wintypes.DWORD, ctypes.POINTER(DATA_BLOB)]
    fn.restype = wintypes.BOOL
    if not fn(ctypes.byref(blob_in), None, None, None, None, 0x1, ctypes.byref(blob_out)):  # UI_FORBIDDEN
        raise ctypes.WinError()
    try:
        return ctypes.string_at(blob_out.pbData, blob_out.cbData)
    finally:
        kernel32 = ctypes.windll.kernel32
        kernel32.LocalFree.argtypes = [ctypes.c_void_p]
        kernel32.LocalFree(ctypes.cast(blob_out.pbData, ctypes.c_void_p))


def protect_secret(text: str) -> str:
    """API key -> what is stored in the settings: encrypted for this Windows account when possible."""
    if not text:
        return ""
    if sys.platform == "win32":
        try:
            stored = "dpapi:" + base64.b64encode(_dpapi(text.encode("utf-8"), True)).decode("ascii")
            if unprotect_secret(stored) == text:  # only trust it once it reads back
                return stored
        except Exception:
            pass
    return text


def unprotect_secret(stored: str) -> str:
    if not stored:
        return ""
    if stored.startswith("dpapi:"):
        try:
            return _dpapi(base64.b64decode(stored[6:]), False).decode("utf-8")
        except Exception:
            return ""
    return stored  # stored before encryption existed, or encryption unavailable


def secrets_are_encrypted() -> bool:
    return sys.platform == "win32" and protect_secret("check").startswith("dpapi:")


class _MaxTokensRejected(Exception):
    pass


def normalize_ai_url(url: str) -> str:
    """'localhost:1234', '…/v1/chat/completions', Ollama's '…:11434' -> 'http://host:port/v1'."""
    u = (url or "").strip().rstrip("/")
    if not u:
        return ""
    if "://" not in u:
        u = "http://" + u
    for suffix in ("/chat/completions", "/completions", "/models"):
        if u.lower().endswith(suffix):
            u = u[: -len(suffix)].rstrip("/")
    parts = urllib.parse.urlsplit(u)
    path = parts.path.rstrip("/")
    if path in ("", "/api"):  # bare server, or Ollama's own API path
        path = "/v1"
    return urllib.parse.urlunsplit((parts.scheme, parts.netloc, path, "", ""))


def _ai_open(base_url: str, connect_timeout: float, read_timeout: float):
    parts = urllib.parse.urlsplit(base_url)
    if parts.scheme not in ("http", "https") or not parts.hostname:
        raise AiError(f"“{base_url}” isn't a valid server address.")
    cls = http.client.HTTPSConnection if parts.scheme == "https" else http.client.HTTPConnection
    conn = cls(parts.hostname, parts.port, timeout=connect_timeout)
    try:
        conn.connect()
    except Exception as e:
        conn.close()
        raise AiError(_ai_connect_error(e, parts.netloc)) from e
    conn.sock.settimeout(read_timeout)
    return conn, parts.path.rstrip("/")


def _ai_connect_error(e: Exception, where: str) -> str:
    if isinstance(e, ConnectionRefusedError):
        return f"Nothing is answering at {where}. Is the model server running?"
    if isinstance(e, socket.gaierror):
        return f"Couldn't find the server “{where}”. Check the address."
    if isinstance(e, (socket.timeout, TimeoutError)):
        return f"The server at {where} didn't respond. Check the address and that the server is running."
    return f"Couldn't connect to {where}: {e}"


def _ai_headers(key: str, extra=None) -> dict:
    headers = {"Content-Type": "application/json", "Accept": "text/event-stream, application/json",
               "User-Agent": "LRCOpenPlayer (translation aligner)"}
    if key:
        headers["Authorization"] = f"Bearer {key}"
    if extra:
        headers.update(extra)
    return headers


def _ai_http_error(status: int, body: bytes, endpoint: str, model: str = "") -> str:
    detail = ""
    try:
        data = json.loads(body.decode("utf-8", "replace"))
        err = data.get("error", data) if isinstance(data, dict) else data
        detail = err.get("message", "") if isinstance(err, dict) else str(err)
    except Exception:
        detail = body.decode("utf-8", "replace").strip()[:200]
    detail = (detail or "").strip()
    if status in (401, 403):
        return f"The server refused the request (HTTP {status}). Check the API key."
    if status == 402:
        return "The account has run out of credits (HTTP 402)." + (f" {detail}" if detail else "")
    if status == 429:
        return ("Too many requests right now (HTTP 429). Free models are often busy; "
                "wait a minute or pick another model.")
    if model and "model" in detail.lower() and status in (400, 404):
        return f"The server doesn't have the model “{model}” loaded: {detail}"
    if status == 404:
        return (f"There's no {endpoint} at this address (HTTP 404). "
                "OpenAI-compatible servers usually end in /v1.")
    return f"The server answered with an error (HTTP {status})" + (f": {detail}" if detail else ".")


def list_ai_models(base_url: str, key: str = "", timeout: float = 6.0, headers=None):
    """Model ids the server offers (GET /models)."""
    return list_ai_models_detailed(base_url, key, timeout, headers)[0]


def list_ai_models_detailed(base_url: str, key: str = "", timeout: float = 6.0, headers=None):
    """(model ids, {id: (price per input token, price per output token)}) from GET /models."""
    conn, path = _ai_open(base_url, timeout, timeout)
    try:
        conn.request("GET", path + "/models", headers=_ai_headers(key, headers))
        resp = conn.getresponse()
        body = resp.read()
        if resp.status != 200:
            raise AiError(_ai_http_error(resp.status, body, "/models"))
        data = json.loads(body.decode("utf-8", "replace"))
        items = data.get("data", data.get("models", [])) if isinstance(data, dict) else data
        ids, pricing = [], {}
        for item in items or []:
            mid = (item.get("id") or item.get("name") or item.get("model")) if isinstance(item, dict) else str(item)
            if mid and mid not in ids:
                ids.append(mid)
            price = item.get("pricing") if isinstance(item, dict) else None
            if mid and isinstance(price, dict):
                try:
                    p_in, p_out = float(price.get("prompt") or 0), float(price.get("completion") or 0)
                    if p_in >= 0 and p_out >= 0:
                        pricing[mid] = (p_in, p_out)
                except (TypeError, ValueError):
                    pass
        return ids, pricing
    except AiError:
        raise
    except (socket.timeout, TimeoutError) as e:
        raise AiError("The server didn't answer in time.") from e
    except ValueError as e:
        raise AiError("The server's answer wasn't valid JSON. Is this an OpenAI-compatible API?") from e
    except OSError as e:
        raise AiError(f"Connection problem: {e}") from e
    finally:
        conn.close()


def check_openrouter_key(base_url: str, key: str, headers=None, timeout: float = 10.0) -> str:
    """Asks OpenRouter whether the key is valid. Returns a short note, or raises AiError."""
    for endpoint in ("/key", "/auth/key"):
        conn, path = _ai_open(base_url, timeout, timeout)
        try:
            conn.request("GET", path + endpoint, headers=_ai_headers(key, headers))
            resp = conn.getresponse()
            body = resp.read()
        except (socket.timeout, TimeoutError) as e:
            raise AiError("OpenRouter didn't answer in time.") from e
        except OSError as e:
            raise AiError(f"Connection problem: {e}") from e
        finally:
            conn.close()
        if resp.status == 404:
            continue
        if resp.status in (401, 403):
            raise AiError("OpenRouter didn't accept this API key. Check that you copied all of it.")
        if resp.status != 200:
            raise AiError(_ai_http_error(resp.status, body, endpoint))
        try:
            data = json.loads(body.decode("utf-8", "replace")).get("data") or {}
        except (ValueError, AttributeError):
            data = {}
        left = data.get("limit_remaining")
        if isinstance(left, (int, float)) and not isinstance(left, bool):
            return f"Key accepted (${left:.2f} left of its spending limit)."
        return "Key accepted."
    return ""  # can't be checked this way; the first alignment will show any problem


class AiCheckWorker(QThread):
    """'Test connection' off the GUI thread: checks the key (OpenRouter) and lists the models."""
    result = pyqtSignal(dict)

    def __init__(self, provider: str, base_url: str, key: str, headers=None):
        super().__init__()
        self.provider, self.base_url, self.key, self.headers = provider, base_url, key, headers or {}

    def run(self):
        info = {"provider": self.provider, "ids": [], "pricing": {}, "error": "", "key_note": ""}
        try:
            if self.provider == "openrouter":
                info["key_note"] = check_openrouter_key(self.base_url, self.key, self.headers)
            info["ids"], info["pricing"] = list_ai_models_detailed(
                self.base_url, self.key, 15.0 if self.provider == "openrouter" else 6.0, self.headers)
        except AiError as e:
            info["error"] = str(e)
        except Exception as e:
            info["error"] = f"Unexpected problem: {e}"
        self.result.emit(info)


class AiAlignWorker(QThread):
    """Sends the alignment request and streams the answer back."""
    progress = pyqtSignal(str, int)   # phase ("waiting" | "thinking" | "answering"), answer lines so far
    succeeded = pyqtSignal(str, str)  # the model's whole answer, finish reason
    failed = pyqtSignal(str, str)     # message for the user, answer received so far

    READ_TIMEOUT = 600.0  # a big model on a busy GPU can take minutes for the first token

    def __init__(self, base_url: str, model: str, key: str, messages, total_rows: int, headers=None):
        super().__init__()
        self.base_url, self.model, self.key = base_url, model, key
        self.headers = headers or {}
        self.messages = messages
        self.total_rows = total_rows
        self._cancelled = False
        self._conn = None
        self.raw = ""

    def cancel(self):
        """Stops waiting and drops the connection, which also stops most servers generating."""
        self._cancelled = True
        conn = self._conn
        if conn is not None and conn.sock is not None:
            try:
                conn.sock.shutdown(socket.SHUT_RDWR)
            except OSError:
                pass

    def run(self):
        try:
            model = self.model
            if not model:
                ids = list_ai_models(self.base_url, self.key, headers=self.headers)
                model = ids[0] if ids else "local-model"
            try:
                text, finish = self._chat(model, with_max_tokens=True)
            except _MaxTokensRejected:
                text, finish = self._chat(model, with_max_tokens=False)
            if not self._cancelled:
                self.succeeded.emit(text, finish)
        except AiError as e:
            if not self._cancelled:
                self.failed.emit(str(e), self.raw)
        except (socket.timeout, TimeoutError):
            if not self._cancelled:
                self.failed.emit("The model stopped answering (no reply for 10 minutes).", self.raw)
        except Exception as e:
            if not self._cancelled:
                self.failed.emit(f"Connection problem: {e}", self.raw)
        finally:
            conn, self._conn = self._conn, None
            if conn is not None:
                conn.close()

    def _chat(self, model: str, with_max_tokens: bool):
        body = {"model": model, "messages": self.messages, "temperature": 0.2, "stream": True}
        if with_max_tokens:
            body["max_tokens"] = 6000 + 32 * self.total_rows  # room for models that think first
        conn, path = _ai_open(self.base_url, 8.0, self.READ_TIMEOUT)
        self._conn = conn
        if self._cancelled:
            return "", "cancelled"
        self.progress.emit("waiting", 0)
        conn.request("POST", path + "/chat/completions", json.dumps(body).encode("utf-8"),
                     _ai_headers(self.key, self.headers))
        resp = conn.getresponse()
        if resp.status != 200:
            err = resp.read()
            low = err.decode("utf-8", "replace").lower()
            if with_max_tokens and resp.status in (400, 422) and ("max_tokens" in low or "context" in low):
                conn.close()
                raise _MaxTokensRejected()
            raise AiError(_ai_http_error(resp.status, err, "/chat/completions", model))

        if "text/event-stream" not in (resp.getheader("Content-Type") or "").lower():
            data = json.loads(resp.read().decode("utf-8", "replace"))  # server ignored "stream"
            choice = (data.get("choices") or [{}])[0]
            self.raw = (choice.get("message") or {}).get("content") or choice.get("text") or ""
            return self.raw, choice.get("finish_reason") or ""

        content, finish = [], ""
        thinking = False
        last_emit = 0.0
        while not self._cancelled:
            line = resp.readline()
            if not line:
                break
            line = line.decode("utf-8", "replace").strip()
            if not line.startswith("data:"):
                continue
            payload = line[5:].strip()
            if payload == "[DONE]":
                break
            try:
                obj = json.loads(payload)
            except ValueError:
                continue
            if isinstance(obj, dict) and obj.get("error"):
                err = obj["error"]
                raise AiError(f"The model reported an error: {err.get('message', err) if isinstance(err, dict) else err}")
            for choice in obj.get("choices") or []:
                delta = choice.get("delta") or choice.get("message") or {}
                piece = delta.get("content") or choice.get("text") or ""
                if delta.get("reasoning_content") or delta.get("reasoning"):
                    thinking = True
                if piece:
                    content.append(piece)
                finish = choice.get("finish_reason") or finish
            now = time.monotonic()
            if now - last_emit > 0.15:
                last_emit = now
                self.raw = "".join(content)
                answered = count_answer_lines(self.raw)
                in_think = "<think>" in self.raw.lower() and "</think>" not in self.raw.lower()
                phase = "answering" if answered else ("thinking" if thinking or in_think else "waiting")
                self.progress.emit(phase, answered)
        self.raw = "".join(content)
        return self.raw, ("cancelled" if self._cancelled else finish)


class AiSettingsDialog(QDialog):
    """View ▾ → AI alignment…: a model on this computer, or one on OpenRouter."""

    LOCAL_HINT = ("Works with any server that offers an OpenAI-compatible API, such as LM Studio (port 1234), "
                  "Ollama (11434), llama.cpp (8080) or KoboldCpp (5001). Lyrics are sent only to this address.")

    def __init__(self, player_window):
        super().__init__(player_window)
        self.pw = player_window
        self.worker = None
        self.local_models = []
        self.or_models = []
        self.or_pricing = {}
        self.setWindowTitle("AI alignment")
        self.setMinimumWidth(600)
        self.setStyleSheet(dialog_stylesheet(PAL))

        v = QVBoxLayout(self)
        v.setContentsMargins(22, 20, 22, 18)
        v.setSpacing(12)
        head = QLabel("AI alignment")
        head.setObjectName("dlgHeading")
        intro = QLabel(
            "Optional help for Lyrics ▾ → Add translation: a language model works out which translated "
            "line belongs to which sung line. It only matches lines; it never writes or changes the translation."
        )
        intro.setObjectName("syncMuted")
        intro.setWordWrap(True)
        self.chk = QCheckBox("Use AI to align translations")

        switch = QHBoxLayout()
        switch.setSpacing(6)
        self.btn_local = QPushButton("On this computer")
        self.btn_or = QPushButton("OpenRouter (online)")
        self.provider_group = QButtonGroup(self)
        for b in (self.btn_local, self.btn_or):
            b.setCheckable(True)
            b.setObjectName("segment")
            self.provider_group.addButton(b)
            switch.addWidget(b)
        switch.addStretch()

        # ----- model on this computer
        local = QWidget()
        g = QGridLayout(local)
        g.setContentsMargins(0, 0, 0, 0)
        g.setHorizontalSpacing(12)
        g.setVerticalSpacing(8)
        self.ed_url = QLineEdit()
        self.ed_url.setPlaceholderText("http://127.0.0.1:1234/v1")
        self.ed_model = QLineEdit()
        self.ed_model.setPlaceholderText("Filled in by Test connection")
        self.btn_pick = QPushButton("Choose  ▾")
        self.btn_pick.setEnabled(False)
        self.btn_pick.setToolTip("Run Test connection to list the server's models")
        self.ed_key = QLineEdit()
        self.ed_key.setEchoMode(QLineEdit.EchoMode.Password)
        self.ed_key.setPlaceholderText("Usually not needed for local servers")
        for row, (text, widget) in enumerate((("Server address", self.ed_url), ("Model", self.ed_model),
                                               ("API key", self.ed_key))):
            lbl = QLabel(text)
            lbl.setObjectName("syncMuted")
            g.addWidget(lbl, row, 0)
            g.addWidget(widget, row, 1, 1, 1 if widget is self.ed_model else 2)
        g.addWidget(self.btn_pick, 1, 2)
        g.setColumnStretch(1, 1)
        g.setColumnMinimumWidth(0, 100)

        # ----- OpenRouter
        online = QWidget()
        g = QGridLayout(online)
        g.setContentsMargins(0, 0, 0, 0)
        g.setHorizontalSpacing(12)
        g.setVerticalSpacing(8)
        self.ed_or_key = QLineEdit()
        self.ed_or_key.setEchoMode(QLineEdit.EchoMode.Password)
        self.ed_or_key.setPlaceholderText("sk-or-…")
        self.btn_get_key = QPushButton("Get a key ↗")
        self.btn_get_key.setObjectName("link")
        self.btn_get_key.setToolTip(OPENROUTER_KEYS_URL)
        self.ed_or_model = QLineEdit()
        self.ed_or_model.setPlaceholderText("Run Test connection, then type to search (e.g. qwen)")
        self.lbl_cost = QLabel("")
        self.lbl_cost.setObjectName("syncMuted")
        for row, (text, widget) in enumerate((("API key", self.ed_or_key), ("Model", self.ed_or_model))):
            lbl = QLabel(text)
            lbl.setObjectName("syncMuted")
            g.addWidget(lbl, row, 0)
            g.addWidget(widget, row, 1, 1, 1 if widget is self.ed_or_key else 2)
        g.addWidget(self.btn_get_key, 0, 2)
        g.addWidget(self.lbl_cost, 2, 1, 1, 2)
        g.setColumnStretch(1, 1)
        g.setColumnMinimumWidth(0, 100)

        self.pages = QStackedWidget()
        self.pages.addWidget(local)
        self.pages.addWidget(online)

        test_row = QHBoxLayout()
        self.btn_test = QPushButton("Test connection")
        self.lbl_test = QLabel("")
        self.lbl_test.setWordWrap(True)
        test_row.addWidget(self.btn_test)
        test_row.addWidget(self.lbl_test, 1)

        self.hint = QLabel("")
        self.hint.setObjectName("syncHelp")
        self.hint.setWordWrap(True)

        buttons = QHBoxLayout()
        self.btn_cancel = QPushButton("Cancel")
        self.btn_save = QPushButton("Save")
        self.btn_save.setObjectName("primary")
        buttons.addStretch()
        buttons.addWidget(self.btn_cancel)
        buttons.addWidget(self.btn_save)

        v.addWidget(head)
        v.addWidget(intro)
        v.addSpacing(2)
        v.addWidget(self.chk)
        v.addLayout(switch)
        v.addWidget(self.pages)
        v.addLayout(test_row)
        v.addWidget(self.hint)
        v.addSpacing(4)
        v.addLayout(buttons)

        for b in self.findChildren(QPushButton):
            b.setAutoDefault(False)
            b.setDefault(False)

        raw = player_window.ai_settings_raw()
        self.chk.setChecked(raw["enabled"])
        self.ed_url.setText(raw["url"])
        self.ed_model.setText(raw["model"])
        self.ed_key.setText(raw["key"])
        self.ed_or_key.setText(raw["or_key"])
        self.ed_or_model.setText(raw["or_model"])
        (self.btn_or if raw["provider"] == "openrouter" else self.btn_local).setChecked(True)

        self.chk.toggled.connect(self._on_toggled)
        self.provider_group.buttonClicked.connect(lambda _b: self._on_provider())
        self.btn_get_key.clicked.connect(lambda: QDesktopServices.openUrl(QUrl(OPENROUTER_KEYS_URL)))
        self.ed_or_model.textChanged.connect(lambda _t: self._update_cost())
        self.btn_test.clicked.connect(self.test_connection)
        self.btn_cancel.clicked.connect(self.reject)
        self.btn_save.clicked.connect(self.save)
        self._on_provider()
        self._on_toggled(raw["enabled"])

    def _provider(self) -> str:
        return "openrouter" if self.btn_or.isChecked() else "local"

    def _on_provider(self):
        online = self._provider() == "openrouter"
        self.pages.setCurrentIndex(1 if online else 0)
        if online:
            stored = " Your key is stored encrypted for your Windows account." if secrets_are_encrypted() else ""
            self.hint.setText("Lyrics and the translation are sent to OpenRouter and to the company that runs the "
                              "model you choose. OpenRouter charges per use; the estimate is for a typical song."
                              + stored)
        else:
            self.hint.setText(self.LOCAL_HINT)
        self._set_test_status("")
        self._update_cost()

    def _on_toggled(self, on: bool):
        for w in (self.btn_local, self.btn_or, self.pages, self.btn_test, self.hint):
            w.setEnabled(on)
        self.btn_pick.setEnabled(on and bool(self.local_models))

    def _set_test_status(self, text: str, error: bool = False):
        set_role(self.lbl_test, "syncError" if error else "syncStatus")
        self.lbl_test.setText(text)

    def test_connection(self):
        provider = self._provider()
        if provider == "openrouter":
            url, key = OPENROUTER_BASE, self.ed_or_key.text().strip()
            if not key:
                self._set_test_status("Paste your OpenRouter API key first.", error=True)
                return
        else:
            url, key = normalize_ai_url(self.ed_url.text()), self.ed_key.text().strip()
            if not url:
                self._set_test_status("Enter the server address first.", error=True)
                return
            self.ed_url.setText(url)
        self.btn_test.setEnabled(False)
        self._set_test_status("Connecting…")
        worker = AiCheckWorker(provider, url, key, {"X-Title": APP_TITLE} if provider == "openrouter" else {})
        self.worker = worker
        self.pw.keep_worker(worker)
        worker.result.connect(self._on_test_result)
        worker.start()

    def _on_test_result(self, info: dict):
        self.worker = None
        self.btn_test.setEnabled(self.chk.isChecked())
        if info["provider"] != self._provider():
            return  # switched provider while testing
        if info["error"]:
            self._set_test_status(info["error"], error=True)
            return
        ids = info["ids"]
        if info["provider"] == "openrouter":
            self.or_models, self.or_pricing = ids, info["pricing"]
            completer = QCompleter(ids, self)
            completer.setCaseSensitivity(Qt.CaseSensitivity.CaseInsensitive)
            completer.setFilterMode(Qt.MatchFlag.MatchContains)
            completer.setMaxVisibleItems(12)
            completer.popup().setStyleSheet(
                f"QListView {{ background-color: {PAL['menu']}; color: {PAL['text_soft']}; "
                f"border: 1px solid {PAL['menu_border']}; padding: 4px; font-size: 13px; outline: none; }}"
                f"QListView::item {{ padding: 5px 8px; border-radius: 4px; }}"
                f"QListView::item:selected {{ background-color: {PAL['sel']}; color: {PAL['sel_text']}; }}")
            self.ed_or_model.setCompleter(completer)
            note = info["key_note"] or "Connected."
            self._set_test_status(f"{note} {len(ids)} models to choose from: type in Model to search them.")
            self._update_cost()
            return

        self.local_models = ids
        menu = QMenu(self.btn_pick)
        for mid in ids:
            menu.addAction(mid).triggered.connect(lambda _=False, m=mid: self.ed_model.setText(m))
        self.btn_pick.setMenu(menu)
        self.btn_pick.setEnabled(bool(ids) and self.chk.isChecked())
        current = self.ed_model.text().strip()
        if ids and (not current or (current not in ids and len(ids) == 1)):
            self.ed_model.setText(ids[0])
        if not ids:
            self._set_test_status("Connected, but the server didn't list any models. Type the model name yourself.")
        elif current and current not in ids and len(ids) > 1:
            self._set_test_status(f"Connected, but “{current}” isn't one of its {len(ids)} models. Pick one with Choose.",
                                  error=True)
        else:
            self._set_test_status(f"Connected. {len(ids)} model{'s' if len(ids) != 1 else ''} available.")

    def _update_cost(self):
        model = self.ed_or_model.text().strip()
        if self._provider() != "openrouter" or not model or not self.or_models:
            self.lbl_cost.setText("")
            return
        if model not in self.or_models:
            matches = sum(1 for m in self.or_models if model.lower() in m.lower())
            if matches:  # still typing: point at the list instead of complaining
                set_role(self.lbl_cost, "syncMuted")
                self.lbl_cost.setText(f"{matches} matching model{'s' if matches != 1 else ''}: pick one from the list.")
            else:
                set_role(self.lbl_cost, "syncError")
                self.lbl_cost.setText(f"OpenRouter has no model called “{model}”.")
            return
        set_role(self.lbl_cost, "syncMuted")
        price = self.or_pricing.get(model)
        if price is None:
            self.lbl_cost.setText("")
            return
        cost = 3000 * price[0] + 1500 * price[1]  # a typical song: ~3k tokens in, up to ~1.5k out
        if cost == 0:
            self.lbl_cost.setText("Free model: usually slower, and limited to a few requests a minute.")
        else:
            self.lbl_cost.setText(f"About ${cost:.4f} per song" if cost < 0.01 else f"About ${cost:.3f} per song")

    def save(self):
        enabled, provider = self.chk.isChecked(), self._provider()
        if enabled and provider == "local" and not normalize_ai_url(self.ed_url.text()):
            self._set_test_status("Enter the server address, or turn AI alignment off.", error=True)
            return
        if enabled and provider == "openrouter":
            if not self.ed_or_key.text().strip():
                self._set_test_status("Paste your OpenRouter API key, or turn AI alignment off.", error=True)
                return
            if not self.ed_or_model.text().strip():
                self._set_test_status("Choose a model: run Test connection, then type in Model to search.",
                                      error=True)
                return
        self.pw.save_ai_settings(enabled, normalize_ai_url(self.ed_url.text()), self.ed_model.text().strip(),
                                 self.ed_key.text().strip(), provider=provider,
                                 or_model=self.ed_or_model.text().strip(), or_key=self.ed_or_key.text().strip())
        self.accept()

    def done(self, result):
        if self.worker is not None:
            try:
                self.worker.result.disconnect(self._on_test_result)
            except TypeError:
                pass
        super().done(result)


class _TintDelegate(QStyledItemDelegate):
    """Paints a row tint behind cells flagged with UserRole (stylesheets ignore item backgrounds)."""

    def paint(self, painter, option, index):
        if index.data(Qt.ItemDataRole.UserRole):
            tint = QColor(PAL["accent"])
            tint.setAlphaF(0.14 if PAL["dark"] else 0.10)
            painter.fillRect(option.rect, tint)
        super().paint(painter, option, index)


class TranslationDialog(QDialog):
    """Lyrics ▾ → Add translation: puts a translation line under each sung line of synced lyrics.

    Matching is either line by line, or done by the optional local model; either way the
    result is reviewed (and fixed) in a table before it's saved.
    """

    def __init__(self, player_window, track_path: str, title: str):
        super().__init__(player_window)
        self.pw = player_window
        self.track_path = track_path
        self.dirty = False
        self.worker = None
        self.raw_answer = ""
        self.trans_lines = []    # cleaned translation lines (what T1, T2 … refer to)
        self.skipped = set()     # rows the model gave no answer for
        # All cards of the current lyrics; "rows" are the sung ones (♪ breaks are kept as they are)
        self.cards = [(c.time_ms, c.main_text, c.sub_text) for c in player_window.lyric_cards]
        self.row_cards = [i for i, (_, main, _) in enumerate(self.cards) if main and main != "♪"]
        self.texts = [self.cards[i][2] for i in self.row_cards]

        self.setWindowTitle(f"Add translation — {title}")
        self.resize(920, 680)
        self.setStyleSheet(dialog_stylesheet(PAL))
        outer = QVBoxLayout(self)
        outer.setContentsMargins(18, 18, 18, 14)
        outer.setSpacing(10)
        self.stack = QStackedWidget()
        outer.addWidget(self.stack, 1)

        # ---------- Page 0: paste the translation ----------
        page = QWidget()
        v = QVBoxLayout(page)
        v.setContentsMargins(0, 0, 0, 0)
        v.setSpacing(10)
        intro = QLabel(
            "Paste a translation of these lyrics, one line per line. Each translated line will appear "
            "under the sung line it belongs to. Section labels like [Chorus] are fine: they're skipped."
        )
        intro.setObjectName("syncMuted")
        intro.setWordWrap(True)
        self.txt = QPlainTextEdit()
        self.txt.setPlaceholderText("Paste the translation here…")
        self.lbl_counts = QLabel("")
        self.lbl_counts.setObjectName("syncMuted")
        self.lbl_counts.setWordWrap(True)

        actions = QHBoxLayout()
        self.btn_load = QPushButton("Load .txt / .lrc…")
        self.btn_ai_setup = QPushButton("Set up AI…")
        self.btn_match = QPushButton("Match line by line")
        self.btn_ai = QPushButton("Align with AI")
        actions.addWidget(self.btn_load)
        actions.addStretch()
        actions.addWidget(self.btn_ai_setup)
        actions.addWidget(self.btn_match)
        actions.addWidget(self.btn_ai)

        self.busy_box = QWidget()
        self.busy_box.setObjectName("plainBox")
        busy = QHBoxLayout(self.busy_box)
        busy.setContentsMargins(0, 0, 0, 0)
        self.lbl_busy = QLabel("")
        self.lbl_busy.setObjectName("syncStatus")
        self.progress = QProgressBar()
        self.progress.setTextVisible(False)
        self.progress.setFixedHeight(6)
        self.btn_cancel_ai = QPushButton("Cancel")
        busy.addWidget(self.lbl_busy)
        busy.addWidget(self.progress, 1)
        busy.addWidget(self.btn_cancel_ai)
        self.busy_box.hide()

        v.addWidget(intro)
        v.addWidget(self.txt, 1)
        v.addWidget(self.lbl_counts)
        v.addLayout(actions)
        v.addWidget(self.busy_box)
        self.stack.addWidget(page)

        # ---------- Page 1: review ----------
        page = QWidget()
        v = QVBoxLayout(page)
        v.setContentsMargins(0, 0, 0, 0)
        v.setSpacing(10)
        self.table = QTableWidget(0, 3)
        self.table.setHorizontalHeaderLabels(["Time", "Sung line", "Translation"])
        self.table.verticalHeader().hide()
        self.table.setShowGrid(False)
        self.table.setWordWrap(True)
        self.table.setSelectionBehavior(QAbstractItemView.SelectionBehavior.SelectRows)
        self.table.setSelectionMode(QAbstractItemView.SelectionMode.SingleSelection)
        self.table.setEditTriggers(QAbstractItemView.EditTrigger.DoubleClicked
                                   | QAbstractItemView.EditTrigger.EditKeyPressed)
        self.table.setVerticalScrollMode(QAbstractItemView.ScrollMode.ScrollPerPixel)
        header = self.table.horizontalHeader()
        header.setHighlightSections(False)
        header.setSectionResizeMode(0, QHeaderView.ResizeMode.Fixed)
        header.resizeSection(0, 78)
        header.setSectionResizeMode(1, QHeaderView.ResizeMode.Stretch)
        header.setSectionResizeMode(2, QHeaderView.ResizeMode.Stretch)
        header.setDefaultAlignment(Qt.AlignmentFlag.AlignLeft | Qt.AlignmentFlag.AlignVCenter)
        self.table.setItemDelegate(_TintDelegate(self.table))

        tools = QHBoxLayout()
        self.btn_shift_down = QPushButton("Shift down")
        self.btn_shift_down.setToolTip("Push this translation and the ones after it one line down")
        self.btn_shift_up = QPushButton("Shift up")
        self.btn_shift_up.setToolTip("Remove this line's translation and pull the ones after it up")
        self.btn_clear = QPushButton("Clear")
        self.btn_clear.setToolTip("Remove this line's translation")
        self.btn_romaji = QPushButton("あ")
        self.btn_romaji.setObjectName("toggle")
        self.btn_romaji.setCheckable(True)
        self.btn_romaji.setFixedWidth(36)
        self.btn_romaji.setToolTip("Show the sung lines as romaji")
        self.lbl_cover = QLabel("")
        self.lbl_cover.setObjectName("syncMuted")
        tools.addWidget(self.btn_shift_down)
        tools.addWidget(self.btn_shift_up)
        tools.addWidget(self.btn_clear)
        tools.addSpacing(8)
        tools.addWidget(self.lbl_cover, 1)
        tools.addWidget(self.btn_romaji)

        bottom = QHBoxLayout()
        self.btn_edit = QPushButton("←  Edit translation")
        self.btn_preview = QPushButton("Preview in player")
        self.btn_embed = QPushButton("Embed in file")
        self.btn_embed.setToolTip("Write the lyrics with the translation into the song itself")
        self.btn_save = QPushButton("Save .lrc")
        self.btn_save.setObjectName("primary")
        bottom.addWidget(self.btn_edit)
        bottom.addStretch()
        bottom.addWidget(self.btn_preview)
        bottom.addWidget(self.btn_embed)
        bottom.addWidget(self.btn_save)

        hint = QLabel("Double-click a time to hear that line   ·   Double-click a translation to edit it   ·   "
                      "Tinted rows have no translation")
        hint.setObjectName("syncHelp")
        hint.setWordWrap(True)
        v.addWidget(self.table, 1)
        v.addLayout(tools)
        v.addLayout(bottom)
        v.addWidget(hint)
        self.stack.addWidget(page)

        status_row = QHBoxLayout()
        self.lbl_status = QLabel("")
        self.lbl_status.setObjectName("syncStatus")
        self.lbl_status.setWordWrap(True)
        self.btn_raw = QPushButton("Show model's answer")
        self.btn_raw.hide()
        status_row.addWidget(self.lbl_status, 1)
        status_row.addWidget(self.btn_raw)
        outer.addLayout(status_row)

        for b in self.findChildren(QPushButton):
            b.setAutoDefault(False)
            b.setDefault(False)

        # ---------- Signals ----------
        self.txt.textChanged.connect(self._update_counts)
        self.btn_load.clicked.connect(self._load_file)
        self.btn_ai_setup.clicked.connect(lambda: self.pw.open_ai_settings())
        self.btn_match.clicked.connect(self.match_lines)
        self.btn_ai.clicked.connect(self.align_with_ai)
        self.btn_cancel_ai.clicked.connect(self.cancel_ai)
        self.btn_shift_down.clicked.connect(self.shift_down)
        self.btn_shift_up.clicked.connect(self.shift_up)
        self.btn_clear.clicked.connect(self.clear_line)
        self.btn_romaji.clicked.connect(self._toggle_romaji)
        self.btn_edit.clicked.connect(self.back_to_text)
        self.btn_preview.clicked.connect(self.preview)
        self.btn_embed.clicked.connect(self.embed)
        self.btn_save.clicked.connect(self.save)
        self.btn_raw.clicked.connect(self.show_raw_answer)
        self.table.itemChanged.connect(self._on_item_changed)
        self.table.cellDoubleClicked.connect(self._on_cell_double_clicked)

        self.update_ai_buttons()
        if any(self.texts):
            # These lyrics already carry a translation: start in the review table
            self.txt.setPlainText("\n".join(t for t in self.texts if t))
            self.trans_lines = clean_translation_lines(self.txt.toPlainText())
            self._fill_table()
            self.stack.setCurrentIndex(1)
            self._set_status("Loaded the translation that's already in these lyrics.")
        else:
            self._update_counts()

    # ------------------------------------------------------------------ helpers
    def _track_ok(self) -> bool:
        pw = self.pw
        return 0 <= pw.current_track_idx < len(pw.playlist) and pw.playlist[pw.current_track_idx] == self.track_path

    def _originals(self):
        return [self.cards[i][1] for i in self.row_cards]

    def _set_status(self, text: str, error: bool = False):
        set_role(self.lbl_status, "syncError" if error else "syncStatus")
        self.lbl_status.setText(text)

    def update_ai_buttons(self):
        cfg = self.pw.ai_settings()
        on = cfg["ready"]
        self.btn_ai.setVisible(on)
        self.btn_ai_setup.setVisible(not on)
        set_role(self.btn_ai, "primary")
        set_role(self.btn_match, "" if on else "primary")
        if on:
            self.btn_ai.setToolTip(f"Uses {cfg['model'] or 'the model'} on {cfg['where']}")

    def apply_theme(self):
        self.setStyleSheet(dialog_stylesheet(PAL))
        if self.table.rowCount():
            self._fill_table()

    def _update_counts(self):
        lines = clean_translation_lines(self.txt.toPlainText())
        content = sum(1 for l in lines if not is_section_header(l))
        n = len(self.row_cards)
        text = f"{n} sung lines   ·   {content} translation lines"
        if content and content == n:
            text += "   ·   same count, so matching line by line should work"
        elif content:
            text += "   ·   different counts: some lines are split, joined or repeated"
        self.lbl_counts.setText(text if content else f"{n} sung lines")

    def _load_file(self):
        start_dir = os.path.dirname(self.track_path)
        path, _ = QFileDialog.getOpenFileName(self, "Load translation", start_dir,
                                              "Lyrics (*.txt *.lrc);;All files (*)")
        if path:
            with open(path, "r", encoding="utf-8-sig", errors="ignore") as f:
                self.txt.setPlainText(f.read())

    def _set_busy(self, busy: bool):
        self.busy_box.setVisible(busy)
        for w in (self.btn_load, self.btn_match, self.btn_ai, self.btn_ai_setup):
            w.setEnabled(not busy)
        self.txt.setReadOnly(busy)

    # ------------------------------------------------------------------ matching
    def match_lines(self):
        lines = clean_translation_lines(self.txt.toPlainText())
        content = [l for l in lines if not is_section_header(l)]
        if not content:
            self._set_status("Paste a translation first.", error=True)
            return
        self.trans_lines = lines
        self.texts = match_in_order(len(self.row_cards), lines)
        self.skipped = set()
        self.btn_raw.hide()
        self.dirty = True
        self._fill_table()
        self.stack.setCurrentIndex(1)
        n = len(self.row_cards)
        if len(content) == n:
            self._set_status("Matched line by line. Play a few lines to check they line up.")
        else:
            self._set_status(f"Matched line by line, but the counts differ ({n} sung, {len(content)} translated), "
                             "so later lines may be off. Fix them with Shift down / up, or use AI alignment.")

    def align_with_ai(self):
        cfg = self.pw.ai_settings()
        if not cfg["ready"]:
            self.update_ai_buttons()
            self._set_status("AI alignment isn't set up. Use Set up AI…", error=True)
            return
        lines = clean_translation_lines(self.txt.toPlainText())
        if not any(not is_section_header(l) for l in lines):
            self._set_status("Paste a translation first.", error=True)
            return
        self.trans_lines = lines
        self.raw_answer = ""
        self.btn_raw.hide()
        messages = build_alignment_messages(self._originals(), lines)
        worker = AiAlignWorker(cfg["url"], cfg["model"], cfg["key"], messages, len(self.row_cards), cfg["headers"])
        self.worker = worker
        self.pw.keep_worker(worker)
        worker.progress.connect(self._on_ai_progress)
        worker.succeeded.connect(self._on_ai_done)
        worker.failed.connect(self._on_ai_failed)
        self._model_name = cfg["model"] or "the model"
        self._waiting_text = ("Waiting for OpenRouter…" if cfg["provider"] == "openrouter"
                              else "Waiting for the model… (a big model can take a minute to start)")
        self._set_busy(True)
        self._set_status("")
        self._on_ai_progress("waiting", 0)
        worker.start()

    def _on_ai_progress(self, phase: str, answered: int):
        n = len(self.row_cards)
        if phase == "answering":
            self.progress.setRange(0, n)
            self.progress.setValue(min(answered, n))
            self.lbl_busy.setText(f"Matching lines… {min(answered, n)} of {n}")
        else:
            self.progress.setRange(0, 0)  # busy indicator
            self.lbl_busy.setText("The model is thinking…" if phase == "thinking" else self._waiting_text)

    def _on_ai_done(self, answer: str, finish: str):
        self.worker = None
        self._set_busy(False)
        self.raw_answer = answer
        self.btn_raw.show()
        n = len(self.row_cards)
        mapping = parse_alignment(answer, n, self.trans_lines)
        cut_short = finish == "length"
        if len(mapping) < max(1, n // 2):
            why = ("It stopped before finishing (it ran out of room). "
                   if cut_short else "Its answer didn't use the expected “O1: T1” format. ")
            self._set_status(f"The model didn't produce a usable alignment. {why}"
                             "Try again, or use Match line by line.", error=True)
            return
        texts = alignment_to_texts(mapping, n, self.trans_lines)
        self.skipped = {i for i, t in enumerate(texts) if t is None}
        self.texts = [t or "" for t in texts]
        self.dirty = True
        self._fill_table()
        self.stack.setCurrentIndex(1)
        if self.skipped:
            extra = " It stopped early." if cut_short else ""
            self._set_status(f"Aligned by {self._model_name}, but it skipped {len(self.skipped)} line"
                             f"{'s' if len(self.skipped) != 1 else ''} (tinted).{extra} Check the result before saving.")
        else:
            self._set_status(f"Aligned by {self._model_name}. Play a few lines to check, then save.")

    def _on_ai_failed(self, message: str, raw: str):
        self.worker = None
        self._set_busy(False)
        self.raw_answer = raw
        self.btn_raw.setVisible(bool(raw.strip()))
        self._set_status(message, error=True)

    def cancel_ai(self):
        if self.worker is not None:
            self.worker.cancel()
            self.worker = None
        self._set_busy(False)
        self._set_status("Cancelled.")

    def show_raw_answer(self):
        box = QMessageBox(self)
        box.setWindowTitle("Model's answer")
        box.setText("This is what the model sent back. It helps when the alignment looks wrong.")
        box.setDetailedText(self.raw_answer or "(empty)")
        box.exec()

    # ------------------------------------------------------------------ review table
    def _sung_display(self, text: str) -> str:
        if self.pw.romaji_enabled:
            return self.pw.romaji.convert(text) or text
        return text

    def _fill_table(self):
        self.btn_romaji.setChecked(self.pw.romaji_enabled)
        self.table.blockSignals(True)
        self.table.setRowCount(len(self.row_cards))
        fixed = Qt.ItemFlag.ItemIsEnabled | Qt.ItemFlag.ItemIsSelectable
        for r, ci in enumerate(self.row_cards):
            t_ms, main, _ = self.cards[ci]
            t_item = QTableWidgetItem(f"{t_ms // 60000}:{(t_ms % 60000) / 1000:04.1f}")
            t_item.setFlags(fixed)
            t_item.setForeground(QColor(PAL["muted"]))
            t_item.setToolTip("Double-click to hear this line")
            s_item = QTableWidgetItem(self._sung_display(main))
            s_item.setFlags(fixed)
            s_item.setToolTip(main)
            tr_item = QTableWidgetItem(self.texts[r])
            tr_item.setFlags(fixed | Qt.ItemFlag.ItemIsEditable)
            self._mark_repeat(tr_item, main)
            self.table.setItem(r, 0, t_item)
            self.table.setItem(r, 1, s_item)
            self.table.setItem(r, 2, tr_item)
            self._style_row(r)
        self.table.blockSignals(False)
        self.table.resizeRowsToContents()
        self._refresh_coverage()

    def _mark_repeat(self, item, sung: str):
        """A translation that just repeats the sung line ("Spinning, spinning") isn't saved; show that."""
        if item.text().strip() and _same_text(item.text(), sung):
            item.setForeground(QColor(PAL["faint"]))
            item.setToolTip("Same as the sung line, so it won't be saved as a translation")
        else:
            item.setData(Qt.ItemDataRole.ForegroundRole, None)

    def _style_row(self, r: int):
        empty = not self.texts[r].strip()
        for c in range(3):
            item = self.table.item(r, c)
            if item is not None:
                item.setData(Qt.ItemDataRole.UserRole, empty)
        tr = self.table.item(r, 2)
        if tr is not None:
            if r in self.skipped and empty:
                tr.setToolTip("The model gave no answer for this line")
            elif not (tr.text().strip() and _same_text(tr.text(), self.cards[self.row_cards[r]][1])):
                tr.setToolTip("")

    def _refresh_coverage(self):
        n = len(self.row_cards)
        filled = sum(1 for t in self.texts if t.strip())
        content = [l for l in self.trans_lines if not is_section_header(l)]
        unused = [l for l in content if not line_is_used(l, self.texts)]
        text = f"{filled} of {n} lines have a translation"
        if unused:
            text += f"   ·   {len(unused)} translation line{'s' if len(unused) != 1 else ''} not used"
            self.lbl_cover.setToolTip("Not used anywhere:\n" + "\n".join(unused[:25]))
        else:
            self.lbl_cover.setToolTip("")
        self.lbl_cover.setText(text)

    def _on_item_changed(self, item):
        if item.column() != 2:
            return
        r = item.row()
        self.texts[r] = item.text().strip()
        self.skipped.discard(r)
        self.dirty = True
        self.table.blockSignals(True)
        self._mark_repeat(item, self.cards[self.row_cards[r]][1])
        self._style_row(r)
        self.table.blockSignals(False)
        self.table.resizeRowToContents(r)
        self._refresh_coverage()

    def _selected_row(self):
        r = self.table.currentRow()
        if not 0 <= r < len(self.texts):
            self._set_status("Click a line first.", error=True)
            return None
        return r

    def _after_edit(self, select_row: int):
        self.dirty = True
        self._fill_table()
        self.table.setCurrentCell(select_row, 2)
        self._set_status("")

    def shift_down(self):
        r = self._selected_row()
        if r is None:
            return
        self.texts.insert(r, "")
        self.texts.pop()
        self.skipped = set()
        self._after_edit(min(r + 1, len(self.texts) - 1))

    def shift_up(self):
        r = self._selected_row()
        if r is None:
            return
        self.texts.pop(r)
        self.texts.append("")
        self.skipped = set()
        self._after_edit(r)

    def clear_line(self):
        r = self._selected_row()
        if r is None:
            return
        self.texts[r] = ""
        self._after_edit(r)

    def _toggle_romaji(self):
        self.pw.btn_romaji.toggle()  # same switch as the player's あ button
        self._fill_table()

    def _on_cell_double_clicked(self, row: int, column: int):
        if column == 2 or not self._track_ok():
            return
        t_ms = self.cards[self.row_cards[row]][0]
        self.pw.seek_to(max(0, t_ms + self.pw.lyrics_offset_ms - 300))
        if self.pw.player.playbackState() != QMediaPlayer.PlaybackState.PlayingState:
            self.pw.player.play()

    def back_to_text(self):
        self.stack.setCurrentIndex(0)
        self._set_status("")
        self._update_counts()
        self.txt.setFocus()

    # ------------------------------------------------------------------ output
    def build_lrc(self) -> str:
        row_of_card = {ci: r for r, ci in enumerate(self.row_cards)}
        lines = []
        for ci, (t_ms, main, _) in enumerate(self.cards):
            stamp = f"[{_fmt_lrc_time(t_ms)}]"
            if ci not in row_of_card:
                lines.append(stamp)  # instrumental break
                continue
            lines.append(stamp + main)
            tr = self.texts[row_of_card[ci]].strip()
            if tr and not _same_text(tr, main):  # "Spinning, spinning" needs no translation line
                lines.append(stamp + tr)  # same timestamp: shown as the line's sub-line
        return "\n".join(lines) + "\n"

    def preview(self):
        if not self._track_ok():
            self._set_status("The player is on another song, so it can't preview this one.", error=True)
            return
        self.pw.parse_and_display_lyrics(self.build_lrc())
        self._set_status("Showing the result in the player (not saved yet).")

    def save(self):
        text = self.build_lrc()
        path = os.path.splitext(self.track_path)[0] + ".lrc"
        if os.path.exists(path):
            answer = QMessageBox.question(
                self, "Save lyrics", f"{os.path.basename(path)} already exists and will be replaced.\n\nSave anyway?")
            if answer != QMessageBox.StandardButton.Yes:
                return
        try:
            with open(path, "w", encoding="utf-8") as f:
                f.write(text)
        except OSError as e:
            QMessageBox.warning(self, "Save lyrics", f"Couldn't save the file:\n{e}")
            return
        self.dirty = False
        if self._track_ok():
            self.pw.parse_and_display_lyrics(text)
        self._set_status(f"Saved {os.path.basename(path)}")

    def embed(self):
        text = self.build_lrc()
        if self.pw.embed_lyrics_in_file(self.track_path, text, parent=self):
            self.dirty = False
            if self._track_ok():
                self.pw.parse_and_display_lyrics(text)
            self._set_status(f"Lyrics with translation embedded in {os.path.basename(self.track_path)}")

    # ------------------------------------------------------------------ events
    def keyPressEvent(self, event):
        if event.key() == Qt.Key.Key_Escape:
            self.close()
            return
        if event.modifiers() & Qt.KeyboardModifier.ControlModifier and event.key() == Qt.Key.Key_S:
            if self.stack.currentIndex() == 1:
                self.save()
            return
        super().keyPressEvent(event)

    def closeEvent(self, event):
        if self.worker is not None:
            answer = QMessageBox.question(self, "Add translation", "The model is still working. Stop it and close?")
            if answer != QMessageBox.StandardButton.Yes:
                event.ignore()
                return
            self.cancel_ai()
        if self.dirty:
            answer = QMessageBox.question(self, "Add translation",
                                          "The translation isn't saved yet. Close and lose it?")
            if answer != QMessageBox.StandardButton.Yes:
                event.ignore()
                return
        super().closeEvent(event)


# ---------------------------------------------------------
# Main Audio Player Window
# ---------------------------------------------------------
_LRC_STAMP = re.compile(r"\[(\d{1,3}):(\d{1,2}(?:[.:]\d{1,3})?)\]")
_LRC_LEADING = re.compile(r"^((?:\[\d{1,3}:\d{1,2}(?:[.:]\d{1,3})?\])+)(.*)$")
_LRC_META = re.compile(r"^\[[A-Za-z#]+:.*\]$")


def write_embedded_lyrics(path: str, text: str):
    """Writes lyrics into the song's tags.

    Works on a copy in the same folder and swaps it in only after the copy reads back
    fine, so a failure can never leave a half-written song behind.
    """
    import shutil
    import mutagen
    from mutagen.id3 import ID3, USLT
    from mutagen.mp4 import MP4

    fd, tmp = tempfile.mkstemp(prefix=".embedding_", suffix=os.path.splitext(path)[1],
                               dir=os.path.dirname(path) or None)
    os.close(fd)
    try:
        shutil.copyfile(path, tmp)
        audio = mutagen.File(tmp)
        if audio is None:
            raise ValueError("This audio format isn't supported for embedding.")
        if audio.tags is None:
            audio.add_tags()
        tags = audio.tags

        if isinstance(audio, MP4):                      # .m4a
            tags["\xa9lyr"] = [text]
        elif isinstance(tags, ID3):                     # .mp3, .wav
            tags.delall("USLT")
            tags.add(USLT(encoding=3, lang="eng", desc="", text=text))
        else:                                           # .flac, .ogg, .opus (Vorbis comments)
            if "UNSYNCEDLYRICS" in tags:
                del tags["UNSYNCEDLYRICS"]              # avoid two competing lyric fields
            tags["LYRICS"] = [text]

        audio.save()
        if mutagen.File(tmp) is None:
            raise ValueError("The updated file couldn't be read back, so the original was left unchanged.")
        os.replace(tmp, path)
    finally:
        if os.path.exists(tmp):
            os.remove(tmp)


def _fmt_lrc_time(ms: int) -> str:
    """mm:ss.xx as used inside LRC timestamps."""
    cs = int(round(max(0, ms) / 10))
    minutes, rest = divmod(cs, 6000)
    seconds, hundredths = divmod(rest, 100)
    return f"{minutes:02d}:{seconds:02d}.{hundredths:02d}"


def _stamp_to_ms(match) -> int:
    minutes = int(match.group(1))
    seconds = float(match.group(2).replace(":", "."))
    return int(round((minutes * 60 + seconds) * 1000))


class LyricsSyncDialog(QDialog):
    """Tap-to-sync editor: plain lyrics in, LRC out.

    Uses the main window's player, so playback, romaji display and the
    lyrics view all stay in one place. Each entry is a dict:
    {"text": str, "time": int | None, "sub": str, "break": bool}
    "sub" is a paired line (translation) that shares the main line's timestamp.
    """

    ROW_VPAD = 9   # space above and below each row's text
    ROW_GAP = 2    # space between a line and its romaji/translation

    HELP = ("Enter  stamp line   ·   Backspace  undo   ·   Space  play/pause   ·   ← →  5 s   ·   "
            "↑ ↓  move ▶   ·   B  instrumental break   ·   [ ]  nudge selected line   ·   Ctrl+S  save")

    def __init__(self, player_window, track_path: str, title: str):
        super().__init__(player_window)
        self.pw = player_window
        self.track_path = track_path
        self.entries = []
        self.cursor = 0      # index of the next line to stamp (the ▶ line)
        self.dirty = False
        self._rows = []      # (marker, time, main, sub) labels per list row

        self.setWindowTitle(f"Sync lyrics — {title}")
        self.resize(860, 680)
        self.setFocusPolicy(Qt.FocusPolicy.StrongFocus)
        self._apply_stylesheet()

        outer = QVBoxLayout(self)
        outer.setContentsMargins(18, 18, 18, 18)
        outer.setSpacing(10)
        self.stack = QStackedWidget()
        outer.addWidget(self.stack)

        # ---------- Page 0: the lyrics text ----------
        page_text = QWidget()
        v0 = QVBoxLayout(page_text)
        v0.setContentsMargins(0, 0, 0, 0)
        v0.setSpacing(10)
        intro = QLabel(
            "Paste the lyrics, one sung line per line. Empty lines are skipped.\n"
            "Lines that already have [mm:ss.xx] timestamps keep them, so you can also fix an existing LRC."
        )
        intro.setObjectName("syncMuted")
        intro.setWordWrap(True)
        self.txt_input = QPlainTextEdit()
        self.txt_input.setPlaceholderText("Paste lyrics here…")
        row0 = QHBoxLayout()
        self.btn_load_file = QPushButton("Load .txt / .lrc…")
        self.btn_use_shown = QPushButton("Use lyrics shown in player")
        self.btn_start = QPushButton("Start syncing  →")
        self.btn_start.setObjectName("primary")
        row0.addWidget(self.btn_load_file)
        row0.addWidget(self.btn_use_shown)
        row0.addStretch()
        row0.addWidget(self.btn_start)
        v0.addWidget(intro)
        v0.addWidget(self.txt_input, 1)
        v0.addLayout(row0)
        self.stack.addWidget(page_text)

        # ---------- Page 1: syncing ----------
        page_sync = QWidget()
        v1 = QVBoxLayout(page_sync)
        v1.setContentsMargins(0, 0, 0, 0)
        v1.setSpacing(10)

        self.lbl_warn = QLabel("")
        self.lbl_warn.setObjectName("syncWarn")
        self.lbl_warn.setWordWrap(True)
        self.lbl_warn.hide()

        self.list = QListWidget()
        self.list.setObjectName("syncList")
        self.list.setFocusPolicy(Qt.FocusPolicy.NoFocus)
        self.list.setHorizontalScrollBarPolicy(Qt.ScrollBarPolicy.ScrollBarAlwaysOff)

        transport = QHBoxLayout()
        transport.setSpacing(6)
        self.btn_restart = QPushButton("Restart")
        self.btn_back = QPushButton("−5 s")
        self.btn_play = QPushButton()
        self.btn_play.setIconSize(QSize(18, 18))
        self.btn_play.setFixedWidth(56)
        self.btn_fwd = QPushButton("+5 s")
        self.lbl_progress = QLabel("")
        self.lbl_progress.setObjectName("syncProgress")
        self.lbl_time = QLabel("00:00.00")
        self.lbl_time.setFont(QFont("Consolas", 13))
        self.lbl_time.setObjectName("syncTime")
        self.btn_stamp = QPushButton("Stamp line   ⏎")
        self.btn_stamp.setObjectName("primary")
        for w in (self.btn_restart, self.btn_back, self.btn_play, self.btn_fwd):
            transport.addWidget(w)
        transport.addWidget(self.lbl_time)
        transport.addWidget(self.lbl_progress)
        transport.addStretch()
        transport.addWidget(self.btn_stamp)

        bottom = QHBoxLayout()
        bottom.setSpacing(6)
        self.btn_edit = QPushButton("←  Edit text")
        self.btn_break = QPushButton("Add ♪ break")
        self.btn_preview = QPushButton("Preview in player")
        self.btn_copy = QPushButton("Copy LRC")
        self.btn_embed = QPushButton("Embed in file")
        self.btn_embed.setToolTip("Write the lyrics into the song itself, so they travel with it when you share it")
        self.btn_save = QPushButton("Save .lrc")
        self.btn_save.setObjectName("primary")
        bottom.addWidget(self.btn_edit)
        bottom.addWidget(self.btn_break)
        bottom.addStretch()
        bottom.addWidget(self.btn_preview)
        bottom.addWidget(self.btn_copy)
        bottom.addWidget(self.btn_embed)
        bottom.addWidget(self.btn_save)

        self.lbl_help = QLabel(self.HELP)
        self.lbl_help.setObjectName("syncHelp")
        self.lbl_help.setWordWrap(True)
        self.lbl_status = QLabel("")
        self.lbl_status.setObjectName("syncStatus")

        v1.addWidget(self.lbl_warn)
        v1.addWidget(self.list, 1)
        v1.addLayout(transport)
        v1.addLayout(bottom)
        v1.addWidget(self.lbl_status)
        v1.addWidget(self.lbl_help)
        self.stack.addWidget(page_sync)

        # Buttons on the sync page must not take keyboard focus: the dialog itself
        # handles Enter / Space / arrows. No button may react to Enter as "default".
        for b in self.findChildren(QPushButton):
            b.setAutoDefault(False)
            b.setDefault(False)
        for b in page_sync.findChildren(QPushButton):
            b.setFocusPolicy(Qt.FocusPolicy.NoFocus)

        # ---------- Signals ----------
        self.btn_load_file.clicked.connect(self._load_file)
        self.btn_use_shown.clicked.connect(lambda: self.txt_input.setPlainText(self._shown_lyrics_text()))
        self.btn_start.clicked.connect(self.start_syncing)
        self.btn_restart.clicked.connect(lambda: self.pw.seek_to(0))
        self.btn_back.clicked.connect(lambda: self._seek_relative(-5000))
        self.btn_fwd.clicked.connect(lambda: self._seek_relative(+5000))
        self.btn_play.clicked.connect(lambda: self.pw.toggle_play())
        self.btn_stamp.clicked.connect(self.stamp)
        self.btn_edit.clicked.connect(self.back_to_text)
        self.btn_break.clicked.connect(self.insert_break)
        self.btn_preview.clicked.connect(self.preview)
        self.btn_copy.clicked.connect(self.copy_lrc)
        self.btn_save.clicked.connect(self.save)
        self.btn_embed.clicked.connect(self.embed)
        self.list.itemClicked.connect(self._on_item_clicked)
        self.list.itemDoubleClicked.connect(self._on_item_double_clicked)

        self._icon_play = make_media_icon("play")
        self._icon_pause = make_media_icon("pause")
        self.btn_play.setIcon(self._icon_play)
        self.timer = QTimer(self)
        self.timer.setInterval(80)
        self.timer.timeout.connect(self._tick)
        self.timer.start()

        # Start from whatever the player shows (e.g. an LRC that is out of sync)
        shown = self._shown_lyrics_text()
        self.txt_input.setPlainText(shown)
        self.btn_use_shown.setEnabled(bool(shown))
        self.stack.setCurrentIndex(0)

    # ------------------------------------------------------------------ theme
    def _apply_stylesheet(self):
        self.setStyleSheet(dialog_stylesheet(PAL))

    def apply_theme(self):
        """Called by the player when the theme or accent changes."""
        self._apply_stylesheet()
        self._icon_play = make_media_icon("play")
        self._icon_pause = make_media_icon("pause")
        if self.entries and self._rows:
            self._refresh(scroll=False)

    # ------------------------------------------------------------------ helpers
    def _track_ok(self) -> bool:
        pw = self.pw
        return 0 <= pw.current_track_idx < len(pw.playlist) and pw.playlist[pw.current_track_idx] == self.track_path

    def _seek_relative(self, delta_ms: int):
        if self._track_ok():
            self.pw.seek_to(max(0, self.pw.player.position() + delta_ms))

    def _shown_lyrics_text(self) -> str:
        """The lyrics currently in the player, as text (with timestamps if they are synced)."""
        if not self._track_ok():
            return ""
        return self.pw.current_lyrics_text().rstrip("\n")

    def _load_file(self):
        start_dir = os.path.dirname(self.track_path)
        path, _ = QFileDialog.getOpenFileName(self, "Load lyrics", start_dir, "Lyrics (*.lrc *.txt);;All files (*)")
        if path:
            with open(path, "r", encoding="utf-8-sig", errors="ignore") as f:
                self.txt_input.setPlainText(f.read())

    @staticmethod
    def _parse_text(text: str):
        entries = []
        for raw in text.splitlines():
            line = raw.strip()
            if not line or _LRC_META.match(line):
                continue
            m = _LRC_LEADING.match(line)
            if m:
                body = m.group(2).strip()
                if body == "♪":
                    body = ""
                for sm in _LRC_STAMP.finditer(m.group(1)):
                    entries.append({"text": body, "time": _stamp_to_ms(sm), "sub": "", "break": not body})
            else:
                entries.append({"text": line, "time": None, "sub": "", "break": False})

        if entries and all(e["time"] is not None for e in entries):
            entries.sort(key=lambda e: e["time"])  # stable: a translation stays after its line

        # A line sharing its timestamp with the previous one is that line's translation
        # (the same pairing rule the player uses), so it moves together with it.
        paired = []
        for e in entries:
            prev = paired[-1] if paired else None
            if (prev and e["time"] is not None and prev["time"] is not None
                    and abs(e["time"] - prev["time"]) <= 150 and not prev["sub"] and e["text"] and prev["text"]):
                prev["sub"] = e["text"]
            else:
                paired.append(e)
        return paired

    def _entries_to_text(self) -> str:
        out = []
        for e in self.entries:
            if e["time"] is None:
                out.append(e["text"] or "♪")
                if e["sub"]:
                    out.append(e["sub"])
            else:
                stamp = f"[{_fmt_lrc_time(e['time'])}]"
                out.append(stamp + e["text"])
                if e["sub"]:
                    out.append(stamp + e["sub"])
        return "\n".join(out)

    def build_lrc(self):
        """(lrc_text, number_of_lines_without_timestamp)."""
        stamped = sorted((e for e in self.entries if e["time"] is not None), key=lambda e: e["time"])
        lines = []
        for e in stamped:
            stamp = f"[{_fmt_lrc_time(e['time'])}]"
            lines.append(stamp + e["text"])
            if e["sub"]:
                lines.append(stamp + e["sub"])
        missing = sum(1 for e in self.entries if e["time"] is None)
        return ("\n".join(lines) + "\n") if lines else "", missing

    # ------------------------------------------------------------------ pages
    def start_syncing(self):
        entries = self._parse_text(self.txt_input.toPlainText())
        if not entries:
            QMessageBox.information(self, "Sync lyrics", "Paste some lyrics first.")
            return
        self.entries = entries
        first_open = next((i for i, e in enumerate(entries) if e["time"] is None), None)
        self.cursor = first_open if first_open is not None else 0
        self._rebuild_rows()
        self.stack.setCurrentIndex(1)
        self.setFocus()
        self._set_status("Press Space to play, then Enter whenever the ▶ line starts.")

    def back_to_text(self):
        self.txt_input.setPlainText(self._entries_to_text())
        self.stack.setCurrentIndex(0)
        self.txt_input.setFocus()

    # ------------------------------------------------------------------ list
    def _rebuild_rows(self):
        self.list.clear()
        self._rows = []
        for _ in self.entries:
            w = QWidget()
            w.setObjectName("syncRow")
            w.setAttribute(Qt.WidgetAttribute.WA_TransparentForMouseEvents)
            h = QHBoxLayout(w)
            h.setContentsMargins(6, self.ROW_VPAD, 6, self.ROW_VPAD)
            h.setSpacing(10)
            marker = QLabel("")
            marker.setFixedWidth(14)
            time_lbl = QLabel("")
            time_lbl.setFixedWidth(78)
            time_lbl.setFont(QFont("Consolas", 11))
            col = QVBoxLayout()
            col.setSpacing(self.ROW_GAP)
            main_lbl = QLabel("")
            main_lbl.setFont(QFont("Segoe UI", 12))
            sub_lbl = QLabel("")
            sub_lbl.setFont(QFont("Segoe UI", 10))
            col.addWidget(main_lbl)
            col.addWidget(sub_lbl)
            h.addWidget(marker)
            h.addWidget(time_lbl)
            h.addLayout(col, 1)
            item = QListWidgetItem()
            self.list.addItem(item)
            self.list.setItemWidget(item, w)
            self._rows.append((marker, time_lbl, main_lbl, sub_lbl))
        self._refresh()

    def _refresh(self, scroll: bool = True):
        latest = -1
        for i, e in enumerate(self.entries):
            marker, time_lbl, main_lbl, sub_lbl = self._rows[i]
            marker.setText("▶" if i == self.cursor else "")
            marker.setStyleSheet(f"color: {PAL['accent']};")

            t = e["time"]
            if t is None:
                time_lbl.setText("--:--.--")
                time_lbl.setStyleSheet(f"color: {PAL['text_disabled']};")
            else:
                time_lbl.setText(_fmt_lrc_time(t))
                # earlier than a line above it: probably a mis-tap
                time_lbl.setStyleSheet(f"color: {PAL['error'] if t < latest else PAL['time_ok']};")
                latest = max(latest, t)

            main, sub = self.pw._display_for(e["text"], e["sub"])
            main_lbl.setText(main or "♪")
            if i == self.cursor:
                main_lbl.setStyleSheet(f"color: {PAL['lyric_active']}; font-weight: bold;")
            elif t is None:
                main_lbl.setStyleSheet(f"color: {PAL['faint2']};")
            else:
                main_lbl.setStyleSheet(f"color: {PAL['text_soft']};")
            sub_lbl.setText(sub)
            sub_lbl.setStyleSheet(f"color: {PAL['lyric_inactive_sub']};")
            sub_lbl.setVisible(bool(sub))

            # Height from the real (style-sheet-applied) fonts, so rows never get squeezed
            main_lbl.ensurePolished()
            sub_lbl.ensurePolished()
            row_h = 2 * self.ROW_VPAD + main_lbl.fontMetrics().height() + 2
            if sub:
                row_h += self.ROW_GAP + sub_lbl.fontMetrics().height()
            item = self.list.item(i)
            widget = self.list.itemWidget(item)
            widget.setFixedHeight(row_h)
            item.setSizeHint(QSize(widget.sizeHint().width(), row_h))

        done = sum(1 for e in self.entries if e["time"] is not None)
        self.lbl_progress.setText(f"{done} / {len(self.entries)} stamped")
        if scroll and self.entries:
            row = min(self.cursor, len(self.entries) - 1)
            self.list.scrollToItem(self.list.item(row), QListWidget.ScrollHint.PositionAtCenter)

    def _select(self, row: int):
        if 0 <= row < self.list.count():
            self.list.setCurrentRow(row)

    def _set_status(self, text: str):
        self.lbl_status.setText(text)

    def _on_item_clicked(self, item):
        self.cursor = self.list.row(item)
        self._refresh(scroll=False)

    def _on_item_double_clicked(self, item):
        t = self.entries[self.list.row(item)]["time"]
        if t is not None and self._track_ok():
            self.pw.seek_to(max(0, t - 1500))  # replay from just before the line
            if self.pw.player.playbackState() != QMediaPlayer.PlaybackState.PlayingState:
                self.pw.player.play()

    # ------------------------------------------------------------------ actions
    def stamp(self):
        if not self.entries or not self._track_ok():
            return
        if self.cursor >= len(self.entries):
            self._set_status("Every line is stamped. Save when you're happy with it.")
            return
        self.entries[self.cursor]["time"] = self.pw.player.position()
        self._select(self.cursor)
        self.cursor += 1
        self.dirty = True
        self._refresh()
        if self.cursor >= len(self.entries):
            self._set_status("That was the last line. Preview it in the player, then save.")
        else:
            self._set_status("")

    def undo(self):
        if self.cursor <= 0 or not self.entries:
            return
        i = self.cursor - 1
        e = self.entries[i]
        t = e["time"]
        if e["break"]:
            del self.entries[i]
            self.cursor = i
            self._rebuild_rows()
        else:
            e["time"] = None
            self.cursor = i
            self._refresh()
        self._select(self.cursor)
        if self._track_ok():
            # rewind a little before the line so it can be stamped again
            self.pw.seek_to(max(0, (t if t is not None else self.pw.player.position()) - 3000))
        self.dirty = True
        self._set_status("Undone. Playback jumped back 3 s so you can stamp the line again.")

    def insert_break(self):
        if not self.entries or not self._track_ok():
            return
        pos = self.pw.player.position()
        self.entries.insert(self.cursor, {"text": "", "time": pos, "sub": "", "break": True})
        self.cursor += 1
        self.dirty = True
        self._rebuild_rows()
        self._select(self.cursor - 1)
        self._set_status("Added an instrumental break (shown as ♪ in the player).")

    def nudge_selected(self, delta_ms: int):
        row = self.list.currentRow()
        if not (0 <= row < len(self.entries)) or self.entries[row]["time"] is None:
            self._set_status("Click a stamped line first to nudge it.")
            return
        self.entries[row]["time"] = max(0, self.entries[row]["time"] + delta_ms)
        self.dirty = True
        self._refresh(scroll=False)

    def move_cursor(self, delta: int):
        if not self.entries:
            return
        self.cursor = max(0, min(len(self.entries) - 1, self.cursor + delta))
        self._select(self.cursor)
        self._refresh()

    def preview(self):
        text, _ = self.build_lrc()
        if not text:
            self._set_status("Nothing is stamped yet.")
            return
        if not self._track_ok():
            self._set_status("The player is on another song, so it can't preview this one.")
            return
        self.pw.parse_and_display_lyrics(text)
        self._set_status("Showing the result in the player (not saved yet).")

    def copy_lrc(self):
        text, _ = self.build_lrc()
        if text:
            QApplication.clipboard().setText(text)
            self._set_status("LRC copied to the clipboard.")

    def save(self):
        text, missing = self.build_lrc()
        if not text:
            self._set_status("Nothing is stamped yet.")
            return
        path = os.path.splitext(self.track_path)[0] + ".lrc"
        notes = []
        if missing:
            notes.append(f"{missing} line(s) have no timestamp and will be left out.")
        if os.path.exists(path):
            notes.append(f"{os.path.basename(path)} already exists and will be replaced.")
        if notes:
            answer = QMessageBox.question(self, "Save lyrics", "\n".join(notes) + "\n\nSave anyway?")
            if answer != QMessageBox.StandardButton.Yes:
                return
        try:
            with open(path, "w", encoding="utf-8") as f:
                f.write(text)
        except OSError as e:
            QMessageBox.warning(self, "Save lyrics", f"Couldn't save the file:\n{e}")
            return
        self.dirty = False
        if self._track_ok():
            self.pw.parse_and_display_lyrics(text)
        self._set_status(f"Saved {os.path.basename(path)}")

    def embed(self):
        text, missing = self.build_lrc()
        if not text:
            self._set_status("Nothing is stamped yet.")
            return
        if missing:
            answer = QMessageBox.question(
                self, "Embed lyrics",
                f"{missing} line(s) have no timestamp and will be left out.\n\nContinue?")
            if answer != QMessageBox.StandardButton.Yes:
                return
        if self.pw.embed_lyrics_in_file(self.track_path, text, parent=self):
            self.dirty = False
            if self._track_ok():
                self.pw.parse_and_display_lyrics(text)
            self._set_status(f"Lyrics embedded in {os.path.basename(self.track_path)}")

    # ------------------------------------------------------------------ events
    def _tick(self):
        ok = self._track_ok()
        playing = self.pw.player.playbackState() == QMediaPlayer.PlaybackState.PlayingState
        self.btn_play.setIcon(self._icon_pause if playing else self._icon_play)
        self.lbl_time.setText(_fmt_lrc_time(self.pw.player.position()) if ok else "--:--.--")
        for b in (self.btn_stamp, self.btn_break, self.btn_restart, self.btn_back, self.btn_fwd, self.btn_preview):
            b.setEnabled(ok)
        if ok:
            self.lbl_warn.hide()
        else:
            self.lbl_warn.setText(
                f"The player switched to another song. Go back to “{os.path.basename(self.track_path)}” "
                "to keep syncing. Saving still writes the lyrics for the right song."
            )
            self.lbl_warn.show()

    def keyPressEvent(self, event):
        key = event.key()
        mods = event.modifiers()
        if key == Qt.Key.Key_Escape:
            self.close()
            return
        if self.stack.currentIndex() != 1:
            super().keyPressEvent(event)
            return
        ctrl = bool(mods & Qt.KeyboardModifier.ControlModifier)
        if key in (Qt.Key.Key_Return, Qt.Key.Key_Enter):
            self.stamp()
        elif key == Qt.Key.Key_Backspace or (ctrl and key == Qt.Key.Key_Z):
            self.undo()
        elif ctrl and key == Qt.Key.Key_S:
            self.save()
        elif key == Qt.Key.Key_Space:
            self.pw.toggle_play()
        elif key == Qt.Key.Key_Left:
            self._seek_relative(-5000)
        elif key == Qt.Key.Key_Right:
            self._seek_relative(+5000)
        elif key == Qt.Key.Key_Up:
            self.move_cursor(-1)
        elif key == Qt.Key.Key_Down:
            self.move_cursor(+1)
        elif key == Qt.Key.Key_Home:
            self.pw.seek_to(0)
        elif key == Qt.Key.Key_B and not mods:
            self.insert_break()
        elif key == Qt.Key.Key_BracketLeft:
            self.nudge_selected(-100)
        elif key == Qt.Key.Key_BracketRight:
            self.nudge_selected(+100)
        elif key == Qt.Key.Key_R and not mods:
            self.pw.btn_romaji.toggle()
            self._refresh(scroll=False)
        else:
            super().keyPressEvent(event)

    def closeEvent(self, event):
        if self.dirty:
            answer = QMessageBox.question(
                self, "Sync lyrics",
                "You have timestamps that aren't saved yet. Close and lose them?",
            )
            if answer != QMessageBox.StandardButton.Yes:
                event.ignore()
                return
        self.timer.stop()
        super().closeEvent(event)


class AudioLRCPlayer(QMainWindow):
    def __init__(self):
        super().__init__()
        self.setWindowTitle(APP_TITLE)
        self.resize(1150, 780)
        if os.path.exists("app.ico"):
            self.setWindowIcon(QIcon("app.ico"))

        # Audio Core
        self.player = QMediaPlayer()
        self.audio_output = QAudioOutput()
        self.player.setAudioOutput(self.audio_output)
        self.audio_output.setVolume(0.8)
        # Must happen before anything plays (see prime_qt_virtuals)
        prime_qt_virtuals(self.player.errorOccurred)
        prime_qt_virtuals(self.audio_output.volumeChanged)
        # Follow the system's default output (headphones unplugged / plugged in). Qt keeps
        # using the device it started with, so without this the sound just stops.
        self.media_devices = QMediaDevices(self)
        prime_qt_virtuals(self.media_devices.audioOutputsChanged)
        self.media_devices.audioOutputsChanged.connect(self._on_audio_outputs_changed)

        # Playlist / Album Queue Management
        self.playlist = []            # List of absolute file paths
        self.current_track_idx = -1
        self._queue_was_visible = False
        self.source_kind = ""         # "folder" | "file" | "files" (what the playlist was built from)
        self.source_path = ""
        self.play_order = []          # playlist indices in playing order (shuffled copy when shuffle is on)
        self.order_pos = -1           # position of the current track in play_order
        self.shuffle_on = False
        self.repeat_mode = "off"      # "off" | "track" | "album"

        # State Management
        self.lyric_cards = []
        self.current_card_index = -1
        self.is_synced = False
        self.user_is_seeking = False
        self.current_pixmap = None
        self.current_info = {}
        self._sync_dialog = None       # open "Sync lyrics" window, if any
        self._translate_dialog = None  # open "Add translation" window, if any
        self._workers = set()         # keep running fetch threads alive until they finish

        # Lyrics sync offset in ms. Positive = lyrics appear later, negative = earlier.
        self.lyrics_offset_ms = 0

        # Session memory
        self.settings = QSettings()
        self.romaji = RomajiConverter()
        self.romaji_enabled = False
        self._pending_seek_ms = None  # resume position, applied once the media has loaded
        self._windowed_geometry = None
        self._queue_wanted = False
        self._closing = False

        self._init_ui()
        self._init_animations()
        self._connect_signals()

        # OS media integration (Windows SMTC)
        self.media_controls = WindowsMediaControls(self)
        self.media_controls.play_requested.connect(self._smtc_play)
        self.media_controls.pause_requested.connect(self._smtc_pause)
        self.media_controls.next_requested.connect(self.play_next)
        self.media_controls.previous_requested.connect(self.play_previous)
        self.media_controls.seek_requested.connect(self._smtc_seek)

        self._quitting = False
        QApplication.instance().setQuitOnLastWindowClosed(False)  # the tray keeps the app alive
        QApplication.instance().aboutToQuit.connect(self._on_about_to_quit)
        self._init_tray()

        # Restore window/UI state now (before show), playback right after the window appears
        self._restore_session_ui()
        QTimer.singleShot(0, self._restore_playback_session)

        # Periodically checkpoint playback position in case the app is killed
        self._autosave_timer = QTimer(self)
        self._autosave_timer.setInterval(5000)
        self._autosave_timer.timeout.connect(lambda: self._save_session(full=False))
        self._autosave_timer.start()

    def _init_ui(self):
        self.setStyleSheet(app_stylesheet(PAL))

        central_widget = QWidget()
        self.setCentralWidget(central_widget)
        self.main_layout = QHBoxLayout(central_widget)
        self.main_layout.setContentsMargins(25, 25, 25, 25)
        self.main_layout.setSpacing(25)

        # ----------------------------------------
        # Left Panel (Controls & Cover)
        # ----------------------------------------
        self.left_panel = QWidget()
        self.left_panel.setFixedWidth(330)
        left_layout = QVBoxLayout(self.left_panel)
        left_layout.setContentsMargins(0, 0, 0, 0)
        left_layout.setSpacing(10)

        # Action Buttons (2x2 grid; Open and Lyrics are drop-down menus)
        buttons_layout = QGridLayout()
        buttons_layout.setSpacing(6)

        self.btn_open = QPushButton("Open  ▾")
        open_menu = QMenu(self.btn_open)
        self.act_open_file = open_menu.addAction("Audio file…")
        self.act_open_file.setShortcut(QKeySequence("Ctrl+O"))
        self.act_open_album = open_menu.addAction("Album folder…")
        self.act_open_album.setShortcut(QKeySequence("Ctrl+Shift+O"))
        self.btn_open.setMenu(open_menu)

        self.btn_lyrics = QPushButton("Lyrics  ▾")
        lyrics_menu = QMenu(self.btn_lyrics)
        self.act_load_lrc = lyrics_menu.addAction("Load LRC file…")
        self.act_load_lrc.setShortcut(QKeySequence("Ctrl+L"))
        self.act_search_online = lyrics_menu.addAction("Search online (LRCLIB)")
        self.act_search_online.setEnabled(False)
        lyrics_menu.addSeparator()
        self.act_sync_lyrics = lyrics_menu.addAction("Sync lyrics…")
        self.act_sync_lyrics.setShortcut(QKeySequence("Ctrl+T"))
        self.act_sync_lyrics.setEnabled(False)
        self.act_translate = lyrics_menu.addAction("Add translation…")
        self.act_translate.setShortcut(QKeySequence("Ctrl+Shift+T"))
        self.act_translate.setEnabled(False)
        self.act_embed_lyrics = lyrics_menu.addAction("Embed lyrics in file…")
        self.act_embed_lyrics.setEnabled(False)
        self.btn_lyrics.setMenu(lyrics_menu)

        # Menus are separate pop-up windows; adding the actions to the main window too
        # makes their keyboard shortcuts work while the menus are closed.
        for act in (self.act_open_file, self.act_open_album, self.act_load_lrc, self.act_sync_lyrics,
                    self.act_translate):
            self.addAction(act)

        self.btn_view = QPushButton("View  ▾")
        view_menu = QMenu(self.btn_view)
        self.act_details = view_menu.addAction("Track details…")
        self.act_details.setShortcut(QKeySequence("Ctrl+I"))
        self.act_details.setEnabled(False)
        view_menu.addSeparator()

        theme_menu = view_menu.addMenu("Theme")
        self.theme_group = QActionGroup(self)
        for key, label in THEMES:
            act = theme_menu.addAction(label)
            act.setCheckable(True)
            act.setData(key)
            self.theme_group.addAction(act)

        accent_menu = view_menu.addMenu("Accent color")
        self.accent_group = QActionGroup(self)
        for key, label, _ in ACCENTS:
            act = accent_menu.addAction(label)
            act.setCheckable(True)
            act.setData(key)
            self.accent_group.addAction(act)

        viz_menu = view_menu.addMenu("Visualizer")
        self.viz_group = QActionGroup(self)
        for key, label in (("off", "Off"), ("bars", "Spectrum bars"), ("wave", "Soft wave")):
            act = viz_menu.addAction(label)
            act.setCheckable(True)
            act.setData(key)
            self.viz_group.addAction(act)
        self.viz_menu = viz_menu
        if QAudioBufferOutput is None:
            viz_menu.setTitle("Visualizer (needs PyQt6 6.8+)")
            for act in self.viz_group.actions():
                act.setEnabled(act.data() == "off")

        view_menu.addSeparator()
        self.act_close_to_tray = view_menu.addAction("Keep playing in tray when closed")
        self.act_close_to_tray.setCheckable(True)
        self.act_local_ai = view_menu.addAction("AI alignment…")
        self.act_local_ai.setToolTip("Optional: a model that aligns translations, on this computer or OpenRouter")
        self.act_shortcuts = view_menu.addAction("Keyboard shortcuts")
        self.act_shortcuts.setShortcut(QKeySequence("F1"))
        self.btn_view.setMenu(view_menu)
        self.addAction(self.act_details)

        self.btn_queue = QPushButton("☰ Tracklist")
        self.btn_queue.setEnabled(False)

        buttons_layout.addWidget(self.btn_open, 0, 0)
        buttons_layout.addWidget(self.btn_lyrics, 0, 1)
        buttons_layout.addWidget(self.btn_view, 1, 0)
        buttons_layout.addWidget(self.btn_queue, 1, 1)
        left_layout.addLayout(buttons_layout)

        # Cover Art
        self.lbl_cover = QLabel()
        self.lbl_cover.setFixedSize(330, 330)
        self.lbl_cover.setObjectName("coverArt")
        self.lbl_cover.setAlignment(Qt.AlignmentFlag.AlignCenter)
        self.lbl_cover.setText("No Album Art")
        self.lbl_cover.setToolTip("Double-click anywhere or press F11 for fullscreen\nF1 shows all keyboard shortcuts")
        left_layout.addWidget(self.lbl_cover)

        # Track Title & Artist
        self.lbl_title = QLabel("No track loaded")
        self.lbl_title.setObjectName("trackTitle")
        self.lbl_title.setWordWrap(True)
        left_layout.addWidget(self.lbl_title)

        self.lbl_artist = QLabel("")
        self.lbl_artist.setObjectName("trackArtist")
        self.lbl_artist.setWordWrap(True)
        left_layout.addWidget(self.lbl_artist)

        # Seek Bar
        progress_layout = QHBoxLayout()
        self.lbl_current_time = QLabel("0:00")
        self.lbl_current_time.setObjectName("timeLabel")
        self.seek_slider = QSlider(Qt.Orientation.Horizontal)
        self.lbl_total_time = QLabel("0:00")
        self.lbl_total_time.setObjectName("timeLabel")
        progress_layout.addWidget(self.lbl_current_time)
        progress_layout.addWidget(self.seek_slider)
        progress_layout.addWidget(self.lbl_total_time)
        left_layout.addLayout(progress_layout)

        # Gramophone Quality Badge
        self.lbl_specs_badge = QLabel("")
        self.lbl_specs_badge.setObjectName("specsBadge")
        self.lbl_specs_badge.setAlignment(Qt.AlignmentFlag.AlignCenter)
        self.lbl_specs_badge.hide()
        left_layout.addWidget(self.lbl_specs_badge)

        # Audio Controls: Shuffle, Prev, Play, Next, Repeat, Mute + Volume
        controls_layout = QHBoxLayout()
        controls_layout.setSpacing(5)
        self._icon_play = make_media_icon("play")
        self._icon_pause = make_media_icon("pause")
        self.btn_shuffle = QPushButton()
        self.btn_shuffle.setObjectName("iconToggle")
        self.btn_shuffle.setCheckable(True)
        self.btn_shuffle.setFixedWidth(32)
        self.btn_prev = QPushButton()
        self.btn_prev.setObjectName("iconButton")
        self.btn_prev.setFixedWidth(40)
        self.btn_prev.setToolTip("Previous track   ( Ctrl+← )")
        self.btn_play = QPushButton()
        self.btn_play.setObjectName("iconButton")
        self.btn_play.setFixedWidth(52)
        self.btn_play.setToolTip("Play / pause   ( Space )")
        self.btn_next = QPushButton()
        self.btn_next.setObjectName("iconButton")
        self.btn_next.setFixedWidth(40)
        self.btn_next.setToolTip("Next track   ( Ctrl+→ )")
        self.btn_repeat = QPushButton()
        self.btn_repeat.setObjectName("iconToggle")
        self.btn_repeat.setCheckable(True)
        self.btn_repeat.setFixedWidth(32)
        self.btn_mute = QPushButton()
        self.btn_mute.setObjectName("iconFlat")
        self.btn_mute.setFixedWidth(28)
        self.btn_mute.setToolTip("Mute   ( M )")
        for b in (self.btn_shuffle, self.btn_prev, self.btn_play, self.btn_next, self.btn_repeat, self.btn_mute):
            b.setIconSize(QSize(18, 18))
            b.setMinimumHeight(34)
        for b in (self.btn_shuffle, self.btn_repeat, self.btn_mute):
            b.setFocusPolicy(Qt.FocusPolicy.NoFocus)  # keep Space for play/pause

        self.vol_slider = QSlider(Qt.Orientation.Horizontal)
        self.vol_slider.setRange(0, 100)
        self.vol_slider.setValue(80)
        self.vol_slider.setMinimumWidth(50)
        self.vol_slider.setToolTip("Volume")

        controls_layout.addWidget(self.btn_shuffle)
        controls_layout.addWidget(self.btn_prev)
        controls_layout.addWidget(self.btn_play)
        controls_layout.addWidget(self.btn_next)
        controls_layout.addWidget(self.btn_repeat)
        controls_layout.addSpacing(4)
        controls_layout.addWidget(self.btn_mute)
        controls_layout.addWidget(self.vol_slider, 1)
        left_layout.addLayout(controls_layout)

        # Lyrics sync offset:  [ − ]  +0.0 s  [ + ]  [Reset]
        offset_layout = QHBoxLayout()
        lbl_offset_name = QLabel("Lyrics sync:")
        lbl_offset_name.setObjectName("dimLabel")
        self.btn_offset_minus = QPushButton("−")
        self.btn_offset_plus = QPushButton("+")
        self.btn_offset_reset = QPushButton("Reset")
        self.lbl_offset = QLabel("+0.0 s")
        self.lbl_offset.setMinimumWidth(52)
        self.lbl_offset.setAlignment(Qt.AlignmentFlag.AlignCenter)
        self.lbl_offset.setObjectName("offsetValue")
        self.btn_offset_minus.setFixedWidth(32)
        self.btn_offset_plus.setFixedWidth(32)
        self.btn_offset_reset.setFixedWidth(56)
        self.btn_offset_minus.setToolTip("Show lyrics earlier by 0.1 s   ( [ )")
        self.btn_offset_plus.setToolTip("Show lyrics later by 0.1 s   ( ] )")
        self.btn_offset_reset.setToolTip("Reset lyrics offset   ( \\ )")
        self.btn_romaji = QPushButton("あ")
        self.btn_romaji.setCheckable(True)
        self.btn_romaji.setFixedWidth(36)
        self.btn_romaji.setToolTip("Show Japanese lyrics as romaji   ( R )")
        self.btn_romaji.setObjectName("toggle")
        for b in (self.btn_offset_minus, self.btn_offset_plus, self.btn_offset_reset, self.btn_romaji):
            # Keep keyboard focus on the window so Space still toggles playback
            b.setFocusPolicy(Qt.FocusPolicy.NoFocus)
        offset_layout.addWidget(lbl_offset_name)
        offset_layout.addWidget(self.btn_offset_minus)
        offset_layout.addWidget(self.lbl_offset)
        offset_layout.addWidget(self.btn_offset_plus)
        offset_layout.addWidget(self.btn_offset_reset)
        offset_layout.addStretch()
        offset_layout.addWidget(self.btn_romaji)
        left_layout.addLayout(offset_layout)
        left_layout.addStretch()

        self.main_layout.addWidget(self.left_panel)

        # ----------------------------------------
        # Collapsible Album Tracklist Drawer
        # ----------------------------------------
        self.queue_panel = QWidget()
        self.queue_panel.setFixedWidth(280)
        queue_layout = QVBoxLayout(self.queue_panel)
        queue_layout.setContentsMargins(0, 0, 0, 0)
        queue_layout.setSpacing(10)

        queue_header_layout = QHBoxLayout()
        self.lbl_queue_title = QLabel("Album Tracklist")
        self.lbl_queue_title.setObjectName("panelTitle")
        btn_close_queue = QPushButton("✕")
        btn_close_queue.setFixedSize(26, 26)
        btn_close_queue.clicked.connect(self.queue_panel.hide)

        queue_header_layout.addWidget(self.lbl_queue_title)
        queue_header_layout.addStretch()
        queue_header_layout.addWidget(btn_close_queue)
        queue_layout.addLayout(queue_header_layout)

        self.queue_list = QListWidget()
        self.queue_list.setVerticalScrollBarPolicy(Qt.ScrollBarPolicy.ScrollBarAsNeeded)
        self.queue_list.setHorizontalScrollBarPolicy(Qt.ScrollBarPolicy.ScrollBarAlwaysOff)
        queue_layout.addWidget(self.queue_list, 1)

        self.queue_panel.hide()
        self.main_layout.addWidget(self.queue_panel)

        # ----------------------------------------
        # Right Column: Lyrics View
        # ----------------------------------------
        self.scroll_area = QScrollArea()
        self.scroll_area.setWidgetResizable(True)
        self.scroll_area.setVerticalScrollBarPolicy(Qt.ScrollBarPolicy.ScrollBarAlwaysOff)
        self.scroll_area.setHorizontalScrollBarPolicy(Qt.ScrollBarPolicy.ScrollBarAlwaysOff)

        self.scroll_area.viewport().setObjectName("lyricsViewport")
        self.scroll_area.viewport().setAutoFillBackground(False)
        self.lyrics_container = QWidget()
        self.lyrics_container.setObjectName("lyricsContainer")
        self.lyrics_container.setStyleSheet("background: transparent;")
        self.lyrics_layout = QVBoxLayout(self.lyrics_container)
        self.lyrics_layout.setContentsMargins(10, 100, 10, 160)
        self.lyrics_layout.setSpacing(10)
        self.lyrics_layout.addStretch()

        self.scroll_area.setWidget(self.lyrics_container)
        self.visualizer = VisualizerBackdrop(self.player)
        stage_layout = QGridLayout(self.visualizer)
        stage_layout.setContentsMargins(0, 0, 0, 0)
        stage_layout.addWidget(self.scroll_area, 0, 0)
        self.main_layout.addWidget(self.visualizer, 1)

        # Shown while files are dragged over the window
        self.drop_overlay = QLabel(central_widget)
        self.drop_overlay.setObjectName("dropOverlay")
        self.drop_overlay.setAlignment(Qt.AlignmentFlag.AlignCenter)
        self.drop_overlay.setAttribute(Qt.WidgetAttribute.WA_TransparentForMouseEvents)
        self.drop_overlay.hide()
        self.setAcceptDrops(True)

    def apply_theme(self, theme: str = "", accent: str = "", persist: bool = True):
        """Switch theme and/or accent at runtime. Everything re-reads PAL."""
        theme = theme or PAL.get("theme", "dark")
        accent = accent or PAL.get("accent_key", "coral")
        if theme not in dict(THEMES):
            theme = "dark"
        if accent not in ACCENT_HEX:
            accent = "coral"
        set_palette(theme, accent)
        self.setStyleSheet(app_stylesheet(PAL))
        self._refresh_icons()
        self._rescale_cards(self.isFullScreen())
        self._apply_title_bar(PAL["dark"])
        self.visualizer.update()
        for dlg in (getattr(self, "_sync_dialog", None), getattr(self, "_translate_dialog", None)):
            if dlg is not None:
                dlg.apply_theme()
        for act in self.theme_group.actions():
            act.setChecked(act.data() == theme)
        for act in self.accent_group.actions():
            key = act.data()
            act.setChecked(key == accent)
            act.setIcon(make_swatch_icon(ACCENT_HEX[key], key == accent))
        if persist:
            self.settings.setValue("ui/theme", theme)
            self.settings.setValue("ui/accent", accent)

    def _refresh_icons(self):
        self._icon_play = make_media_icon("play")
        self._icon_pause = make_media_icon("pause")
        self.btn_prev.setIcon(make_media_icon("prev"))
        self.btn_next.setIcon(make_media_icon("next"))
        playing = self.player.playbackState() == QMediaPlayer.PlaybackState.PlayingState
        self.btn_play.setIcon(self._icon_pause if playing else self._icon_play)
        self._update_shuffle_repeat_ui()
        self._update_mute_icon()

    def _apply_title_bar(self, dark: bool):
        """Match the Windows title bar to the theme (Windows 10 2004+ / 11)."""
        if sys.platform != "win32":
            return
        try:
            value = ctypes.c_int(1 if dark else 0)
            ctypes.windll.dwmapi.DwmSetWindowAttribute(
                int(self.winId()), 20, ctypes.byref(value), ctypes.sizeof(value))  # DWMWA_USE_IMMERSIVE_DARK_MODE
        except Exception:
            pass

    # -----------------------------------------------------
    # System tray
    # -----------------------------------------------------
    def _init_tray(self):
        self.tray = None
        if not QSystemTrayIcon.isSystemTrayAvailable():
            self.act_close_to_tray.setEnabled(False)
            self.act_close_to_tray.setToolTip("No system tray available")
            return
        icon = QApplication.windowIcon()
        if icon.isNull():
            icon = make_media_icon("play", PAL["accent"], 32)
        self.tray = QSystemTrayIcon(icon, self)
        menu = QMenu(self)
        self.tray_act_now = menu.addAction("Nothing playing")
        self.tray_act_now.setEnabled(False)
        menu.addSeparator()
        self.tray_act_play = menu.addAction("Play")
        self.tray_act_play.triggered.connect(lambda: self.toggle_play())
        menu.addAction("Next").triggered.connect(lambda: self.play_next())
        menu.addAction("Previous").triggered.connect(lambda: self.play_previous())
        menu.addSeparator()
        self.tray_act_shuffle = menu.addAction("Shuffle")
        self.tray_act_shuffle.setCheckable(True)
        self.tray_act_shuffle.triggered.connect(lambda on: self.set_shuffle(on))
        self.tray_act_repeat = menu.addAction("Repeat: off")
        self.tray_act_repeat.triggered.connect(lambda: self.cycle_repeat())
        menu.addSeparator()
        self.tray_act_window = menu.addAction("Hide player")
        self.tray_act_window.triggered.connect(lambda: self.toggle_window())
        menu.addAction("Quit").triggered.connect(lambda: self.quit_app())
        menu.aboutToShow.connect(self._update_tray)
        self.tray.setContextMenu(menu)
        self.tray.activated.connect(self._on_tray_activated)
        self._update_tray()
        self.tray.show()

    def _update_tray(self):
        if self.tray is None:
            return
        info = self.current_info or {}
        has_track = 0 <= self.current_track_idx < len(self.playlist)
        title = info.get("title", "") if has_track else ""
        artist = info.get("artist", "") if has_track else ""
        if artist == "Unknown Artist":
            artist = ""
        now = f"{title} — {artist}" if title and artist else (title or "Nothing playing")
        self.tray_act_now.setText(now if len(now) <= 60 else now[:57] + "…")
        self.tray.setToolTip(now[:120] if has_track else APP_TITLE)
        playing = self.player.playbackState() == QMediaPlayer.PlaybackState.PlayingState
        self.tray_act_play.setText("Pause" if playing else "Play")
        self.tray_act_shuffle.setChecked(self.shuffle_on)
        self.tray_act_repeat.setText({"off": "Repeat: off", "track": "Repeat: this track",
                                      "album": "Repeat: whole album"}[self.repeat_mode])
        self.tray_act_window.setText("Hide player" if self.isVisible() and not self.isMinimized() else "Show player")

    def _on_tray_activated(self, reason):
        R = QSystemTrayIcon.ActivationReason
        if reason in (R.Trigger, R.DoubleClick):
            self.toggle_window()
        elif reason == R.MiddleClick:
            self.toggle_play()

    def toggle_window(self):
        if self.isVisible() and not self.isMinimized():
            self.hide()
        else:
            self.show_window()
        self._update_tray()

    def show_window(self):
        self.setWindowState(self.windowState() & ~Qt.WindowState.WindowMinimized)
        self.show()
        self.raise_()
        self.activateWindow()

    def _close_to_tray(self) -> bool:
        return self.tray is not None and self.act_close_to_tray.isChecked() and not self._quitting

    def quit_app(self):
        self._quitting = True
        if not self.close():  # the sync window may ask to keep unsaved timestamps
            self._quitting = False

    def _on_about_to_quit(self):
        if self.tray is not None:
            self.tray.hide()
        self.media_controls.shutdown()

    def _init_animations(self):
        self.scroll_anim = QPropertyAnimation(self.scroll_area.verticalScrollBar(), b"value")
        self.scroll_anim.setDuration(360)
        self.scroll_anim.setEasingCurve(QEasingCurve.Type.OutCubic)

    def _connect_signals(self):
        self.act_open_file.triggered.connect(lambda: self.open_single_file_dialog())
        self.act_open_album.triggered.connect(lambda: self.open_album_folder_dialog())
        self.act_load_lrc.triggered.connect(lambda: self.open_lrc_dialog())
        self.act_search_online.triggered.connect(lambda: self.search_lyrics_online())
        self.act_sync_lyrics.triggered.connect(lambda: self.open_sync_dialog())
        self.act_translate.triggered.connect(lambda: self.open_translation_dialog())
        self.act_local_ai.triggered.connect(lambda: self.open_ai_settings())
        self.act_embed_lyrics.triggered.connect(lambda: self.embed_shown_lyrics())
        self.act_details.triggered.connect(lambda: self.show_details_dialog())
        self.act_shortcuts.triggered.connect(lambda: self.show_shortcuts())
        self.theme_group.triggered.connect(lambda act: self.apply_theme(theme=act.data()))
        self.accent_group.triggered.connect(lambda act: self.apply_theme(accent=act.data()))
        self.viz_group.triggered.connect(lambda act: self.set_visualizer(act.data()))
        self.act_close_to_tray.toggled.connect(lambda on: self.settings.setValue("ui/close_to_tray", on))
        self.btn_queue.clicked.connect(self.toggle_queue_panel)
        self.btn_romaji.toggled.connect(self.set_romaji_enabled)

        self.btn_play.clicked.connect(self.toggle_play)
        self.btn_prev.clicked.connect(self.play_previous)
        self.btn_next.clicked.connect(self.play_next)
        self.btn_shuffle.clicked.connect(lambda checked: self.set_shuffle(checked))
        self.btn_repeat.clicked.connect(lambda _=False: self.cycle_repeat())
        self.btn_mute.clicked.connect(self.toggle_mute)

        self.seek_slider.sliderPressed.connect(self._on_seek_pressed)
        self.seek_slider.sliderReleased.connect(self._on_seek_released)
        self.vol_slider.valueChanged.connect(self._on_volume_changed)

        self.queue_list.itemDoubleClicked.connect(self._on_queue_item_double_clicked)

        self.btn_offset_minus.clicked.connect(lambda: self.adjust_lyrics_offset(-100))
        self.btn_offset_plus.clicked.connect(lambda: self.adjust_lyrics_offset(+100))
        self.btn_offset_reset.clicked.connect(lambda: self.set_lyrics_offset(0))

        self.player.positionChanged.connect(self._on_position_changed)
        self.player.durationChanged.connect(self._on_duration_changed)
        self.player.mediaStatusChanged.connect(self._on_media_status_changed)
        self.player.playbackStateChanged.connect(self._on_playback_state_changed)

    def mouseDoubleClickEvent(self, event):
        self.toggle_fullscreen()
        super().mouseDoubleClickEvent(event)

    # -----------------------------------------------------
    # File & Album Loading
    # -----------------------------------------------------
    SUPPORTED_EXTS = (".flac", ".mp3", ".wav", ".m4a", ".ogg", ".opus")

    def _dialog_start_dir(self) -> str:
        """Where file dialogs should open: the last album folder / track's folder, if it still exists."""
        candidates = []
        if self.source_kind == "folder":
            candidates.append(self.source_path)
        if 0 <= self.current_track_idx < len(self.playlist):
            candidates.append(os.path.dirname(self.playlist[self.current_track_idx]))
        candidates.append(self.settings.value("playback/source_path", "", type=str))
        for c in candidates:
            if c and os.path.isdir(c):
                return c
            if c and os.path.isfile(c):
                return os.path.dirname(c)
        return ""

    def _scan_album_folder(self, folder: str):
        audio_files = []
        for root, _, files in os.walk(folder):
            for f in files:
                if f.lower().endswith(self.SUPPORTED_EXTS):
                    audio_files.append(os.path.join(root, f))
        # Sort by disc #, track #, and filename
        audio_files.sort(key=get_track_sort_key)
        return audio_files

    def _set_playlist(self, files, kind: str, source: str):
        self.playlist = files
        self.source_kind = kind
        self.source_path = source
        self.current_track_idx = -1
        self._rebuild_order()
        self._populate_queue_list()

    def _rebuild_order(self, first=None):
        """Playing order: album order, or a shuffled copy that starts with `first`."""
        n = len(self.playlist)
        if self.shuffle_on:
            rest = [i for i in range(n) if i != first]
            random.shuffle(rest)
            self.play_order = ([first] if first is not None and 0 <= first < n else []) + rest
        else:
            self.play_order = list(range(n))
        cur = self.current_track_idx
        self.order_pos = self.play_order.index(cur) if 0 <= cur < n else -1
        self._update_navigation()

    def _start_index(self) -> int:
        """Where a freshly opened playlist begins (a random track when shuffling)."""
        return self.play_order[0] if self.play_order else 0

    def play_index(self, index: int):
        """A track picked by the user: under shuffle, the rest of the album is reshuffled after it."""
        if self.shuffle_on:
            self._rebuild_order(first=index)
        self.load_track_by_index(index)

    def _has_next(self) -> bool:
        return bool(self.play_order) and (
            self.order_pos + 1 < len(self.play_order) or self.repeat_mode == "album")

    def _update_navigation(self):
        if hasattr(self, "media_controls"):
            self.media_controls.set_navigation(bool(self.playlist), self._has_next())

    def open_single_file_dialog(self):
        file_path, _ = QFileDialog.getOpenFileName(
            self, "Select Audio File", self._dialog_start_dir(),
            "Audio Files (*.flac *.mp3 *.wav *.m4a *.ogg *.opus)"
        )
        if file_path:
            self.open_file(file_path)

    def open_album_folder_dialog(self):
        folder = QFileDialog.getExistingDirectory(self, "Select Album Folder", self._dialog_start_dir())
        if folder:
            self.open_album_folder(folder)

    def open_file(self, file_path: str):
        self._set_playlist([file_path], "file", file_path)
        self.load_track_by_index(0)

    def open_album_folder(self, folder: str) -> bool:
        audio_files = self._scan_album_folder(folder)
        if not audio_files:
            QMessageBox.information(self, "Open album", f"No audio files found in “{os.path.basename(folder)}”.")
            return False
        self._set_playlist(audio_files, "folder", folder)
        self._show_queue_panel()
        self.load_track_by_index(self._start_index())
        return True

    def open_files(self, files):
        """Several songs (and/or folders) dropped at once."""
        if not files:
            return
        if len(files) == 1:
            self.open_file(files[0])
            return
        self._set_playlist(list(files), "files", os.path.dirname(files[0]))
        self._show_queue_panel()
        self.load_track_by_index(self._start_index())

    def _show_queue_panel(self):
        if self.isFullScreen():
            self._queue_was_visible = True
        else:
            self.queue_panel.show()

    # -----------------------------------------------------
    # Drag & drop
    # -----------------------------------------------------
    LYRICS_EXTS = (".lrc", ".txt")

    def _drop_plan(self, mime):
        """What a drop would do: (action, payload, headline, detail) or None. Cheap: no folder scanning."""
        if not mime.hasUrls():
            return None
        paths = [u.toLocalFile() for u in mime.urls() if u.isLocalFile()]
        paths = [p for p in paths if p]
        if not paths:
            return None
        has_track = 0 <= self.current_track_idx < len(self.playlist)
        if len(paths) == 1 and paths[0].lower().endswith(self.LYRICS_EXTS) and os.path.isfile(paths[0]):
            if not has_track:
                return None
            return ("lyrics", paths[0], "Drop to show these lyrics", os.path.basename(paths[0]))
        folders = [p for p in paths if os.path.isdir(p)]
        audio = [p for p in paths if os.path.isfile(p) and p.lower().endswith(self.SUPPORTED_EXTS)]
        if len(folders) == 1 and not audio:
            return ("folder", folders[0], "Drop to play this album", os.path.basename(folders[0].rstrip("/\\")))
        if len(audio) == 1 and not folders:
            return ("file", audio[0], "Drop to play this song", os.path.basename(audio[0]))
        if folders or audio:
            count = len(folders) + len(audio)
            return ("files", (folders, audio), "Drop to play", f"{count} items")
        return None

    def _show_drop_overlay(self, headline: str, detail: str):
        self.drop_overlay.setText(
            f"<div>{headline}</div>"
            f"<div style='font-size:13px; font-weight:400; color:{PAL['muted']}'>{detail}</div>"
        )
        self._place_drop_overlay()
        self.drop_overlay.raise_()
        self.drop_overlay.show()

    def _place_drop_overlay(self):
        rect = self.centralWidget().rect().adjusted(14, 14, -14, -14)
        self.drop_overlay.setGeometry(rect)

    def resizeEvent(self, event):
        super().resizeEvent(event)
        if self.drop_overlay.isVisible():
            self._place_drop_overlay()

    def dragEnterEvent(self, event):
        plan = self._drop_plan(event.mimeData())
        if plan is None:
            event.ignore()
            return
        self._show_drop_overlay(plan[2], plan[3])
        event.acceptProposedAction()

    def dragMoveEvent(self, event):
        if self.drop_overlay.isVisible():
            event.acceptProposedAction()
        else:
            event.ignore()

    def dragLeaveEvent(self, event):
        self.drop_overlay.hide()
        super().dragLeaveEvent(event)

    def dropEvent(self, event):
        self.drop_overlay.hide()
        plan = self._drop_plan(event.mimeData())
        if plan is None:
            event.ignore()
            return
        event.acceptProposedAction()
        action, payload = plan[0], plan[1]
        # Run after the drop finishes, so Explorer isn't left waiting while an album is scanned
        QTimer.singleShot(0, lambda: self._run_drop(action, payload))

    def _run_drop(self, action, payload):
        if action == "lyrics":
            with open(payload, "r", encoding="utf-8-sig", errors="ignore") as f:
                self.parse_and_display_lyrics(f.read())
        elif action == "folder":
            self.open_album_folder(payload)
        elif action == "file":
            self.open_file(payload)
        elif action == "files":
            folders, audio = payload
            QApplication.setOverrideCursor(Qt.CursorShape.WaitCursor)
            try:
                files = []
                for folder in folders:
                    files.extend(self._scan_album_folder(folder))
                files.extend(sorted(audio, key=get_track_sort_key))
            finally:
                QApplication.restoreOverrideCursor()
            self.open_files(files)

    def _populate_queue_list(self):
        self.queue_list.clear()
        for idx, path in enumerate(self.playlist):
            title = os.path.splitext(os.path.basename(path))[0]
            try:
                import mutagen
                audio = mutagen.File(path)
                if audio:
                    for k in ("title", "tit2", "\xa9nam"):
                        val = audio.get(k) or (audio.tags.get(k) if hasattr(audio, "tags") and audio.tags else None)
                        if val:
                            title = str(val[0] if isinstance(val, list) else getattr(val, "text", str(val)))
                            break
            except Exception:
                pass

            item = QListWidgetItem(f"{idx + 1:02d}. {title}")
            self.queue_list.addItem(item)

        self.btn_queue.setEnabled(len(self.playlist) > 0)
        self.lbl_queue_title.setText(f"Tracklist ({len(self.playlist)})")

    def load_track_by_index(self, index: int, autoplay: bool = True, start_position_ms: int = 0):
        if not (0 <= index < len(self.playlist)):
            return

        self.current_track_idx = index
        if index in self.play_order:
            self.order_pos = self.play_order.index(index)
        else:
            self._rebuild_order(first=index)
        file_path = self.playlist[index]

        # A resume position can only be applied reliably once the media has loaded
        self._pending_seek_ms = start_position_ms if start_position_ms > 0 else None

        self.queue_list.setCurrentRow(index)
        self.player.setSource(QUrl.fromLocalFile(file_path))
        if autoplay:
            self.player.play()
        else:
            self.player.pause()

        embedded_lyrics, cover, info = extract_metadata_and_cover(file_path)
        self.current_info = info
        self.act_details.setEnabled(True)
        self.act_search_online.setEnabled(True)
        self.act_sync_lyrics.setEnabled(True)
        self.act_translate.setEnabled(True)
        self.act_embed_lyrics.setEnabled(True)

        self.lbl_title.setText(info.get("title", os.path.basename(file_path)))
        self.lbl_artist.setText(info.get("artist", ""))

        hd_prefix = "HD  " if info.get("is_hd") else ""
        specs_text = f"{hd_prefix}{info.get('bits_per_sample')}bit / {info.get('sample_rate_khz')}kHz / {info.get('channels')}ch / {info.get('bitrate_kbps')}kbps"
        self.lbl_specs_badge.setText(specs_text)
        self.lbl_specs_badge.show()

        self._update_cover_art(cover)

        self.media_controls.set_track(
            info, cover,
            has_previous=True,  # "previous" restarts the track when it isn't the first one
            has_next=self._has_next(),
        )
        self.media_controls.update_timeline(0, info.get("duration_sec", 0) * 1000, force=True)
        self._update_tray()

        # A .lrc next to the song wins over lyrics embedded in the file: it is the one
        # you created or fixed on purpose (sync editor, phone app, online search).
        base_path = os.path.splitext(file_path)[0]
        lrc_path = base_path + ".lrc"
        if os.path.exists(lrc_path):
            with open(lrc_path, "r", encoding="utf-8-sig", errors="ignore") as f:
                self.parse_and_display_lyrics(f.read())
            return

        if embedded_lyrics:
            self.parse_and_display_lyrics(embedded_lyrics)
            return

        self.search_lyrics_online()

    def toggle_queue_panel(self):
        self.queue_panel.setVisible(not self.queue_panel.isVisible())

    def _on_queue_item_double_clicked(self, item):
        row = self.queue_list.row(item)
        if row != -1:
            self.play_index(row)

    # -----------------------------------------------------
    # Transport Controls (Play / Prev / Next / Auto-Advance)
    # -----------------------------------------------------
    def toggle_play(self):
        # Button label / SMTC status follow playbackStateChanged, so headset and
        # media-key events keep the UI in sync too.
        if self.player.playbackState() == QMediaPlayer.PlaybackState.PlayingState:
            self.player.pause()
        else:
            self.player.play()

    def _on_playback_state_changed(self, state):
        playing = state == QMediaPlayer.PlaybackState.PlayingState
        self.btn_play.setIcon(self._icon_pause if playing else self._icon_play)
        self.media_controls.set_playback_state(state)
        self.media_controls.update_timeline(self.player.position(), self.player.duration(), force=True)
        self._update_tray()
        if state == QMediaPlayer.PlaybackState.PausedState:
            self._save_session(full=False)

    # --- Slots for OS media controls (run on the GUI thread) ---
    def _smtc_play(self):
        if self.playlist and self.player.playbackState() != QMediaPlayer.PlaybackState.PlayingState:
            self.player.play()

    def _smtc_pause(self):
        if self.player.playbackState() == QMediaPlayer.PlaybackState.PlayingState:
            self.player.pause()

    def _smtc_seek(self, ms: int):
        if self.playlist:
            self.seek_to(ms)

    def play_previous(self):
        if not self.playlist:
            return
        # If track has been playing for > 3 seconds, restart it; otherwise go to previous
        if self.player.position() > 3000:
            self.seek_to(0)
        elif self.order_pos > 0:
            self.load_track_by_index(self.play_order[self.order_pos - 1])
        elif self.repeat_mode == "album" and len(self.play_order) > 1:
            self.load_track_by_index(self.play_order[-1])
        else:
            self.seek_to(0)

    def play_next(self):
        if not self.playlist:
            return
        if self.order_pos + 1 < len(self.play_order):
            self.load_track_by_index(self.play_order[self.order_pos + 1])
        elif self.repeat_mode == "album":
            if self.shuffle_on and len(self.playlist) > 1:
                # New round in a new order; the song that just played lands in the
                # second half, so it never plays twice in a row.
                cur = self.current_track_idx
                rest = [i for i in range(len(self.playlist)) if i != cur]
                random.shuffle(rest)
                rest.insert(random.randint(len(rest) // 2 + 1, len(rest)), cur)
                self.play_order = rest
            self.load_track_by_index(self.play_order[0])

    # -----------------------------------------------------
    # Shuffle / repeat / mute
    # -----------------------------------------------------
    REPEAT_CYCLE = ("off", "track", "album")
    REPEAT_TIPS = {
        "off": "Repeat: off   ( L )",
        "track": "Repeat: this track   ( L )",
        "album": "Repeat: whole album   ( L )",
    }

    def set_shuffle(self, on: bool, persist: bool = True):
        self.shuffle_on = bool(on)
        cur = self.current_track_idx
        self._rebuild_order(first=cur if 0 <= cur < len(self.playlist) else None)
        self._update_shuffle_repeat_ui()
        if persist:
            self.settings.setValue("playback/shuffle", self.shuffle_on)

    def cycle_repeat(self):
        i = self.REPEAT_CYCLE.index(self.repeat_mode)
        self.set_repeat(self.REPEAT_CYCLE[(i + 1) % len(self.REPEAT_CYCLE)])

    def set_repeat(self, mode: str, persist: bool = True):
        self.repeat_mode = mode if mode in self.REPEAT_CYCLE else "off"
        # Repeat-track loops inside the player itself, which is gapless
        self.player.setLoops(QMediaPlayer.Loops.Infinite if self.repeat_mode == "track" else QMediaPlayer.Loops.Once)
        self._update_shuffle_repeat_ui()
        self._update_navigation()
        if persist:
            self.settings.setValue("playback/repeat", self.repeat_mode)

    def _update_shuffle_repeat_ui(self):
        on, off = PAL["accent"], PAL["icon_muted"]
        self.btn_shuffle.setChecked(self.shuffle_on)
        self.btn_shuffle.setIcon(make_media_icon("shuffle", on if self.shuffle_on else off))
        self.btn_shuffle.setToolTip(f"Shuffle: {'on' if self.shuffle_on else 'off'}   ( S )")
        active = self.repeat_mode != "off"
        self.btn_repeat.setChecked(active)
        self.btn_repeat.setIcon(make_media_icon(
            "repeat_one" if self.repeat_mode == "track" else "repeat", on if active else off))
        self.btn_repeat.setToolTip(self.REPEAT_TIPS[self.repeat_mode])
        if getattr(self, "tray", None) is not None:
            self._update_tray()

    def _on_audio_outputs_changed(self):
        self._follow_default_output()
        # Some systems announce the new device before they make it the default
        QTimer.singleShot(400, self._follow_default_output)

    def _follow_default_output(self):
        default = QMediaDevices.defaultAudioOutput()
        if default.isNull() or self.audio_output.device().id() == default.id():
            return
        self.audio_output.setDevice(default)

    def set_visualizer(self, mode: str, persist: bool = True):
        self.visualizer.set_mode(mode)
        for act in self.viz_group.actions():
            act.setChecked(act.data() == self.visualizer.mode)
        if persist:
            self.settings.setValue("ui/visualizer", self.visualizer.mode)

    def cycle_visualizer(self):
        modes = VisualizerBackdrop.MODES
        self.set_visualizer(modes[(modes.index(self.visualizer.mode) + 1) % len(modes)])

    def toggle_mute(self):
        self.audio_output.setMuted(not self.audio_output.isMuted())
        self._update_mute_icon()

    def _on_volume_changed(self, value: int):
        self.audio_output.setVolume(value / 100.0)
        if self.audio_output.isMuted() and value > 0:
            self.audio_output.setMuted(False)
            self._update_mute_icon()

    def _update_mute_icon(self):
        muted = self.audio_output.isMuted()
        self.btn_mute.setIcon(make_media_icon("mute" if muted else "volume",
                                              PAL["accent"] if muted else PAL["icon_muted"]))
        self.btn_mute.setToolTip("Unmute   ( M )" if muted else "Mute   ( M )")

    def _on_media_status_changed(self, status):
        MS = QMediaPlayer.MediaStatus

        # Resume position from the previous session, applied once media is ready
        if status in (MS.LoadedMedia, MS.BufferedMedia) and self._pending_seek_ms is not None:
            resume_ms = self._pending_seek_ms
            self._pending_seek_ms = None
            duration = self.player.duration()
            if duration > 0 and resume_ms >= duration - 1500:
                resume_ms = 0  # was at the very end; start over instead
            if resume_ms > 0:
                self.player.setPosition(resume_ms)
                self.media_controls.update_timeline(resume_ms, duration, force=True)

        # Auto-advance to the next track when current one reaches the end
        if status == MS.EndOfMedia:
            if self._editor_open():
                return  # stay on the song whose lyrics are being edited
            if self.repeat_mode == "track":  # normally handled by setLoops; just in case
                self.player.setPosition(0)
                self.player.play()
            elif self._has_next():
                self.play_next()

    def seek_to(self, ms: int):
        self.player.setPosition(ms)
        self.media_controls.update_timeline(ms, self.player.duration(), force=True)

    def _on_position_changed(self, pos_ms):
        if not self.user_is_seeking:
            self.seek_slider.setValue(pos_ms)
            self.lbl_current_time.setText(self._format_time(pos_ms))

        self.media_controls.update_timeline(pos_ms, self.player.duration())
        self._sync_lyrics_to_position(pos_ms)

    def _sync_lyrics_to_position(self, pos_ms: int):
        """Activate the lyric card for pos_ms, taking the lyrics offset into account."""
        if not self.lyric_cards or not self.is_synced:
            return

        adjusted = pos_ms - self.lyrics_offset_ms
        new_index = -1
        for i, card in enumerate(self.lyric_cards):
            if adjusted >= card.time_ms:
                new_index = i
            else:
                break

        if new_index == self.current_card_index:
            return

        if new_index == -1:
            # Before the first line (e.g. after a backwards seek or offset change): deactivate
            if 0 <= self.current_card_index < len(self.lyric_cards):
                self.lyric_cards[self.current_card_index].update_style(
                    is_active=False, is_fullscreen=self.isFullScreen(), is_synced=True, animate=True
                )
            self.current_card_index = -1
            return

        self._update_active_card(new_index)
        self.current_card_index = new_index

    # -----------------------------------------------------
    # Lyrics offset
    # -----------------------------------------------------
    OFFSET_LIMIT_MS = 30000

    def adjust_lyrics_offset(self, delta_ms: int):
        self.set_lyrics_offset(self.lyrics_offset_ms + delta_ms)

    def set_lyrics_offset(self, offset_ms: int, persist: bool = True):
        offset_ms = max(-self.OFFSET_LIMIT_MS, min(self.OFFSET_LIMIT_MS, int(offset_ms)))
        self.lyrics_offset_ms = offset_ms
        self.lbl_offset.setText(f"{offset_ms / 1000:+.1f} s")
        if persist:
            self.settings.setValue("lyrics/offset_ms", offset_ms)
        self._sync_lyrics_to_position(self.player.position())

    def _update_active_card(self, active_index):
        if not self.is_synced:
            return

        is_fs = self.isFullScreen()

        if 0 <= self.current_card_index < len(self.lyric_cards):
            self.lyric_cards[self.current_card_index].update_style(
                is_active=False, is_fullscreen=is_fs, is_synced=True, animate=True
            )

        if 0 <= active_index < len(self.lyric_cards):
            active_card = self.lyric_cards[active_index]
            active_card.update_style(
                is_active=True, is_fullscreen=is_fs, is_synced=True, animate=True
            )
            QTimer.singleShot(25, lambda: self._smooth_scroll_to_card(active_card))

    def _smooth_scroll_to_card(self, active_card):
        if active_card not in self.lyric_cards:
            return  # the lyrics were replaced before this delayed scroll ran
        target_val = active_card.pos().y() - (self.scroll_area.viewport().height() - active_card.height()) // 2
        v_bar = self.scroll_area.verticalScrollBar()
        bounded_target = max(0, min(target_val, v_bar.maximum()))

        self.scroll_anim.stop()
        self.scroll_anim.setStartValue(v_bar.value())
        self.scroll_anim.setEndValue(bounded_target)
        self.scroll_anim.start()

    def _on_duration_changed(self, duration_ms):
        self.seek_slider.setRange(0, duration_ms)
        self.lbl_total_time.setText(self._format_time(duration_ms))
        self.media_controls.update_timeline(self.player.position(), duration_ms, force=True)

    def _on_seek_pressed(self):
        self.user_is_seeking = True

    def _on_seek_released(self):
        self.user_is_seeking = False
        self.seek_to(self.seek_slider.value())

    # -----------------------------------------------------
    # Lyrics Parsing & Searching
    # -----------------------------------------------------
    def search_lyrics_online(self):
        if self.current_track_idx == -1 or not self.playlist:
            return

        current_file = self.playlist[self.current_track_idx]
        self._clear_lyrics()
        placeholder = QLabel("Searching online lyrics (LRCLIB)...")
        placeholder.setObjectName("lyricsStatus")
        self.lyrics_layout.insertWidget(0, placeholder)

        base_path = os.path.splitext(current_file)[0]
        lrc_save_path = base_path + ".lrc"

        worker = LrcFetchWorker(
            title=self.current_info.get("title", ""),
            artist=self.current_info.get("artist", ""),
            duration_sec=self.current_info.get("duration_sec", 0),
            save_path=lrc_save_path
        )
        # Keep a reference until the thread finishes (dropping it mid-run crashes Qt),
        # and ignore results that arrive after the user has moved to another track.
        self._workers.add(worker)
        worker.finished.connect(lambda w=worker: self._workers.discard(w))
        worker.finished_signal.connect(
            lambda text, msg, path=current_file: self._on_online_lyrics_fetched(text, msg, path)
        )
        worker.start()

    def _on_online_lyrics_fetched(self, lyrics_text: str, status_msg: str, requested_path: str = ""):
        if requested_path and not (
            0 <= self.current_track_idx < len(self.playlist)
            and self.playlist[self.current_track_idx] == requested_path
        ):
            return  # stale result for a track that is no longer current
        self._clear_lyrics()
        if lyrics_text:
            self.parse_and_display_lyrics(lyrics_text)
        else:
            placeholder = QLabel("No lyrics found (local or online). Use Lyrics ▾ → Load LRC file to add them manually.")
            placeholder.setObjectName("lyricsEmpty")
            self.lyrics_layout.insertWidget(0, placeholder)

    def current_lyrics_text(self) -> str:
        """The lyrics currently shown, as LRC if they are synced, otherwise as plain text."""
        if not self.lyric_cards:
            return ""
        lines = []
        for card in self.lyric_cards:
            main = "" if card.main_text == "♪" else card.main_text
            if self.is_synced:
                stamp = f"[{_fmt_lrc_time(card.time_ms)}]"
                lines.append(stamp + main)
                if card.sub_text:
                    lines.append(stamp + card.sub_text)
            else:
                lines.append(card.main_text)
        return "\n".join(lines) + "\n"

    def embed_shown_lyrics(self):
        if not (0 <= self.current_track_idx < len(self.playlist)):
            return
        text = self.current_lyrics_text()
        if not text.strip():
            QMessageBox.information(self, "Embed lyrics", "There are no lyrics to embed for this song.")
            return
        path = self.playlist[self.current_track_idx]
        if self.embed_lyrics_in_file(path, text):
            QMessageBox.information(self, "Embed lyrics", f"The lyrics are now inside “{os.path.basename(path)}”.")

    def embed_lyrics_in_file(self, path: str, text: str, parent=None) -> bool:
        """Asks for confirmation, then writes the lyrics into the file's tags.

        Windows won't let a file be replaced while it is playing, so the player lets go
        of it for a moment and then resumes at the same position.
        """
        parent = parent or self
        name = os.path.basename(path)
        answer = QMessageBox.question(
            parent, "Embed lyrics",
            f"Write these lyrics into “{name}”?\n\n"
            "They'll travel with the song when you share it. "
            "Playback pauses for a moment while the file is updated.",
        )
        if answer != QMessageBox.StandardButton.Yes:
            return False

        is_current = 0 <= self.current_track_idx < len(self.playlist) and self.playlist[self.current_track_idx] == path
        resume_ms = self.player.position() if is_current else 0
        was_playing = is_current and self.player.playbackState() == QMediaPlayer.PlaybackState.PlayingState
        if is_current:
            self.player.stop()
            self.player.setSource(QUrl())  # release the file
            QApplication.processEvents()

        error = ""
        QApplication.setOverrideCursor(Qt.CursorShape.WaitCursor)
        try:
            for _ in range(6):
                try:
                    write_embedded_lyrics(path, text)
                    error = ""
                    break
                except PermissionError as e:  # the file may take a moment to be released
                    error = str(e)
                    time.sleep(0.25)
                    QApplication.processEvents()
                except Exception as e:
                    error = str(e) or e.__class__.__name__
                    break
        finally:
            QApplication.restoreOverrideCursor()
            if is_current:
                self._pending_seek_ms = resume_ms if resume_ms > 0 else None
                self.player.setSource(QUrl.fromLocalFile(path))
                if was_playing:
                    self.player.play()
                else:
                    self.player.pause()

        if error:
            QMessageBox.warning(parent, "Embed lyrics", f"Couldn't update “{name}”:\n{error}")
            return False
        return True

    def _editor_open(self) -> bool:
        return any(d is not None and d.isVisible() for d in (self._sync_dialog, self._translate_dialog))

    def keep_worker(self, worker):
        """Keeps a background thread referenced until it ends (dropping a running QThread crashes Qt)."""
        self._workers.add(worker)
        worker.finished.connect(lambda w=worker: self._workers.discard(w))

    def ai_settings_raw(self) -> dict:
        s = self.settings
        return {
            "enabled": s.value("ai/enabled", False, type=bool),
            "provider": "openrouter" if s.value("ai/provider", "local", type=str) == "openrouter" else "local",
            "url": normalize_ai_url(s.value("ai/url", "", type=str)),
            "model": s.value("ai/model", "", type=str),
            "key": unprotect_secret(s.value("ai/key", "", type=str)),
            "or_model": s.value("ai/or_model", "", type=str),
            "or_key": unprotect_secret(s.value("ai/or_key", "", type=str)),
        }

    def ai_settings(self) -> dict:
        """The settings that apply now, for whichever provider is chosen."""
        raw = self.ai_settings_raw()
        if raw["provider"] == "openrouter":
            cfg = {"provider": "openrouter", "url": OPENROUTER_BASE, "model": raw["or_model"], "key": raw["or_key"],
                   "where": "OpenRouter", "headers": {"X-Title": APP_TITLE}}
            cfg["ready"] = raw["enabled"] and bool(cfg["key"] and cfg["model"])
        else:
            cfg = {"provider": "local", "url": raw["url"], "model": raw["model"], "key": raw["key"],
                   "where": raw["url"], "headers": {}}
            cfg["ready"] = raw["enabled"] and bool(cfg["url"])
        cfg["enabled"] = raw["enabled"]
        return cfg

    def save_ai_settings(self, enabled: bool, url: str = "", model: str = "", key: str = "",
                         provider: str = "local", or_model=None, or_key=None):
        s = self.settings
        s.setValue("ai/enabled", enabled)
        s.setValue("ai/provider", provider)
        s.setValue("ai/url", url)
        s.setValue("ai/model", model)
        s.setValue("ai/key", protect_secret(key))
        if or_model is not None:
            s.setValue("ai/or_model", or_model)
        if or_key is not None:
            s.setValue("ai/or_key", protect_secret(or_key))
        s.sync()

    def open_ai_settings(self):
        parent = self._translate_dialog if self._translate_dialog is not None and self._translate_dialog.isVisible() else self
        dlg = AiSettingsDialog(self)
        if parent is not self:
            dlg.setParent(parent, dlg.windowFlags())
        dlg.exec()
        if self._translate_dialog is not None:
            self._translate_dialog.update_ai_buttons()

    def open_translation_dialog(self):
        dlg = self._translate_dialog
        if dlg is not None and dlg.isVisible():
            dlg.raise_()
            dlg.activateWindow()
            return
        if not (0 <= self.current_track_idx < len(self.playlist)):
            return
        if not self.lyric_cards or not any(c.main_text and c.main_text != "♪" for c in self.lyric_cards):
            QMessageBox.information(self, "Add translation", "This song has no lyrics to add a translation to yet.")
            return
        if not self.is_synced:
            QMessageBox.information(
                self, "Add translation",
                "A translation is added line by line to synced lyrics, and this song's lyrics "
                "aren't synced yet.\n\nSync them first with Lyrics ▾ → Sync lyrics…")
            return
        path = self.playlist[self.current_track_idx]
        title = self.current_info.get("title") or os.path.basename(path)
        dlg = TranslationDialog(self, path, title)
        dlg.setAttribute(Qt.WidgetAttribute.WA_DeleteOnClose)
        dlg.destroyed.connect(lambda _=None: setattr(self, "_translate_dialog", None))
        self._translate_dialog = dlg
        dlg.show()

    def open_sync_dialog(self):
        if self._sync_dialog is not None and self._sync_dialog.isVisible():
            self._sync_dialog.raise_()
            self._sync_dialog.activateWindow()
            return
        if not (0 <= self.current_track_idx < len(self.playlist)):
            return
        path = self.playlist[self.current_track_idx]
        title = self.current_info.get("title") or os.path.basename(path)
        dlg = LyricsSyncDialog(self, path, title)
        dlg.setAttribute(Qt.WidgetAttribute.WA_DeleteOnClose)
        dlg.finished.connect(lambda _=None: setattr(self, "_sync_dialog", None))
        self._sync_dialog = dlg
        dlg.show()

    def show_details_dialog(self):
        if self.current_info:
            dlg = MetadataDialog(self, self.current_info, self.current_pixmap)
            dlg.exec()

    def open_lrc_dialog(self):
        file_path, _ = QFileDialog.getOpenFileName(self, "Select LRC File", "", "Lyrics Files (*.lrc *.txt)")
        if file_path:
            with open(file_path, "r", encoding="utf-8", errors="ignore") as f:
                self.parse_and_display_lyrics(f.read())

    def _clear_lyrics(self):
        self.scroll_anim.stop()
        while self.lyrics_layout.count() > 1:
            item = self.lyrics_layout.takeAt(0)
            w = item.widget()
            if w:
                w.deleteLater()
        self.lyric_cards = []
        self.current_card_index = -1
        self.is_synced = False
        self.scroll_area.verticalScrollBar().setValue(0)

    def _display_for(self, main_t: str, sub_t: str):
        """(main, sub) to show on a card.

        Romaji mode replaces Japanese lines with romaji; a translation line stays below.
        A paired line that is already the romaji of the main line is dropped, so LRCs
        that contain their own romaji lines don't show it twice.
        """
        if not self.romaji_enabled:
            return main_t, sub_t
        main_r = self.romaji.convert(main_t) or main_t
        sub_r = (self.romaji.convert(sub_t) or sub_t) if sub_t else ""
        if sub_r and _same_text(sub_r, main_r):
            sub_r = ""
        return main_r, sub_r

    def set_romaji_enabled(self, enabled: bool):
        if enabled:
            QApplication.setOverrideCursor(Qt.CursorShape.WaitCursor)
            try:
                ok = self.romaji.load()
            finally:
                QApplication.restoreOverrideCursor()
            if not ok:
                self.btn_romaji.blockSignals(True)
                self.btn_romaji.setChecked(False)
                self.btn_romaji.blockSignals(False)
                QMessageBox.warning(
                    self, "Romaji unavailable",
                    "Romaji conversion needs the 'cutlet' package and a UniDic dictionary.\n\n"
                    "Install with:\n    pip install cutlet unidic-lite\n\n"
                    f"Details: {self.romaji.error}")
                return
        self.romaji_enabled = enabled
        self.settings.setValue("lyrics/romaji", enabled)
        for card in self.lyric_cards:
            card.set_display(*self._display_for(card.main_text, card.sub_text))
        self._rescale_cards(self.isFullScreen())

    def parse_and_display_lyrics(self, content: str):
        self._clear_lyrics()
        tag_regex = re.compile(r"\[(\d{1,2}):(\d{1,2}(?:\.\d+)?)\]")
        raw_entries = []

        for raw_line in content.splitlines():
            line = raw_line.strip()
            if not line:
                continue

            matches = list(tag_regex.finditer(line))
            if matches:
                text = line[matches[-1].end():].strip()
                if not text:
                    text = "♪"
                for m in matches:
                    minutes = float(m.group(1))
                    seconds = float(m.group(2))
                    time_ms = int((minutes * 60 + seconds) * 1000)
                    raw_entries.append((time_ms, text))

        is_fs = self.isFullScreen()

        if not raw_entries:
            self.is_synced = False
            for line_text in [l.strip() for l in content.splitlines() if l.strip()]:
                card = LyricCard(0, line_text, "", self)
                card.set_display(*self._display_for(line_text, ""))
                card.update_style(is_active=False, is_fullscreen=is_fs, is_synced=False, animate=False)
                self.lyrics_layout.insertWidget(len(self.lyric_cards), card)
                self.lyric_cards.append(card)

            QTimer.singleShot(40, self._reset_scroll_and_sync)
            return

        self.is_synced = True
        raw_entries.sort(key=lambda x: x[0])

        grouped_lyrics = []
        i = 0
        n = len(raw_entries)
        while i < n:
            t1, text1 = raw_entries[i]
            if i + 1 < n and abs(raw_entries[i + 1][0] - t1) <= 150:
                text2 = raw_entries[i + 1][1]
                grouped_lyrics.append((t1, text1, text2))
                i += 2
            else:
                grouped_lyrics.append((t1, text1, ""))
                i += 1

        for time_ms, main_t, sub_t in grouped_lyrics:
            card = LyricCard(time_ms, main_t, sub_t, self)
            card.set_display(*self._display_for(main_t, sub_t))
            card.update_style(is_active=False, is_fullscreen=is_fs, is_synced=True, animate=False)
            self.lyrics_layout.insertWidget(len(self.lyric_cards), card)
            self.lyric_cards.append(card)

        QTimer.singleShot(40, self._reset_scroll_and_sync)

    def _reset_scroll_and_sync(self):
        """Scroll lyrics to the top, then jump to the line for the current position (resume / late lyrics)."""
        self.scroll_area.verticalScrollBar().setValue(0)
        if self.is_synced and self.lyric_cards and self.player.position() > 0:
            self._sync_lyrics_to_position(self.player.position())

    def _update_cover_art(self, pixmap):
        self.current_pixmap = pixmap
        size = 420 if self.isFullScreen() else 330
        if pixmap and not pixmap.isNull():
            scaled = pixmap.scaled(size, size, Qt.AspectRatioMode.KeepAspectRatioByExpanding, Qt.TransformationMode.SmoothTransformation)
            self.lbl_cover.setPixmap(scaled)
            self.lbl_cover.setText("")
        else:
            self.lbl_cover.clear()
            self.lbl_cover.setText("No Album Art")

    # -----------------------------------------------------
    # Fullscreen & Shortcuts
    # -----------------------------------------------------
    def toggle_fullscreen(self):
        if self.isFullScreen():
            self.showNormal()
            self.left_panel.setFixedWidth(330)
            self.lbl_cover.setFixedSize(330, 330)
            self._update_cover_art(self.current_pixmap)
            self._rescale_cards(fullscreen=False)
            if self._queue_was_visible:
                self.queue_panel.show()
        else:
            self._queue_was_visible = self.queue_panel.isVisible()
            self._windowed_geometry = self.saveGeometry()  # so we never persist fullscreen bounds
            self.queue_panel.hide()
            self.showFullScreen()
            self.left_panel.setFixedWidth(420)
            self.lbl_cover.setFixedSize(420, 420)
            self._update_cover_art(self.current_pixmap)
            self._rescale_cards(fullscreen=True)

    def _rescale_cards(self, fullscreen: bool):
        for i, card in enumerate(self.lyric_cards):
            card.update_style(
                is_active=(i == self.current_card_index),
                is_fullscreen=fullscreen,
                is_synced=self.is_synced,
                animate=False
            )

        if self.is_synced and self.current_card_index != -1 and self.current_card_index < len(self.lyric_cards):
            active_card = self.lyric_cards[self.current_card_index]
            QTimer.singleShot(25, lambda: self._smooth_scroll_to_card(active_card))

    def keyPressEvent(self, event):
        if event.key() == Qt.Key.Key_F11:
            self.toggle_fullscreen()
        elif event.key() == Qt.Key.Key_Escape and self.isFullScreen():
            self.toggle_fullscreen()
        elif event.key() == Qt.Key.Key_Space:
            self.toggle_play()
        elif event.modifiers() & Qt.KeyboardModifier.ControlModifier and event.key() == Qt.Key.Key_Right:
            self.play_next()
        elif event.modifiers() & Qt.KeyboardModifier.ControlModifier and event.key() == Qt.Key.Key_Left:
            self.play_previous()
        elif event.key() == Qt.Key.Key_BracketLeft:
            self.adjust_lyrics_offset(-100)
        elif event.key() == Qt.Key.Key_BracketRight:
            self.adjust_lyrics_offset(+100)
        elif event.key() == Qt.Key.Key_Backslash:
            self.set_lyrics_offset(0)
        elif event.key() == Qt.Key.Key_R and not event.modifiers():
            self.btn_romaji.toggle()
        elif event.key() == Qt.Key.Key_S and not event.modifiers():
            self.set_shuffle(not self.shuffle_on)
        elif event.key() == Qt.Key.Key_L and not event.modifiers():
            self.cycle_repeat()
        elif event.key() == Qt.Key.Key_M and not event.modifiers():
            self.toggle_mute()
        elif event.key() == Qt.Key.Key_V and not event.modifiers():
            self.cycle_visualizer()
        else:
            super().keyPressEvent(event)

    def show_shortcuts(self):
        box = QMessageBox(self)
        box.setWindowTitle("Keyboard shortcuts")
        box.setTextFormat(Qt.TextFormat.RichText)
        box.setText(SHORTCUTS_HTML)
        box.exec()

    # -----------------------------------------------------
    # Session memory (QSettings)
    # -----------------------------------------------------
    def _restore_session_ui(self):
        """Window geometry, volume, lyrics offset. Runs before the window is shown."""
        s = self.settings

        self.apply_theme(s.value("ui/theme", "dark", type=str), s.value("ui/accent", "coral", type=str), persist=False)

        volume = max(0, min(100, s.value("ui/volume", 80, type=int)))
        self.vol_slider.setValue(volume)  # slider signal updates the audio output

        self.set_lyrics_offset(s.value("lyrics/offset_ms", 0, type=int), persist=False)
        self.set_shuffle(s.value("playback/shuffle", False, type=bool), persist=False)
        self.set_repeat(s.value("playback/repeat", "off", type=str), persist=False)
        self.set_visualizer(s.value("ui/visualizer", "off", type=str), persist=False)

        geometry = s.value("window/geometry")
        if geometry:
            self.restoreGeometry(geometry)

        self._queue_wanted = s.value("ui/queue_open", False, type=bool)
        self.act_close_to_tray.setChecked(s.value("ui/close_to_tray", True, type=bool) and self.tray is not None)

        if s.value("lyrics/romaji", False, type=bool):
            # Deferred: loading the dictionary is slow, don't delay the first paint
            QTimer.singleShot(0, lambda: self.btn_romaji.setChecked(True))

    def _restore_playback_session(self):
        """Fullscreen, playlist, track and position. Runs right after the window appears."""
        s = self.settings

        if s.value("window/fullscreen", False, type=bool) and not self.isFullScreen():
            self.toggle_fullscreen()

        kind = s.value("playback/source_kind", "", type=str)
        source = s.value("playback/source_path", "", type=str)
        last_track = s.value("playback/last_track", "", type=str)
        last_pos = s.value("playback/last_position_ms", 0, type=int)

        files = []
        if kind == "folder" and source and os.path.isdir(source):
            files = self._scan_album_folder(source)
        elif kind == "file" and last_track and os.path.isfile(last_track):
            files = [last_track]
            source = last_track
        elif kind == "files":
            files = [f for f in (s.value("playback/files", [], type=list) or []) if os.path.isfile(f)]
        if not files:
            return

        self._set_playlist(files, kind, source)

        norm = lambda p: os.path.normcase(os.path.normpath(p))
        wanted = norm(last_track) if last_track else ""
        index = next((i for i, f in enumerate(files) if norm(f) == wanted), None)
        if index is None:
            index, last_pos = self._start_index(), 0  # remembered track is gone; start over
        elif self.shuffle_on:
            self._rebuild_order(first=index)

        # Restore the drawer (in fullscreen it is hidden, and re-shown on leaving fullscreen)
        if self.isFullScreen():
            self._queue_was_visible = self._queue_wanted
        else:
            self.queue_panel.setVisible(self._queue_wanted)

        # Restored paused: never start blasting audio on launch
        self.load_track_by_index(index, autoplay=False, start_position_ms=last_pos)

    def _resume_position_ms(self) -> int:
        if self._pending_seek_ms is not None:  # restore hasn't been applied yet; keep it
            return self._pending_seek_ms
        pos, dur = self.player.position(), self.player.duration()
        if dur > 0 and pos >= dur - 1500:
            return 0
        return max(0, pos)

    def _save_session(self, full: bool = True):
        if self._closing:
            return
        s = self.settings
        s.setValue("ui/volume", self.vol_slider.value())
        s.setValue("lyrics/offset_ms", self.lyrics_offset_ms)

        # Only overwrite the remembered playback if something is actually loaded, so a
        # failed restore (e.g. drive unplugged) doesn't erase it.
        if self.source_kind and 0 <= self.current_track_idx < len(self.playlist):
            s.setValue("playback/source_kind", self.source_kind)
            s.setValue("playback/source_path", self.source_path)
            if self.source_kind == "files":
                s.setValue("playback/files", self.playlist)
            s.setValue("playback/last_track", self.playlist[self.current_track_idx])
            s.setValue("playback/last_position_ms", self._resume_position_ms())

        if full:
            fullscreen = self.isFullScreen()
            geometry = self._windowed_geometry if fullscreen and self._windowed_geometry else self.saveGeometry()
            s.setValue("window/geometry", geometry)
            s.setValue("window/fullscreen", fullscreen)
            queue_open = (self._queue_was_visible or self.queue_panel.isVisible()) if fullscreen \
                else self.queue_panel.isVisible()
            s.setValue("ui/queue_open", queue_open)
        s.sync()

    def closeEvent(self, event):
        if self._close_to_tray():
            event.ignore()
            self._save_session(full=True)
            self.hide()
            self._update_tray()
            if not self.settings.value("ui/tray_hint_shown", False, type=bool):
                self.tray.showMessage(
                    "Still playing",
                    "The player keeps running in the tray. Right-click the icon to quit.",
                    QSystemTrayIcon.MessageIcon.Information, 5000)
                self.settings.setValue("ui/tray_hint_shown", True)
            return

        for dlg in (self._sync_dialog, self._translate_dialog):
            if dlg is not None and dlg.isVisible() and not dlg.close():
                event.ignore()  # unsaved edits: the user chose to keep editing
                return

        # Background threads must be finished before the app goes away
        for worker in list(self._workers):
            if hasattr(worker, "cancel"):
                worker.cancel()
        for worker in list(self._workers):
            worker.wait(2500)

        self._autosave_timer.stop()
        self._save_session(full=True)
        self._closing = True  # stop() below must not overwrite the snapshot we just saved
        self.media_controls.shutdown()
        self.player.stop()
        if self.tray is not None:
            self.tray.hide()
        super().closeEvent(event)
        QApplication.instance().quit()

    @staticmethod
    def _format_time(ms):
        time = QTime(0, 0, 0).addMSecs(ms)
        return time.toString("m:ss")


def resource_path(relative_path: str) -> str:
    base_path = getattr(sys, "_MEIPASS", os.path.abspath("."))
    return os.path.join(base_path, relative_path)

if __name__ == "__main__":
    # Force Windows to register an independent taskbar identity
    try:
        ctypes.windll.shell32.SetCurrentProcessExplicitAppUserModelID("myron.lrcplayer.app.2_3")
    except Exception:
        pass

    app = QApplication(sys.argv)
    # QSettings (session memory) is keyed on these; on Windows it lives in
    # HKCU\Software\LRCPlayer\MinimalistLRCPlayer. Kept from the app's old name on purpose:
    # changing it would make existing installs forget all their settings.
    app.setOrganizationName("LRCPlayer")
    app.setApplicationName("MinimalistLRCPlayer")
    
    # Set the icon application-wide using favicon.ico
    icon_file = resource_path("favicon.ico")
    if os.path.exists(icon_file):
        app_icon = QIcon(icon_file)
        app.setWindowIcon(app_icon)

    window = AudioLRCPlayer()
    if os.path.exists(icon_file):
        window.setWindowIcon(QIcon(icon_file))

    window.show()
    sys.exit(app.exec())