//! The settings file on disk, and the one-time import from the Python app.
//!
//! The rules (what is saved, what is restored, how registry values are read) are in
//! `lrc_core::settings` and `lrc_core::session`; this file finds the file, reads the
//! Python app's registry key on first launch, and writes changes back.

use std::path::PathBuf;

use lrc_core::settings::{self, Loaded, Settings};
use serde_json::Value;

/// View → Smooth animation: draw with the GPU (Slint's FemtoVG renderer) instead of the
/// software renderer. Read once at start, before the window exists.
pub const SMOOTH_ANIMATION_KEY: &str = "ui/smooth_animation";
pub const SMOOTH_ANIMATION_TITLE: &str = "Smooth animation";

/// Whether the saved settings ask for GPU drawing. Only reads; never writes the file.
pub fn smooth_animation_wanted() -> bool {
    settings::default_path()
        .and_then(|path| Settings::peek(&path))
        .is_some_and(|s| s.get_bool(SMOOTH_ANIMATION_KEY, false))
}

/// The settings and where they are kept.
pub struct Store {
    /// `None` when the file couldn't be read: then nothing is written, so whatever it
    /// holds isn't overwritten with defaults.
    path: Option<PathBuf>,
    pub settings: Settings,
}

impl Store {
    /// Reads `%APPDATA%\LRCOpenPlayer\settings.json`. On the very first launch (no file
    /// yet) the Python app's settings are taken over, and the file is written at once,
    /// so they are imported only this one time.
    pub fn open() -> Self {
        let Some(path) = settings::default_path() else {
            return Store {
                path: None,
                settings: Settings::new(),
            };
        };
        match Settings::load(&path) {
            Ok(Loaded::Read(settings)) => Store {
                path: Some(path),
                settings,
            },
            Ok(Loaded::Broken(settings)) => {
                eprintln!(
                    "{} was not valid and was kept as settings.json.broken",
                    path.display()
                );
                Store {
                    path: Some(path),
                    settings,
                }
            }
            Ok(Loaded::Missing) => {
                let store = Store {
                    path: Some(path),
                    settings: import_from_python_app(),
                };
                store.save();
                store
            }
            Err(e) => {
                eprintln!("Could not read {}: {e}", path.display());
                Store {
                    path: None,
                    settings: Settings::new(),
                }
            }
        }
    }

    /// Writes the file now (`QSettings.sync`).
    pub fn save(&self) {
        if let Some(path) = &self.path
            && let Err(e) = self.settings.save(path)
        {
            eprintln!("Could not save {}: {e}", path.display());
        }
    }

    /// Changes one setting and writes the file if it changed (`QSettings.setValue`).
    pub fn set(&mut self, key: &str, value: impl Into<Value>) {
        if self.settings.set(key, value) {
            self.save();
        }
    }
}

/// The Python app's settings, read from its registry key. Empty when it never ran.
#[cfg(windows)]
fn import_from_python_app() -> Settings {
    let values = registry::read_tree(settings::PYTHON_REGISTRY_KEY);
    settings::import_qsettings(values.iter().map(|(k, v)| (k.as_str(), v)))
}

#[cfg(not(windows))]
fn import_from_python_app() -> Settings {
    Settings::new()
}

#[cfg(windows)]
mod registry {
    //! Reading a registry key with its subkeys, as QSettings lays out groups.

    use lrc_core::settings::RegValue;
    use windows::Win32::Foundation::{ERROR_MORE_DATA, ERROR_SUCCESS};
    use windows::Win32::System::Registry::{
        HKEY, HKEY_CURRENT_USER, KEY_READ, REG_BINARY, REG_DWORD, REG_EXPAND_SZ, REG_MULTI_SZ,
        REG_QWORD, REG_SZ, RegCloseKey, RegEnumKeyExW, RegEnumValueW, RegOpenKeyExW,
    };
    use windows::core::{PCWSTR, PWSTR};

    /// An open key, closed when dropped.
    struct Key(HKEY);

    impl Drop for Key {
        fn drop(&mut self) {
            // SAFETY: the handle came from RegOpenKeyExW and is closed only here.
            let _ = unsafe { RegCloseKey(self.0) };
        }
    }

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(Some(0)).collect()
    }

    fn open(parent: HKEY, path: &str) -> Option<Key> {
        let name = wide(path);
        let mut key = HKEY::default();
        // SAFETY: `name` is NUL-terminated and outlives the call; `key` is written on success.
        let status =
            unsafe { RegOpenKeyExW(parent, PCWSTR(name.as_ptr()), Some(0), KEY_READ, &mut key) };
        (status == ERROR_SUCCESS).then_some(Key(key))
    }

    /// Every value under `HKEY_CURRENT_USER\<path>`, its subkeys included, named
    /// `group/key` the way QSettings names them.
    pub fn read_tree(path: &str) -> Vec<(String, RegValue)> {
        let mut out = Vec::new();
        if let Some(key) = open(HKEY_CURRENT_USER, path) {
            read_key(&key, "", &mut out, 0);
        }
        out
    }

    fn read_key(key: &Key, prefix: &str, out: &mut Vec<(String, RegValue)>, depth: u32) {
        for (name, value) in values(key) {
            out.push((format!("{prefix}{name}"), value));
        }
        if depth >= 4 {
            return;
        }
        for sub in subkeys(key) {
            if let Some(child) = open(key.0, &sub) {
                read_key(&child, &format!("{prefix}{sub}/"), out, depth + 1);
            }
        }
    }

    fn subkeys(key: &Key) -> Vec<String> {
        let mut names = Vec::new();
        let mut buf = vec![0u16; 256];
        for index in 0.. {
            let mut len = buf.len() as u32;
            // SAFETY: `buf` holds `len` UTF-16 units; the other outputs are unused.
            let status = unsafe {
                RegEnumKeyExW(
                    key.0,
                    index,
                    Some(PWSTR(buf.as_mut_ptr())),
                    &mut len,
                    None,
                    None,
                    None,
                    None,
                )
            };
            if status != ERROR_SUCCESS {
                break;
            }
            names.push(String::from_utf16_lossy(&buf[..len as usize]));
        }
        names
    }

    fn values(key: &Key) -> Vec<(String, RegValue)> {
        let mut out = Vec::new();
        // Value names are at most 16,383 characters.
        let mut name = vec![0u16; 16_384];
        let mut data = vec![0u8; 4096];
        let mut index = 0;
        loop {
            let mut name_len = name.len() as u32;
            let mut data_len = data.len() as u32;
            let mut kind = 0u32;
            // SAFETY: both buffers are as long as the lengths passed with them.
            let status = unsafe {
                RegEnumValueW(
                    key.0,
                    index,
                    Some(PWSTR(name.as_mut_ptr())),
                    &mut name_len,
                    None,
                    Some(&mut kind),
                    Some(data.as_mut_ptr()),
                    Some(&mut data_len),
                )
            };
            if status == ERROR_MORE_DATA {
                // The data is bigger than the buffer: grow it and read this value again.
                data.resize((data_len as usize).max(data.len() * 2), 0);
                continue;
            }
            if status != ERROR_SUCCESS {
                break;
            }
            let value_name = String::from_utf16_lossy(&name[..name_len as usize]);
            out.push((value_name, to_value(kind, &data[..data_len as usize])));
            index += 1;
        }
        out
    }

    fn utf16(bytes: &[u8]) -> Vec<u16> {
        bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|&c| u16::from_le_bytes(c))
            .collect()
    }

    fn to_value(kind: u32, bytes: &[u8]) -> RegValue {
        match kind {
            k if k == REG_SZ.0 || k == REG_EXPAND_SZ.0 => {
                let units = utf16(bytes);
                let end = units.iter().position(|&u| u == 0).unwrap_or(units.len());
                RegValue::Sz(String::from_utf16_lossy(&units[..end]))
            }
            k if k == REG_MULTI_SZ.0 => {
                let units = utf16(bytes);
                let items = units
                    .split(|&u| u == 0)
                    .filter(|s| !s.is_empty())
                    .map(String::from_utf16_lossy)
                    .collect();
                RegValue::MultiSz(items)
            }
            k if k == REG_DWORD.0 && bytes.len() >= 4 => {
                RegValue::Dword(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
            }
            k if k == REG_QWORD.0 && bytes.len() >= 8 => {
                let mut b = [0u8; 8];
                b.copy_from_slice(&bytes[..8]);
                RegValue::Qword(u64::from_le_bytes(b))
            }
            k if k == REG_BINARY.0 => RegValue::Binary(bytes.to_vec()),
            _ => RegValue::Binary(bytes.to_vec()),
        }
    }
}
