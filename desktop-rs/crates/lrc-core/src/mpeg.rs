//! MP3 length and bitrate the way mutagen works them out (`mutagen.mp3.MPEGInfo`).
//!
//! The player's track details show both, and lofty's estimate of the bitrate is often
//! 1 kbps off mutagen's. This follows mutagen step by step: skip the ID3v2 tags, find a
//! run of valid frames in the first megabyte, and prefer a Xing/Info or VBRI header's
//! frame and byte counts over the first frame's bitrate.

use std::io::{Read, Seek, SeekFrom};

/// The stream numbers mutagen reports for an MP3.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct MpegInfo {
    /// Bits per second.
    pub bitrate: u64,
    pub length: f64,
    pub sample_rate: u32,
    pub channels: u32,
}

const MONO: u8 = 3;

struct Frame {
    offset: u64,
    bitrate: u64,
    sample_rate: u32,
    channels: u32,
    /// Set when a Xing or VBRI header gave the length.
    length: Option<f64>,
    /// No Xing or VBRI header: the frame may be a false sync.
    sketchy: bool,
}

fn bitrate_table(version: f32, layer: u8) -> [u64; 15] {
    match (version, layer) {
        (1.0, 1) => [
            0, 32, 64, 96, 128, 160, 192, 224, 256, 288, 320, 352, 384, 416, 448,
        ],
        (1.0, 2) => [
            0, 32, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320, 384,
        ],
        (1.0, 3) => [
            0, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320,
        ],
        (_, 1) => [
            0, 32, 48, 56, 64, 80, 96, 112, 128, 144, 160, 176, 192, 224, 256,
        ],
        _ => [0, 8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160],
    }
}

/// Python's `round()`-to-int via Decimal: halves go to the even neighbour.
fn intround(x: f64) -> u64 {
    x.round_ties_even().max(0.0) as u64
}

fn read_exact_at<R: Read + Seek>(r: &mut R, pos: u64, len: usize) -> Option<Vec<u8>> {
    r.seek(SeekFrom::Start(pos)).ok()?;
    let mut buf = vec![0; len];
    let mut filled = 0;
    while filled < len {
        match r.read(&mut buf[filled..]) {
            Ok(0) => break,
            Ok(n) => filled += n,
            Err(_) => return None,
        }
    }
    buf.truncate(filled);
    Some(buf)
}

fn be32(b: &[u8]) -> u64 {
    u64::from(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
}

/// `LAMEHeader.parse_version`: true if an extended LAME header follows the version.
fn lame_has_header(data: &[u8]) -> bool {
    if data.len() != 20 || !(data.starts_with(b"LAME") || data.starts_with(b"L3.99")) {
        return false;
    }
    let data: &[u8] = {
        let mut d = data;
        while let Some((&c, rest)) = d.split_first() {
            if b"EMAL".contains(&c) {
                d = rest;
            } else {
                break;
            }
        }
        d
    };
    let Some((&major, rest)) = data.split_first() else {
        return false;
    };
    let mut rest = rest;
    while let Some((b'.', r)) = rest.split_first() {
        rest = r;
    }
    let minor_len = rest.iter().take_while(|c| c.is_ascii_digit()).count();
    if !major.is_ascii_digit() || minor_len == 0 {
        return false;
    }
    let major = u32::from(major - b'0');
    let Ok(minor) = std::str::from_utf8(&rest[..minor_len])
        .unwrap()
        .parse::<u64>()
    else {
        return false;
    };
    let tail = &rest[minor_len..];
    let paren = tail.len() >= 11 && tail[tail.len() - 11] == b'(';
    if (major, minor) < (3, 90) || ((major, minor) == (3, 90) && paren) {
        return false;
    }
    tail.len() >= 11
}

impl Frame {
    /// `MPEGFrame.__init__`: the frame header at `offset`, and where the next one starts.
    fn parse<R: Read + Seek>(r: &mut R, offset: u64) -> Option<(Frame, u64)> {
        let h = read_exact_at(r, offset, 4)?;
        if h.len() != 4 {
            return None;
        }
        let bits = u32::from_be_bytes([h[0], h[1], h[2], h[3]]);
        if bits >> 21 != 0x7ff {
            return None;
        }
        let version = ((bits >> 19) & 3) as u8;
        let layer = ((bits >> 17) & 3) as u8;
        let bitrate_index = ((bits >> 12) & 0xf) as usize;
        let rate_index = ((bits >> 10) & 3) as usize;
        let padding = u64::from((bits >> 9) & 1);
        let mode = ((bits >> 6) & 3) as u8;
        if version == 1
            || layer == 0
            || rate_index == 3
            || bitrate_index == 0xf
            || bitrate_index == 0
        {
            return None;
        }
        let version = [2.5f32, 0.0, 2.0, 1.0][usize::from(version)];
        let layer = 4 - layer;
        let channels = if mode == MONO { 1 } else { 2 };
        let bitrate = bitrate_table(version, layer)[bitrate_index] * 1000;
        let sample_rate = match version {
            1.0 => [44100, 48000, 32000],
            2.0 => [22050, 24000, 16000],
            _ => [11025, 12000, 8000],
        }[rate_index];
        let (frame_size, slot) = if layer == 1 {
            (384u64, 4u64)
        } else if version >= 2.0 && layer == 3 {
            (576, 1)
        } else {
            (1152, 1)
        };
        let frame_length = ((frame_size / 8 * bitrate) / u64::from(sample_rate) + padding) * slot;
        let mut frame = Frame {
            offset,
            bitrate,
            sample_rate,
            channels,
            length: None,
            sketchy: true,
        };
        if layer == 3 {
            frame.parse_vbr_header(r, version, mode, frame_size, frame_length);
        }
        Some((frame, offset + frame_length))
    }

    fn parse_vbr_header<R: Read + Seek>(
        &mut self,
        r: &mut R,
        version: f32,
        mode: u8,
        frame_size: u64,
        frame_length: u64,
    ) {
        let xing_offset = match (version == 1.0, mode != MONO) {
            (true, true) => 36,
            (true, false) => 21,
            (false, true) => 21,
            (false, false) => 13,
        };
        if let Some(xing) = self.read_xing(r, self.offset + xing_offset) {
            self.sketchy = false;
            if let Some(frames) = xing.frames {
                let mut samples = frame_size as i64 * frames as i64;
                if let Some(bytes) = xing.bytes
                    && samples > 0
                {
                    let audio_bytes = bytes.saturating_sub(frame_length);
                    self.bitrate = intround(
                        (audio_bytes as f64 * 8.0 * f64::from(self.sample_rate)) / samples as f64,
                    );
                }
                if let Some((delay, padding)) = xing.lame_delay_padding {
                    samples -= delay as i64 + padding as i64;
                }
                let samples = samples.max(0);
                self.length = Some(samples as f64 / f64::from(self.sample_rate));
            }
            return;
        }
        // VBRI
        let Some(v) = read_exact_at(r, self.offset + 36, 26) else {
            return;
        };
        if v.len() != 26 || !v.starts_with(b"VBRI") || u16::from_be_bytes([v[4], v[5]]) != 1 {
            return;
        }
        let bytes = be32(&v[10..14]);
        let frames = be32(&v[14..18]);
        let toc_entries = u64::from(u16::from_be_bytes([v[18], v[19]]));
        let toc_entry_size = u64::from(u16::from_be_bytes([v[22], v[23]]));
        let toc_size = (toc_entry_size * toc_entries) as usize;
        let toc_start = self.offset + 36 + 26;
        let Some(toc) = read_exact_at(r, toc_start, toc_size) else {
            return;
        };
        if toc.len() != toc_size || !(toc_entry_size == 2 || toc_entry_size == 4) {
            return;
        }
        self.sketchy = false;
        let length = (frame_size * frames) as f64 / f64::from(self.sample_rate);
        self.length = Some(length);
        if length != 0.0 {
            self.bitrate = ((bytes as f64 * 8.0) / length) as u64;
        }
    }

    fn read_xing<R: Read + Seek>(&self, r: &mut R, at: u64) -> Option<Xing> {
        let head = read_exact_at(r, at, 8)?;
        if head.len() != 8 || !(head.starts_with(b"Xing") || head.starts_with(b"Info")) {
            return None;
        }
        let flags = be32(&head[4..8]);
        let mut pos = at + 8;
        let mut field = |size: usize| -> Option<Vec<u8>> {
            let data = read_exact_at(r, pos, size)?;
            pos += size as u64;
            (data.len() == size).then_some(data)
        };
        let mut xing = Xing::default();
        if flags & 1 != 0 {
            xing.frames = Some(be32(&field(4)?));
        }
        if flags & 2 != 0 {
            xing.bytes = Some(be32(&field(4)?));
        }
        if flags & 4 != 0 {
            field(100)?;
        }
        if flags & 8 != 0 {
            field(4)?;
        }
        let version_at = pos;
        if let Some(version) = read_exact_at(r, version_at, 20)
            && lame_has_header(&version)
            && let Some(payload) = read_exact_at(r, version_at + 9, 27)
            && payload.len() == 27
            && payload[0] >> 4 == 0
        {
            let d = &payload[12..15];
            let delay = (u32::from(d[0]) << 4) | (u32::from(d[1]) >> 4);
            let padding = (u32::from(d[1] & 0xf) << 8) | u32::from(d[2]);
            xing.lame_delay_padding = Some((delay, padding));
        }
        Some(xing)
    }
}

#[derive(Default)]
struct Xing {
    frames: Option<u64>,
    bytes: Option<u64>,
    lame_delay_padding: Option<(u32, u32)>,
}

/// Where the audio starts: after every ID3v2 tag at the front (WMP stacks several).
fn skip_id3<R: Read + Seek>(r: &mut R) -> u64 {
    let mut pos = 0;
    loop {
        let Some(h) = read_exact_at(r, pos, 10) else {
            return pos;
        };
        if h.len() < 10 || &h[..3] != b"ID3" {
            return pos;
        }
        let size = h[6..10]
            .iter()
            .fold(0u64, |acc, &b| (acc << 7) | u64::from(b & 0x7f));
        if size == 0 {
            return pos;
        }
        pos += 10 + size;
    }
}

/// mutagen's `MPEGInfo`, or `None` where it raises (no MPEG frames found).
pub(crate) fn mpeg_info<R: Read + Seek>(r: &mut R) -> Option<MpegInfo> {
    let start = skip_id3(r);
    let file_size = r.seek(SeekFrom::End(0)).ok()?;
    const MAX_READ: u64 = 1024 * 1024;
    let window = read_exact_at(r, start, MAX_READ as usize + 1)?;

    let mut max_syncs = 1500;
    let mut first: Option<Frame> = None;
    // iter_sync reads at most MAX_READ bytes, so a sync must fit in them.
    for i in 0..window.len().saturating_sub(1).min(MAX_READ as usize - 1) {
        if window[i] != 0xff || window[i + 1] & 0xe0 != 0xe0 {
            continue;
        }
        max_syncs -= 1;
        if max_syncs <= 0 {
            break;
        }
        let mut frames: Vec<Frame> = Vec::new();
        let mut at = start + i as u64;
        for _ in 0..4 {
            let Some((frame, next)) = Frame::parse(r, at) else {
                break;
            };
            let done = !frame.sketchy;
            frames.push(frame);
            at = next;
            if done {
                break;
            }
        }
        let keep_first = frames.len() >= 2 && first.is_none();
        if frames.last().is_some_and(|f| !f.sketchy) {
            first = frames.pop();
            break;
        }
        if frames.len() >= 4 {
            first = Some(frames.swap_remove(0));
            break;
        }
        if keep_first {
            first = Some(frames.swap_remove(0));
        }
    }
    let frame = first?;
    let length = match frame.length {
        Some(l) => l,
        None => {
            let content = file_size.saturating_sub(frame.offset);
            8.0 * content as f64 / frame.bitrate as f64
        }
    };
    Some(MpegInfo {
        bitrate: frame.bitrate,
        length,
        sample_rate: frame.sample_rate,
        channels: frame.channels,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lame_versions() {
        assert!(lame_has_header(
            b"LAME3.100\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00"
        ));
        assert!(lame_has_header(
            b"LAME3.99r\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00"
        ));
        assert!(!lame_has_header(
            b"LAME3.80\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00"
        ));
        assert!(!lame_has_header(
            b"Lavf\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00"
        ));
    }

    #[test]
    fn no_frames_is_none() {
        let mut c = std::io::Cursor::new(b"not an mp3 at all".to_vec());
        assert!(mpeg_info(&mut c).is_none());
    }
}
