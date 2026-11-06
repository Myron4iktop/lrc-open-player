//! Opus length and bitrate the way mutagen works them out (`mutagen.oggopus.OggOpusInfo`).
//!
//! mutagen divides the bytes after the comment header by the length. Lofty counts the
//! bytes a little differently, which moves the bitrate by 1 kbps on some files.

use std::io::{Read, Seek, SeekFrom};

/// One Ogg page header: where its body ends, its granule position, its stream and
/// segment table.
struct Page {
    end: u64,
    granule: i64,
    serial: u32,
    segments: Vec<u8>,
}

fn read_page<R: Read + Seek>(r: &mut R, at: u64) -> Option<Page> {
    r.seek(SeekFrom::Start(at)).ok()?;
    let mut head = [0u8; 27];
    r.read_exact(&mut head).ok()?;
    if &head[..4] != b"OggS" {
        return None;
    }
    let granule = i64::from_le_bytes(head[6..14].try_into().unwrap());
    let serial = u32::from_le_bytes(head[14..18].try_into().unwrap());
    let mut segments = vec![0u8; usize::from(head[26])];
    r.read_exact(&mut segments).ok()?;
    let body: u64 = segments.iter().map(|&s| u64::from(s)).sum();
    Some(Page {
        end: at + 27 + segments.len() as u64 + body,
        granule,
        serial,
        segments,
    })
}

/// mutagen's (length in seconds, bitrate in bits per second) for an Ogg Opus file.
pub(crate) fn opus_info<R: Read + Seek>(r: &mut R) -> Option<(f64, u64)> {
    let size = r.seek(SeekFrom::End(0)).ok()?;
    let first = read_page(r, 0)?;
    let mut head = [0u8; 19];
    r.seek(SeekFrom::Start(28 + first.segments.len() as u64 - 1))
        .ok()?;
    r.read_exact(&mut head).ok()?;
    if &head[..8] != b"OpusHead" {
        return None;
    }
    let pre_skip = i64::from(u16::from_le_bytes([head[10], head[11]]));

    // The comment header is the second packet; mutagen counts the audio from the end of
    // the page where it finishes.
    let mut packets_done = first.segments.iter().filter(|&&s| s < 255).count();
    let mut at = first.end;
    let mut last_granule = first.granule;
    let mut tags_end = None;
    if packets_done >= 2 {
        tags_end = Some(first.end);
    }
    while at < size {
        let Some(page) = read_page(r, at) else {
            break;
        };
        if page.serial == first.serial {
            if tags_end.is_none() {
                packets_done += page.segments.iter().filter(|&&s| s < 255).count();
                if packets_done >= 2 {
                    tags_end = Some(page.end);
                }
            }
            last_granule = page.granule;
        }
        at = page.end;
    }
    let audio_size = size.saturating_sub(tags_end?);
    let length = (last_granule - pre_skip) as f64 / 48000.0;
    let bitrate = if length != 0.0 {
        (audio_size as f64 * 8.0 / length)
            .round_ties_even()
            .max(0.0) as u64
    } else {
        0
    };
    Some((length, bitrate))
}
