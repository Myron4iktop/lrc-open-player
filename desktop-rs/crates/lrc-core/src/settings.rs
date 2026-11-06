//! The player's remembered settings, in place of the Python app's `QSettings`.
//!
//! The Rust build keeps them in `%APPDATA%\LRCOpenPlayer\settings.json`, as one JSON
//! object whose keys are the Python app's `QSettings` keys (`ui/theme`,
//! `playback/last_track`, …), so the two are easy to compare.
//!
//! Reads are lenient, as `QSettings.value(key, default, type=…)` is: a number may come
//! as a string, a boolean as `"true"`, and a value of the wrong kind gives the default.
//!
//! On the very first launch the settings are copied once from the Python app's registry
//! key ([`import_qsettings`]); after that the two apps keep their own.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

/// The folder under `%APPDATA%` (or the config folder elsewhere).
pub const APP_DIR_NAME: &str = "LRCOpenPlayer";
/// The settings file in it.
pub const FILE_NAME: &str = "settings.json";

/// The Python app's registry key under `HKEY_CURRENT_USER` (QSettings with
/// organization `LRCPlayer` and application `MinimalistLRCPlayer`).
pub const PYTHON_REGISTRY_KEY: &str = r"Software\LRCPlayer\MinimalistLRCPlayer";

/// What kind of value a setting holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Bool,
    Int,
    Str,
    List,
}

/// The Python app's settings that the Rust build takes over, and their kinds.
/// `window/geometry` is left out: it is a Qt binary blob, and the Rust build keeps its
/// own window position instead.
pub const IMPORTED_KEYS: [(&str, Kind); 24] = [
    ("ai/enabled", Kind::Bool),
    ("ai/key", Kind::Str),
    ("ai/model", Kind::Str),
    ("ai/or_key", Kind::Str),
    ("ai/or_model", Kind::Str),
    ("ai/provider", Kind::Str),
    ("ai/url", Kind::Str),
    ("lyrics/offset_ms", Kind::Int),
    ("lyrics/romaji", Kind::Bool),
    ("playback/files", Kind::List),
    ("playback/last_position_ms", Kind::Int),
    ("playback/last_track", Kind::Str),
    ("playback/repeat", Kind::Str),
    ("playback/shuffle", Kind::Bool),
    ("playback/source_kind", Kind::Str),
    ("playback/source_path", Kind::Str),
    ("ui/accent", Kind::Str),
    ("ui/close_to_tray", Kind::Bool),
    ("ui/queue_open", Kind::Bool),
    ("ui/theme", Kind::Str),
    ("ui/tray_hint_shown", Kind::Bool),
    ("ui/visualizer", Kind::Str),
    ("ui/volume", Kind::Int),
    ("window/fullscreen", Kind::Bool),
];

/// Where the settings file lives: `%APPDATA%\LRCOpenPlayer\settings.json` on Windows,
/// `$XDG_CONFIG_HOME/LRCOpenPlayer/settings.json` (or `~/.config/…`) elsewhere.
pub fn default_path() -> Option<PathBuf> {
    let base = if cfg!(windows) {
        std::env::var_os("APPDATA").map(PathBuf::from)
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
    }?;
    Some(base.join(APP_DIR_NAME).join(FILE_NAME))
}

/// The remembered settings.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Settings {
    values: Map<String, Value>,
}

/// How [`Settings::load`] went.
#[derive(Debug)]
pub enum Loaded {
    /// The file was read.
    Read(Settings),
    /// There is no settings file yet: this is the first launch.
    Missing,
    /// The file is there but isn't valid. It was renamed to `settings.json.broken`, so
    /// saving can't erase what it held, and the player starts with defaults.
    Broken(Settings),
}

impl Settings {
    pub fn new() -> Self {
        Self::default()
    }

    /// Reads the settings file.
    pub fn load(path: &Path) -> io::Result<Loaded> {
        let text = match fs::read(path) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Loaded::Missing),
            Err(e) => return Err(e),
        };
        match serde_json::from_slice::<Value>(&text) {
            Ok(Value::Object(values)) => Ok(Loaded::Read(Settings { values })),
            _ => {
                let mut broken = path.as_os_str().to_owned();
                broken.push(".broken");
                let _ = fs::rename(path, &broken);
                Ok(Loaded::Broken(Settings::new()))
            }
        }
    }

    /// Writes the settings file. It is written next to the old one and then swapped in,
    /// so a crash or a full disk mid-write can't leave half a file behind.
    pub fn save(&self, path: &Path) -> io::Result<()> {
        let dir = path
            .parent()
            .filter(|d| !d.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        fs::create_dir_all(dir)?;
        let mut text = serde_json::to_string_pretty(&Value::Object(self.values.clone()))
            .map_err(io::Error::other)?;
        text.push('\n');
        let mut tmp = tempfile::Builder::new()
            .prefix(".settings-")
            .suffix(".tmp")
            .tempfile_in(dir)?;
        tmp.write_all(text.as_bytes())?;
        tmp.as_file().sync_all()?;
        tmp.persist(path).map_err(|e| e.error)?;
        Ok(())
    }

    /// The raw value, if there is one.
    pub fn raw(&self, key: &str) -> Option<&Value> {
        self.values.get(key)
    }

    pub fn contains(&self, key: &str) -> bool {
        self.values.contains_key(key)
    }

    pub fn keys(&self) -> impl Iterator<Item = &str> {
        self.values.keys().map(String::as_str)
    }

    /// Sets a value. Returns true when it changed.
    pub fn set(&mut self, key: &str, value: impl Into<Value>) -> bool {
        let value = value.into();
        if self.values.get(key) == Some(&value) {
            return false;
        }
        self.values.insert(key.to_owned(), value);
        true
    }

    pub fn remove(&mut self, key: &str) -> bool {
        self.values.remove(key).is_some()
    }

    /// `value(key, default, type=str)`.
    pub fn get_str(&self, key: &str, default: &str) -> String {
        self.raw(key)
            .and_then(to_str)
            .unwrap_or_else(|| default.to_owned())
    }

    /// `value(key, default, type=int)`.
    pub fn get_int(&self, key: &str, default: i64) -> i64 {
        self.raw(key).and_then(to_int).unwrap_or(default)
    }

    /// `value(key, default, type=bool)`.
    pub fn get_bool(&self, key: &str, default: bool) -> bool {
        self.raw(key).and_then(to_bool).unwrap_or(default)
    }

    /// `value(key, [], type=list)`: a list of strings. A single string is a list of one,
    /// as QSettings reads a one-item list back.
    pub fn get_list(&self, key: &str) -> Vec<String> {
        self.raw(key).and_then(to_list).unwrap_or_default()
    }
}

fn to_str(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(if *b { "true" } else { "false" }.to_owned()),
        _ => None,
    }
}

fn to_int(v: &Value) -> Option<i64> {
    match v {
        Value::Number(n) => n.as_i64().or_else(|| {
            n.as_f64()
                .filter(|f| f.is_finite())
                .map(|f| f.trunc() as i64)
        }),
        Value::String(s) => parse_int(s),
        Value::Bool(b) => Some(i64::from(*b)),
        _ => None,
    }
}

/// QVariant's string-to-int: an optional sign and digits, spaces around allowed.
fn parse_int(s: &str) -> Option<i64> {
    let s = s.trim();
    s.parse::<i64>().ok().or_else(|| {
        // QSettings may hand back "1.5e3"-style text for a double; take its whole part.
        s.parse::<f64>()
            .ok()
            .filter(|f| f.is_finite())
            .map(|f| f.trunc() as i64)
    })
}

/// QVariant's bool conversion: "false", "0" and "" are false, other text is true.
fn to_bool(v: &Value) -> Option<bool> {
    match v {
        Value::Bool(b) => Some(*b),
        Value::Number(n) => Some(n.as_f64().is_some_and(|f| f != 0.0)),
        Value::String(s) => {
            let t = s.trim().to_ascii_lowercase();
            Some(!(t.is_empty() || t == "false" || t == "0"))
        }
        _ => None,
    }
}

fn to_list(v: &Value) -> Option<Vec<String>> {
    match v {
        Value::Array(items) => Some(items.iter().filter_map(to_str).collect()),
        Value::String(s) if s.is_empty() => Some(Vec::new()),
        Value::String(s) => Some(vec![s.clone()]),
        _ => None,
    }
}

// ---------------------------------------------------------------------------------------
// The one-time import from the Python app
// ---------------------------------------------------------------------------------------

/// A value as the Windows registry holds it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegValue {
    /// `REG_SZ` or `REG_EXPAND_SZ`.
    Sz(String),
    /// `REG_MULTI_SZ`.
    MultiSz(Vec<String>),
    /// `REG_DWORD`.
    Dword(u32),
    /// `REG_QWORD`.
    Qword(u64),
    /// `REG_BINARY` and anything else.
    Binary(Vec<u8>),
}

/// QSettings writes some values as text with a type marker: `@Invalid()` for a missing
/// value (an empty list, for one), `@ByteArray(…)`, `@Variant(…)`, and a leading `@`
/// doubled. Returns the plain text, or `None` for a marked value that isn't text.
fn qsettings_text(s: &str) -> Option<String> {
    if let Some(rest) = s.strip_prefix("@@") {
        return Some(format!("@{rest}"));
    }
    if s.starts_with('@') {
        return None;
    }
    Some(s.to_owned())
}

/// Turns one registry value into the JSON value for a setting of `kind`, or `None`
/// when it can't be read as that.
fn convert(value: &RegValue, kind: Kind) -> Option<Value> {
    match (kind, value) {
        (Kind::Str, RegValue::Sz(s)) => qsettings_text(s).map(Value::from),
        (Kind::Str, RegValue::MultiSz(items)) => items.first().cloned().map(Value::from),
        (Kind::Str, RegValue::Dword(n)) => Some(n.to_string().into()),
        (Kind::Str, RegValue::Qword(n)) => Some(n.to_string().into()),

        // Qt keeps an int in a DWORD as its 32 bits; a negative offset reads back so.
        (Kind::Int, RegValue::Dword(n)) => Some(i64::from(*n as i32).into()),
        (Kind::Int, RegValue::Qword(n)) => Some((*n as i64).into()),
        (Kind::Int, RegValue::Sz(s)) => qsettings_text(s)
            .and_then(|t| parse_int(&t))
            .map(Value::from),

        (Kind::Bool, RegValue::Dword(n)) => Some((*n != 0).into()),
        (Kind::Bool, RegValue::Qword(n)) => Some((*n != 0).into()),
        (Kind::Bool, RegValue::Sz(s)) => qsettings_text(s)
            .and_then(|t| to_bool(&Value::String(t)))
            .map(Value::from),

        (Kind::List, RegValue::MultiSz(items)) => Some(
            items
                .iter()
                .filter_map(|s| qsettings_text(s))
                .filter(|s| !s.is_empty())
                .map(Value::from)
                .collect::<Vec<_>>()
                .into(),
        ),
        (Kind::List, RegValue::Sz(s)) => {
            if s == "@Invalid()" || s.is_empty() {
                Some(Value::Array(Vec::new()))
            } else {
                qsettings_text(s).map(|t| vec![Value::from(t)].into())
            }
        }
        _ => None,
    }
}

/// Builds the Rust build's first settings from the Python app's registry values.
///
/// `values` are `(name, value)` pairs, the name being the QSettings key with `/`
/// between the groups (the registry's subkey path), such as `ui/theme`. Names are
/// matched without regard to case, as the registry does. Keys the Rust build doesn't
/// know, `window/geometry`, and values that can't be read are left out.
pub fn import_qsettings<'a>(values: impl IntoIterator<Item = (&'a str, &'a RegValue)>) -> Settings {
    let mut settings = Settings::new();
    for (name, value) in values {
        let name = name.replace('\\', "/");
        let Some(&(key, kind)) = IMPORTED_KEYS
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(&name))
        else {
            continue;
        };
        if let Some(v) = convert(value, kind) {
            settings.set(key, v);
        }
    }
    settings
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn lenient_reads() {
        let mut s = Settings::new();
        s.set("a", "12");
        s.set("b", 7);
        s.set("c", "true");
        s.set("d", "false");
        s.set("e", json!(["x", "y"]));
        s.set("f", "one");
        s.set("g", json!({"no": 1}));
        s.set("h", -250);
        assert_eq!(s.get_int("a", 0), 12);
        assert_eq!(s.get_int("b", 0), 7);
        assert_eq!(s.get_int("c", 3), 3); // "true" is not a number: the default
        assert_eq!(s.get_int("h", 0), -250);
        assert!(s.get_bool("c", false));
        assert!(!s.get_bool("d", true));
        assert!(s.get_bool("b", false));
        assert_eq!(s.get_str("b", ""), "7");
        assert_eq!(s.get_list("e"), ["x", "y"]);
        assert_eq!(s.get_list("f"), ["one"]);
        assert_eq!(s.get_list("missing"), Vec::<String>::new());
        assert_eq!(s.get_str("g", "dflt"), "dflt");
        assert_eq!(s.get_int("missing", 80), 80);
        assert!(s.get_bool("missing", true));
    }

    #[test]
    fn set_reports_changes() {
        let mut s = Settings::new();
        assert!(s.set("ui/volume", 80));
        assert!(!s.set("ui/volume", 80));
        assert!(s.set("ui/volume", 81));
    }

    #[test]
    fn save_and_load_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join(FILE_NAME);
        assert!(matches!(Settings::load(&path).unwrap(), Loaded::Missing));
        let mut s = Settings::new();
        s.set("ui/theme", "light");
        s.set("playback/files", json!(["C:\\a.flac", "C:\\b.mp3"]));
        s.set("lyrics/offset_ms", -300);
        s.save(&path).unwrap();
        let Loaded::Read(back) = Settings::load(&path).unwrap() else {
            panic!("not read back")
        };
        assert_eq!(back, s);
        // Saving again replaces the file.
        s.set("ui/theme", "dark");
        s.save(&path).unwrap();
        let Loaded::Read(back) = Settings::load(&path).unwrap() else {
            panic!()
        };
        assert_eq!(back.get_str("ui/theme", ""), "dark");
        // No temporary files left behind.
        let names: Vec<_> = fs::read_dir(path.parent().unwrap())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, [FILE_NAME]);
    }

    #[test]
    fn a_broken_file_is_kept_aside() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE_NAME);
        fs::write(&path, "{ not json").unwrap();
        assert!(matches!(Settings::load(&path).unwrap(), Loaded::Broken(_)));
        assert!(!path.exists());
        assert_eq!(
            fs::read_to_string(dir.path().join("settings.json.broken")).unwrap(),
            "{ not json"
        );
    }

    #[test]
    fn imports_what_qsettings_wrote() {
        let values = vec![
            ("ui/theme".to_owned(), RegValue::Sz("light".into())),
            ("ui/accent".to_owned(), RegValue::Sz("jade".into())),
            ("ui/volume".to_owned(), RegValue::Dword(65)),
            (
                "lyrics/offset_ms".to_owned(),
                RegValue::Dword((-400i32) as u32),
            ),
            (
                "playback/last_position_ms".to_owned(),
                RegValue::Qword(123_456),
            ),
            ("playback/shuffle".to_owned(), RegValue::Sz("true".into())),
            ("playback/repeat".to_owned(), RegValue::Sz("album".into())),
            ("ui/close_to_tray".to_owned(), RegValue::Sz("false".into())),
            (
                "playback/files".to_owned(),
                RegValue::MultiSz(vec!["D:\\a.flac".into(), "D:\\b.opus".into()]),
            ),
            (
                "playback/source_kind".to_owned(),
                RegValue::Sz("files".into()),
            ),
            (
                "Playback\\Source_Path".to_owned(),
                RegValue::Sz("D:\\".into()),
            ),
            (
                "ai/key".to_owned(),
                RegValue::Sz("dpapi:AQAAANCMnd8BFdERjHoAwE".into()),
            ),
            (
                "window/geometry".to_owned(),
                RegValue::Binary(vec![1, 2, 3]),
            ),
            ("window/fullscreen".to_owned(), RegValue::Sz("false".into())),
            ("something/else".to_owned(), RegValue::Sz("x".into())),
            (
                "ui/visualizer".to_owned(),
                RegValue::Sz("@Invalid()".into()),
            ),
            ("ai/url".to_owned(), RegValue::Sz("@@odd".into())),
        ];
        let s = import_qsettings(values.iter().map(|(k, v)| (k.as_str(), v)));
        assert_eq!(s.get_str("ui/theme", ""), "light");
        assert_eq!(s.get_str("ui/accent", ""), "jade");
        assert_eq!(s.raw("ui/volume"), Some(&json!(65)));
        assert_eq!(s.raw("lyrics/offset_ms"), Some(&json!(-400)));
        assert_eq!(s.raw("playback/last_position_ms"), Some(&json!(123_456)));
        assert_eq!(s.raw("playback/shuffle"), Some(&json!(true)));
        assert_eq!(s.raw("ui/close_to_tray"), Some(&json!(false)));
        assert_eq!(s.get_str("playback/repeat", ""), "album");
        assert_eq!(s.get_list("playback/files"), ["D:\\a.flac", "D:\\b.opus"]);
        assert_eq!(s.get_str("playback/source_path", ""), "D:\\");
        // Keys are copied as they are: still DPAPI-protected for the same Windows user.
        assert_eq!(s.get_str("ai/key", ""), "dpapi:AQAAANCMnd8BFdERjHoAwE");
        assert_eq!(s.get_str("ai/url", ""), "@odd");
        assert!(!s.contains("window/geometry"));
        assert!(!s.contains("something/else"));
        assert!(!s.contains("ui/visualizer"));
        assert_eq!(s.raw("window/fullscreen"), Some(&json!(false)));
    }

    #[test]
    fn imports_lists_written_as_one_string() {
        let one = RegValue::Sz("D:\\only.flac".into());
        let none = RegValue::Sz("@Invalid()".into());
        let s = import_qsettings([("playback/files", &one)]);
        assert_eq!(s.get_list("playback/files"), ["D:\\only.flac"]);
        let s = import_qsettings([("playback/files", &none)]);
        assert_eq!(s.raw("playback/files"), Some(&json!([])));
    }

    #[test]
    fn every_python_key_but_geometry_is_imported() {
        assert!(IMPORTED_KEYS.iter().all(|(k, _)| *k != "window/geometry"));
        let mut keys: Vec<_> = IMPORTED_KEYS.iter().map(|(k, _)| *k).collect();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), 24);
    }
}
