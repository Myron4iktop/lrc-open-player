//! Reading lyrics files in whatever encoding they were saved in
//! (`decode_lyrics_bytes`, `read_lyrics_file` and `file_error_text` in `lrcplayer.py`).

use std::fmt;
use std::io::{self, Read};
use std::path::Path;
use std::sync::atomic::{AtomicU16, Ordering};

use encoding_rs::Encoding;

use crate::codepages;

/// Far more than any song's lyrics (`LYRICS_FILE_LIMIT`).
pub const LYRICS_FILE_LIMIT: u64 = 4 * 1024 * 1024;

static ANSI_CODE_PAGE: AtomicU16 = AtomicU16::new(1252);

/// Tells the decoder which ANSI code page this PC uses (`GetACP()` on Windows).
///
/// The Python app asks `locale.getpreferredencoding()`, which is that code page. The core
/// has no Windows APIs, so the player calls this once at start-up. Until then, and on
/// other systems, it is 1252 (Western European), the same fallback the Python app uses.
pub fn set_ansi_code_page(code_page: u16) {
    ANSI_CODE_PAGE.store(code_page, Ordering::Relaxed);
}

/// The code page set with [`set_ansi_code_page`].
pub fn ansi_code_page() -> u16 {
    ANSI_CODE_PAGE.load(Ordering::Relaxed)
}

/// The text of a lyrics file, whatever encoding it was saved in.
///
/// UTF-8 (with or without a BOM) and UTF-16/32 (Notepad's "Unicode") are recognised by
/// their bytes. Older files use a legacy code page: Shift-JIS for most Japanese lyrics,
/// otherwise usually this PC's ANSI code page (Windows-1252 in Western Europe, GBK in
/// China...).
pub fn decode_lyrics_bytes(data: &[u8]) -> String {
    decode_lyrics_bytes_with(data, ansi_code_page())
}

/// [`decode_lyrics_bytes`] with an explicit ANSI code page.
pub fn decode_lyrics_bytes_with(data: &[u8], ansi_code_page: u16) -> String {
    if let Some(rest) = data.strip_prefix(b"\xef\xbb\xbf") {
        return String::from_utf8_lossy(rest).into_owned();
    }
    if let Some(rest) = data.strip_prefix(b"\xff\xfe\x00\x00") {
        return decode_utf32(rest, false);
    }
    if let Some(rest) = data.strip_prefix(b"\x00\x00\xfe\xff") {
        return decode_utf32(rest, true);
    }
    if let Some(rest) = data.strip_prefix(b"\xff\xfe") {
        return encoding_rs::UTF_16LE
            .decode_without_bom_handling(rest)
            .0
            .into_owned();
    }
    if let Some(rest) = data.strip_prefix(b"\xfe\xff") {
        return encoding_rs::UTF_16BE
            .decode_without_bom_handling(rest)
            .0
            .into_owned();
    }
    if let Ok(text) = std::str::from_utf8(data) {
        return text.to_owned();
    }
    if let Some(text) = decode_strict(data, 932)
        && text.chars().any(|c| ('\u{3040}'..='\u{30ff}').contains(&c))
    {
        // Real Japanese has kana; other code pages rarely decode to it.
        return text;
    }
    if let Some(text) = decode_strict(data, ansi_code_page) {
        return text;
    }
    decode_single_byte(data, &codepages::CP1252, true).unwrap_or_default()
}

/// Decodes with a Windows code page, or `None` where Python's codec would raise an error.
fn decode_strict(data: &[u8], code_page: u16) -> Option<String> {
    let table = match code_page {
        874 => &codepages::CP874,
        1250 => &codepages::CP1250,
        1251 => &codepages::CP1251,
        1252 => &codepages::CP1252,
        1253 => &codepages::CP1253,
        1254 => &codepages::CP1254,
        1255 => &codepages::CP1255,
        1256 => &codepages::CP1256,
        1257 => &codepages::CP1257,
        1258 => &codepages::CP1258,
        932 => return decode_double_byte(data, &SHIFT_JIS),
        936 => return decode_double_byte(data, &GBK),
        949 => return decode_double_byte(data, &UHC),
        950 => return decode_double_byte(data, &BIG5),
        // Code pages Python has no codec for (or UTF-8, which already failed).
        _ => return None,
    };
    decode_single_byte(data, table, false)
}

/// A double-byte code page: encoding_rs for the characters, Python's tables for which
/// bytes are allowed.
struct DoubleByte {
    encoding: &'static Encoding,
    single: &'static [u16; 128],
    lead: &'static [(u8, u8)],
    reject: &'static [(u16, u16)],
    overrides: &'static [(u16, u16)],
}

const SHIFT_JIS: DoubleByte = DoubleByte {
    encoding: encoding_rs::SHIFT_JIS,
    single: &codepages::CP932_SINGLE,
    lead: codepages::CP932_LEAD,
    reject: codepages::CP932_REJECT,
    overrides: codepages::CP932_OVERRIDE,
};
const GBK: DoubleByte = DoubleByte {
    encoding: encoding_rs::GBK,
    single: &codepages::CP936_SINGLE,
    lead: codepages::CP936_LEAD,
    reject: codepages::CP936_REJECT,
    overrides: codepages::CP936_OVERRIDE,
};
const UHC: DoubleByte = DoubleByte {
    encoding: encoding_rs::EUC_KR,
    single: &codepages::CP949_SINGLE,
    lead: codepages::CP949_LEAD,
    reject: codepages::CP949_REJECT,
    overrides: codepages::CP949_OVERRIDE,
};
const BIG5: DoubleByte = DoubleByte {
    encoding: encoding_rs::BIG5,
    single: &codepages::CP950_SINGLE,
    lead: codepages::CP950_LEAD,
    reject: codepages::CP950_REJECT,
    overrides: codepages::CP950_OVERRIDE,
};

fn decode_double_byte(data: &[u8], cp: &DoubleByte) -> Option<String> {
    let mut out = String::with_capacity(data.len());
    let mut i = 0;
    while i < data.len() {
        let b = data[i];
        if b < 0x80 {
            out.push(b as char);
            i += 1;
            continue;
        }
        let single = cp.single[usize::from(b - 0x80)];
        if single != 0 {
            out.push(char::from_u32(u32::from(single))?);
            i += 1;
            continue;
        }
        if !cp.lead.iter().any(|&(lo, hi)| (lo..=hi).contains(&b)) {
            return None;
        }
        let trail = *data.get(i + 1)?;
        let code = u16::from_be_bytes([b, trail]);
        if cp.reject.iter().any(|&(lo, hi)| (lo..=hi).contains(&code)) {
            return None;
        }
        if let Ok(k) = cp.overrides.binary_search_by_key(&code, |&(c, _)| c) {
            out.push(char::from_u32(u32::from(cp.overrides[k].1))?);
        } else {
            let pair = cp
                .encoding
                .decode_without_bom_handling_and_without_replacement(&data[i..i + 2])?;
            let mut chars = pair.chars();
            match (chars.next(), chars.next()) {
                (Some(c), None) => out.push(c),
                _ => return None,
            }
        }
        i += 2;
    }
    Some(out)
}

fn decode_single_byte(data: &[u8], upper: &[u16; 128], replace: bool) -> Option<String> {
    let mut out = String::with_capacity(data.len());
    for &b in data {
        if b < 0x80 {
            out.push(b as char);
            continue;
        }
        match upper[usize::from(b - 0x80)] {
            0 if replace => out.push('\u{fffd}'),
            0 => return None,
            c => out.push(char::from_u32(u32::from(c)).unwrap_or('\u{fffd}')),
        }
    }
    Some(out)
}

/// UTF-32 with replacement characters, as Python's `errors="replace"` gives.
fn decode_utf32(data: &[u8], big_endian: bool) -> String {
    let mut out = String::with_capacity(data.len() / 4);
    let mut chunks = data.chunks_exact(4);
    for chunk in &mut chunks {
        let bytes = [chunk[0], chunk[1], chunk[2], chunk[3]];
        let value = if big_endian {
            u32::from_be_bytes(bytes)
        } else {
            u32::from_le_bytes(bytes)
        };
        out.push(char::from_u32(value).unwrap_or('\u{fffd}'));
    }
    if !chunks.remainder().is_empty() {
        out.push('\u{fffd}');
    }
    out
}

/// Why a lyrics file couldn't be read. Its `Display` is the message the Python app shows.
#[derive(Debug)]
pub enum LyricsFileError {
    /// Over [`LYRICS_FILE_LIMIT`].
    TooBig { name: String },
    /// The file couldn't be opened or read.
    Io { name: String, error: io::Error },
}

impl fmt::Display for LyricsFileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooBig { name } => write!(f, "“{name}” is too big to be a lyrics file."),
            Self::Io { name, error } => match strerror(error) {
                Some(reason) => write!(f, "Couldn't read “{name}”: {reason}."),
                None => write!(f, "Couldn't read “{name}”."),
            },
        }
    }
}

impl std::error::Error for LyricsFileError {}

/// The short reason Python puts in `OSError.strerror`, such as "Permission denied".
fn strerror(error: &io::Error) -> Option<String> {
    use io::ErrorKind as K;
    let text = match error.kind() {
        K::NotFound => "No such file or directory",
        K::PermissionDenied => "Permission denied",
        K::IsADirectory => "Is a directory",
        K::InvalidInput | K::InvalidFilename => "Invalid argument",
        K::AlreadyExists => "File exists",
        K::StorageFull => "No space left on device",
        K::ResourceBusy => "Device or resource busy",
        _ => {
            // Rust adds " (os error N)"; Python's strerror is only the text before it.
            let full = error.to_string();
            let text = match full.rfind(" (os error ") {
                Some(i) => full[..i].to_owned(),
                None => full,
            };
            let text = text.trim_end_matches('.').trim().to_owned();
            return (!text.is_empty()).then_some(text);
        }
    };
    Some(text.to_owned())
}

/// The file name the messages use (`os.path.basename`).
pub(crate) fn base_name(path: &Path) -> String {
    let s = path.to_string_lossy();
    match s.rfind(['/', '\\']) {
        Some(i) => s[i + 1..].to_owned(),
        None => s.into_owned(),
    }
}

/// Reads an .lrc or .txt file.
///
/// The error's `Display` is the message the Python app shows for it
/// (`file_error_text`), such as "Couldn't read “x.lrc”: Permission denied."
pub fn read_lyrics_file(path: impl AsRef<Path>) -> Result<String, LyricsFileError> {
    let path = path.as_ref();
    let io_error = |error| LyricsFileError::Io {
        name: base_name(path),
        error,
    };
    let file = std::fs::File::open(path).map_err(io_error)?;
    let mut data = Vec::new();
    file.take(LYRICS_FILE_LIMIT + 1)
        .read_to_end(&mut data)
        .map_err(io_error)?;
    if data.len() as u64 > LYRICS_FILE_LIMIT {
        return Err(LyricsFileError::TooBig {
            name: base_name(path),
        });
    }
    Ok(decode_lyrics_bytes(&data))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn boms_win() {
        assert_eq!(decode_lyrics_bytes_with(b"\xef\xbb\xbfhi", 1252), "hi");
        assert_eq!(decode_lyrics_bytes_with(b"\xff\xfeh\x00i\x00", 1252), "hi");
        assert_eq!(decode_lyrics_bytes_with(b"\xfe\xff\x00h\x00i", 1252), "hi");
        assert_eq!(
            decode_lyrics_bytes_with(b"\xff\xfe\x00\x00h\x00\x00\x00", 1252),
            "h"
        );
        assert_eq!(
            decode_lyrics_bytes_with(b"\x00\x00\xfe\xff\x00\x00\x00h\x00", 1252),
            "h\u{fffd}"
        );
    }

    #[test]
    fn shift_jis_needs_kana() {
        // "こんにちは" in Shift-JIS.
        let sjis = b"\x82\xb1\x82\xf1\x82\xc9\x82\xbf\x82\xcd";
        assert_eq!(decode_lyrics_bytes_with(sjis, 1252), "こんにちは");
        // "café" in Windows-1252 also decodes as Shift-JIS, but without kana.
        assert_eq!(decode_lyrics_bytes_with(b"caf\xe9", 1252), "café");
    }

    #[test]
    fn undefined_bytes_fall_back_to_1252_with_replacement() {
        assert_eq!(decode_lyrics_bytes_with(b"a\x81b\xe9", 1252), "a\u{fffd}bé");
        // Windows-1251 knows 0xC0 (А), so it wins.
        assert_eq!(decode_lyrics_bytes_with(b"\xc0\xc1", 1251), "АБ");
    }

    #[test]
    fn missing_file_message() {
        let err = read_lyrics_file("/definitely/not/here.lrc").unwrap_err();
        assert_eq!(
            err.to_string(),
            "Couldn't read “here.lrc”: No such file or directory."
        );
    }

    #[test]
    fn too_big_message() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("big.lrc");
        std::fs::write(&path, vec![b'a'; LYRICS_FILE_LIMIT as usize + 1]).unwrap();
        assert_eq!(
            read_lyrics_file(&path).unwrap_err().to_string(),
            "“big.lrc” is too big to be a lyrics file."
        );
        std::fs::write(&path, vec![b'a'; LYRICS_FILE_LIMIT as usize]).unwrap();
        assert!(read_lyrics_file(&path).is_ok());
    }
}
