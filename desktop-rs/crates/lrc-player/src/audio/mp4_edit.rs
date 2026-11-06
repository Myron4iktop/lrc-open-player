//! The edit list of an M4A file, which Symphonia ignores.
//!
//! AAC encoders put about 1,024 to 2,112 frames of silence in front of the music (the
//! "priming") and some padding after it. The file's edit list says which part is the song.
//! FFmpeg, behind Qt in the Python app, plays only that part, so the clock starts with the
//! music. Without it, M4A songs would run about 25 ms behind their lyrics.

use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::Path;

/// The part of the audio track that is the song, in the track's own time units.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Edit {
    /// Where the song starts (the priming to skip).
    pub start: u64,
    /// How long it is, if the edit list says.
    pub length: Option<u64>,
}

/// Reads the edit of the first sound track, if there is a usable one.
pub fn read(path: &Path) -> Option<Edit> {
    let file = File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    let mut r = BufReader::new(file);
    let moov = find_box(&mut r, 0, len, b"moov")?;
    let movie_scale = {
        let mvhd = find_box(&mut r, moov.0, moov.1, b"mvhd")?;
        r.seek(SeekFrom::Start(mvhd.0)).ok()?;
        let version = read_u8(&mut r)?;
        skip(&mut r, if version == 1 { 3 + 16 } else { 3 + 8 })?;
        read_u32(&mut r)?
    };
    // The first trak whose handler is "soun".
    let mut pos = moov.0;
    while let Some((start, end)) = find_box(&mut r, pos, moov.1, b"trak") {
        pos = end;
        let Some(mdia) = find_box(&mut r, start, end, b"mdia") else {
            continue;
        };
        let Some(hdlr) = find_box(&mut r, mdia.0, mdia.1, b"hdlr") else {
            continue;
        };
        r.seek(SeekFrom::Start(hdlr.0 + 8)).ok()?;
        let mut handler = [0u8; 4];
        r.read_exact(&mut handler).ok()?;
        if &handler != b"soun" {
            continue;
        }
        let media_scale = {
            let mdhd = find_box(&mut r, mdia.0, mdia.1, b"mdhd")?;
            r.seek(SeekFrom::Start(mdhd.0)).ok()?;
            let version = read_u8(&mut r)?;
            skip(&mut r, if version == 1 { 3 + 16 } else { 3 + 8 })?;
            read_u32(&mut r)?
        };
        let edts = find_box(&mut r, start, end, b"edts")?;
        let elst = find_box(&mut r, edts.0, edts.1, b"elst")?;
        r.seek(SeekFrom::Start(elst.0)).ok()?;
        let version = read_u8(&mut r)?;
        skip(&mut r, 3)?;
        let count = read_u32(&mut r)?;
        for _ in 0..count.min(16) {
            let (duration, media_time) = if version == 1 {
                (read_u64(&mut r)?, read_u64(&mut r)? as i64)
            } else {
                (
                    u64::from(read_u32(&mut r)?),
                    i64::from(read_u32(&mut r)? as i32),
                )
            };
            skip(&mut r, 4)?; // media rate
            if media_time < 0 {
                continue; // an empty edit (a delay before the track): FFmpeg ignores it too
            }
            let length = (movie_scale > 0 && duration > 0).then(|| {
                (u128::from(duration) * u128::from(media_scale) / u128::from(movie_scale)) as u64
            });
            return Some(Edit {
                start: media_time as u64,
                length,
            });
        }
        return None;
    }
    None
}

/// Finds the first child box named `name` between `from` and `to`; returns its content range.
fn find_box<R: Read + Seek>(r: &mut R, from: u64, to: u64, name: &[u8; 4]) -> Option<(u64, u64)> {
    let mut pos = from;
    while pos + 8 <= to {
        r.seek(SeekFrom::Start(pos)).ok()?;
        let size = u64::from(read_u32(r)?);
        let mut kind = [0u8; 4];
        r.read_exact(&mut kind).ok()?;
        let (header, size) = match size {
            0 => (8, to - pos),
            1 => (16, read_u64(r)?),
            n => (8, n),
        };
        if size < header || pos + size > to {
            return None;
        }
        if &kind == name {
            return Some((pos + header, pos + size));
        }
        pos += size;
    }
    None
}

fn read_u8<R: Read>(r: &mut R) -> Option<u8> {
    let mut b = [0u8; 1];
    r.read_exact(&mut b).ok()?;
    Some(b[0])
}

fn read_u32<R: Read>(r: &mut R) -> Option<u32> {
    let mut b = [0u8; 4];
    r.read_exact(&mut b).ok()?;
    Some(u32::from_be_bytes(b))
}

fn read_u64<R: Read>(r: &mut R) -> Option<u64> {
    let mut b = [0u8; 8];
    r.read_exact(&mut b).ok()?;
    Some(u64::from_be_bytes(b))
}

fn skip<R: Seek>(r: &mut R, n: i64) -> Option<()> {
    r.seek(SeekFrom::Current(n)).ok().map(|_| ())
}
