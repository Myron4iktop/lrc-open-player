//! Song tags: titles and numbers for album order, everything the player shows about a
//! song, the cover, embedded lyrics, and writing lyrics into a song.
//!
//! Ports `read_track_tags`, `extract_metadata_and_cover`, `_embedded_pictures`,
//! `load_cover_image` and `write_embedded_lyrics` from `lrcplayer.py`. The Python app
//! reads tags with mutagen, so this module rebuilds mutagen's view of each format from
//! lofty's format-specific tags: the same keys (`TIT2`, `USLT::eng`, `©nam`, `title`...),
//! the same values and the same order.

use std::fmt;
use std::fs::File;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use image::{ImageFormat, RgbaImage};
use lofty::TextEncoding;
use lofty::config::{ParseOptions, ParsingMode, WriteOptions};
use lofty::file::{AudioFile, FileType};
use lofty::flac::FlacFile;
use lofty::id3::v1::Id3v1Tag;
use lofty::id3::v2::{Frame, Id3v2Tag, UnsynchronizedTextFrame};
use lofty::iff::wav::WavFile;
use lofty::mp4::{Atom, AtomData, AtomIdent, Ilst, Mp4File};
use lofty::mpeg::MpegFile;
use lofty::ogg::tag::VorbisComments;
use lofty::ogg::{OggPictureStorage, OpusFile, VorbisFile};
use lofty::picture::{Picture, PictureInformation, PictureType};
use lofty::probe::Probe;
use lofty::tag::TagExt;
use regex::Regex;

use crate::mpeg::{MpegInfo, mpeg_info};
use crate::ogg::opus_info;
use crate::py;

// ---------------------------------------------------------------------------------------
// Opening a song
// ---------------------------------------------------------------------------------------

/// A song opened with the reader for its format.
enum Audio {
    /// With mutagen's length and bitrate, which lofty works out differently.
    Mpeg(MpegFile, MpegInfo),
    Wav(WavFile),
    Flac(FlacFile),
    Vorbis(VorbisFile),
    /// With mutagen's length and bitrate, which lofty works out differently.
    Opus(OpusFile, Option<(f64, u64)>),
    Mp4(Mp4File),
}

#[derive(Debug)]
enum OpenError {
    /// Not one of the formats the player knows (mutagen.File returned None).
    Unsupported,
    Io(io::Error),
    /// Lofty couldn't read or write the tags; its message.
    Tag(String),
}

impl From<io::Error> for OpenError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

fn tag_error(e: impl fmt::Display) -> OpenError {
    OpenError::Tag(e.to_string())
}

fn open_audio(path: &Path) -> Result<Audio, OpenError> {
    let mut file = File::open(path)?;
    let probe = Probe::new(&mut file).guess_file_type()?;
    let Some(file_type) = probe.file_type() else {
        return Err(OpenError::Unsupported);
    };
    // Lofty "upgrades" some values while reading. For ID3 and MP4 that is what mutagen
    // does too (TYER to TDRC, gnre to ©gen), but in Vorbis comments it would split
    // TRACKNUMBER=5/12 in two, which mutagen leaves alone.
    let vorbis = matches!(
        file_type,
        FileType::Flac | FileType::Vorbis | FileType::Opus
    );
    let options = ParseOptions::new()
        .parsing_mode(ParsingMode::BestAttempt)
        .implicit_conversions(!vorbis);
    let reader = probe.into_inner();
    Ok(match file_type {
        FileType::Mpeg => {
            let tags = MpegFile::read_from(&mut *reader, options).map_err(tag_error)?;
            // mutagen refuses an MP3 in which it finds no MPEG frames.
            let info = mpeg_info(reader).ok_or(OpenError::Unsupported)?;
            Audio::Mpeg(tags, info)
        }
        FileType::Wav => Audio::Wav(WavFile::read_from(reader, options).map_err(tag_error)?),
        FileType::Flac => Audio::Flac(FlacFile::read_from(reader, options).map_err(tag_error)?),
        FileType::Vorbis => {
            Audio::Vorbis(VorbisFile::read_from(reader, options).map_err(tag_error)?)
        }
        FileType::Opus => {
            let tags = OpusFile::read_from(&mut *reader, options).map_err(tag_error)?;
            Audio::Opus(tags, opus_info(reader))
        }
        FileType::Mp4 => Audio::Mp4(Mp4File::read_from(reader, options).map_err(tag_error)?),
        _ => return Err(OpenError::Unsupported),
    })
}

// ---------------------------------------------------------------------------------------
// mutagen's view of the tags
// ---------------------------------------------------------------------------------------

/// One value of a tag, as mutagen hands it out.
#[derive(Debug, Clone)]
enum Value {
    Text(String),
    /// MP4 `trkn` and `disk`: (number, total).
    Pair(u32, u32),
    Int(i64),
}

impl Value {
    /// `str(value)` in Python.
    fn py_str(&self) -> String {
        match self {
            Value::Text(t) => t.clone(),
            Value::Pair(a, b) => format!("({a}, {b})"),
            Value::Int(n) => n.to_string(),
        }
    }
}

/// One tag under its mutagen key, such as `TIT2`, `USLT::eng`, `©nam` or `title`.
#[derive(Debug, Clone)]
struct Item {
    key: String,
    values: Vec<Value>,
}

impl Item {
    fn list(key: impl Into<String>, values: Vec<Value>) -> Self {
        Item {
            key: key.into(),
            values,
        }
    }

    /// The text mutagen gives for the whole tag: every value on its own line.
    fn text(&self) -> String {
        self.values
            .iter()
            .map(Value::py_str)
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// ID3v2.4 keeps several values in one text frame, separated by NUL characters.
fn split_nul(text: &str) -> Vec<Value> {
    let text = text.strip_suffix('\0').unwrap_or(text);
    text.split('\0')
        .map(|t| Value::Text(t.to_owned()))
        .collect()
}

fn lang(code: &[u8; 3]) -> String {
    code.iter().map(|&b| b as char).collect()
}

/// mutagen's genre names for ID3 genre numbers (`mutagen._constants.GENRES`).
const GENRES: [&str; 192] = [
    "Blues",
    "Classic Rock",
    "Country",
    "Dance",
    "Disco",
    "Funk",
    "Grunge",
    "Hip-Hop",
    "Jazz",
    "Metal",
    "New Age",
    "Oldies",
    "Other",
    "Pop",
    "R&B",
    "Rap",
    "Reggae",
    "Rock",
    "Techno",
    "Industrial",
    "Alternative",
    "Ska",
    "Death Metal",
    "Pranks",
    "Soundtrack",
    "Euro-Techno",
    "Ambient",
    "Trip-Hop",
    "Vocal",
    "Jazz+Funk",
    "Fusion",
    "Trance",
    "Classical",
    "Instrumental",
    "Acid",
    "House",
    "Game",
    "Sound Clip",
    "Gospel",
    "Noise",
    "Alt. Rock",
    "Bass",
    "Soul",
    "Punk",
    "Space",
    "Meditative",
    "Instrumental Pop",
    "Instrumental Rock",
    "Ethnic",
    "Gothic",
    "Darkwave",
    "Techno-Industrial",
    "Electronic",
    "Pop-Folk",
    "Eurodance",
    "Dream",
    "Southern Rock",
    "Comedy",
    "Cult",
    "Gangsta Rap",
    "Top 40",
    "Christian Rap",
    "Pop/Funk",
    "Jungle",
    "Native American",
    "Cabaret",
    "New Wave",
    "Psychedelic",
    "Rave",
    "Showtunes",
    "Trailer",
    "Lo-Fi",
    "Tribal",
    "Acid Punk",
    "Acid Jazz",
    "Polka",
    "Retro",
    "Musical",
    "Rock & Roll",
    "Hard Rock",
    "Folk",
    "Folk-Rock",
    "National Folk",
    "Swing",
    "Fast-Fusion",
    "Bebop",
    "Latin",
    "Revival",
    "Celtic",
    "Bluegrass",
    "Avantgarde",
    "Gothic Rock",
    "Progressive Rock",
    "Psychedelic Rock",
    "Symphonic Rock",
    "Slow Rock",
    "Big Band",
    "Chorus",
    "Easy Listening",
    "Acoustic",
    "Humour",
    "Speech",
    "Chanson",
    "Opera",
    "Chamber Music",
    "Sonata",
    "Symphony",
    "Booty Bass",
    "Primus",
    "Porn Groove",
    "Satire",
    "Slow Jam",
    "Club",
    "Tango",
    "Samba",
    "Folklore",
    "Ballad",
    "Power Ballad",
    "Rhythmic Soul",
    "Freestyle",
    "Duet",
    "Punk Rock",
    "Drum Solo",
    "A Cappella",
    "Euro-House",
    "Dance Hall",
    "Goa",
    "Drum & Bass",
    "Club-House",
    "Hardcore",
    "Terror",
    "Indie",
    "BritPop",
    "Afro-Punk",
    "Polsk Punk",
    "Beat",
    "Christian Gangsta Rap",
    "Heavy Metal",
    "Black Metal",
    "Crossover",
    "Contemporary Christian",
    "Christian Rock",
    "Merengue",
    "Salsa",
    "Thrash Metal",
    "Anime",
    "JPop",
    "Synthpop",
    "Abstract",
    "Art Rock",
    "Baroque",
    "Bhangra",
    "Big Beat",
    "Breakbeat",
    "Chillout",
    "Downtempo",
    "Dub",
    "EBM",
    "Eclectic",
    "Electro",
    "Electroclash",
    "Emo",
    "Experimental",
    "Garage",
    "Global",
    "IDM",
    "Illbient",
    "Industro-Goth",
    "Jam Band",
    "Krautrock",
    "Leftfield",
    "Lounge",
    "Math Rock",
    "New Romantic",
    "Nu-Breakz",
    "Post-Punk",
    "Post-Rock",
    "Psytrance",
    "Shoegaze",
    "Space Rock",
    "Trop Rock",
    "World Music",
    "Neoclassical",
    "Audiobook",
    "Audio Theatre",
    "Neue Deutsche Welle",
    "Podcast",
    "Indie Rock",
    "G-Funk",
    "Dubstep",
    "Garage Rock",
    "Psybient",
];

/// `TCON.genres`: mutagen rewrites every ID3 genre this way when it loads a tag, so
/// "17", "(17)" and "(17)Rock" all read as "Rock".
fn id3_genres(texts: &[Value]) -> Vec<Value> {
    let mut genres: Vec<String> = Vec::new();
    for value in texts {
        let Value::Text(value) = value else { continue };
        if !value.is_empty() && value.chars().all(py::is_digit) {
            let n: usize = py::ascii_digits(value).parse().unwrap_or(usize::MAX);
            if n < 256 {
                genres.push(GENRES.get(n).copied().unwrap_or("Unknown").to_owned());
                continue;
            }
        }
        match value.as_str() {
            "CR" => genres.push("Cover".into()),
            "RX" => genres.push("Remix".into()),
            "" => {}
            _ => {
                // ((?:\((?P<id>[0-9]+|RX|CR)\))*)(?P<str>.+)?
                let mut new: Vec<String> = Vec::new();
                let mut rest = value.as_str();
                while let Some(inner) = rest.strip_prefix('(') {
                    let end = inner.find(')');
                    let Some(end) = end else { break };
                    let id = &inner[..end];
                    let ok = id == "RX"
                        || id == "CR"
                        || (!id.is_empty() && id.bytes().all(|b| b.is_ascii_digit()));
                    if !ok {
                        break;
                    }
                    new.push(match id {
                        "CR" => "Cover".to_owned(),
                        "RX" => "Remix".to_owned(),
                        _ => match id.parse::<usize>().ok().and_then(|n| GENRES.get(n)) {
                            Some(g) => (*g).to_owned(),
                            None => "Unknown".to_owned(),
                        },
                    });
                    rest = &inner[end + 1..];
                }
                // `.` stops at a line break.
                let mut name = rest.split('\n').next().unwrap_or("");
                if !name.is_empty() {
                    if name.starts_with("((") {
                        name = &name[1..];
                    }
                    if !new.iter().any(|g| g == name) {
                        new.push(name.to_owned());
                    }
                }
                genres.extend(new);
            }
        }
    }
    genres.into_iter().map(Value::Text).collect()
}

/// mutagen's ID3 keys and values, in the order of the frames in the file.
fn id3_items(tag: &Id3v2Tag) -> Vec<Item> {
    let mut items = Vec::new();
    for frame in tag {
        let id = frame.id_str().to_owned();
        let item = match frame {
            Frame::Text(f) if id == "TCON" => Item::list(id, id3_genres(&split_nul(&f.value))),
            Frame::Text(f) => Item::list(id, split_nul(&f.value)),
            Frame::UserText(f) => {
                Item::list(format!("TXXX:{}", f.description), split_nul(&f.content))
            }
            Frame::Comment(f) => Item::list(
                format!("COMM:{}:{}", f.description, lang(&f.language)),
                split_nul(&f.content),
            ),
            Frame::UnsynchronizedText(f) => Item::list(
                format!("USLT:{}:{}", f.description, lang(&f.language)),
                vec![Value::Text(f.content.to_string())],
            ),
            Frame::Timestamp(f) => Item::list(id, vec![Value::Text(f.timestamp.to_string())]),
            Frame::Url(f) => Item::list(id, vec![Value::Text(f.url().to_owned())]),
            Frame::UserUrl(f) => Item::list(
                format!("WXXX:{}", f.description),
                vec![Value::Text(f.content.to_string())],
            ),
            // Pictures, private data and other binary frames are never text.
            _ => continue,
        };
        items.push(item);
    }
    items
}

/// mutagen adds the ID3v1 fields an MP3 has at its end, where ID3v2 lacks them.
fn merge_id3v1(items: &mut Vec<Item>, v1: &Id3v1Tag) {
    let mut add = |key: &str, value: String| {
        if !items.iter().any(|i| i.key == key) {
            items.push(Item::list(key, vec![Value::Text(value)]));
        }
    };
    let field = |v: &Option<String>| {
        v.as_deref()
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .map(str::to_owned)
    };
    if let Some(t) = field(&v1.title) {
        add("TIT2", t);
    }
    if let Some(t) = field(&v1.artist) {
        add("TPE1", t);
    }
    if let Some(t) = field(&v1.album) {
        add("TALB", t);
    }
    if let Some(y) = v1.year {
        add("TDRC", y.to_string());
    }
    if let Some(t) = field(&v1.comment) {
        add("COMM:ID3v1 Comment:eng", t);
    }
    if let Some(n) = v1.track_number.filter(|&n| n > 0) {
        add("TRCK", n.to_string());
    }
    if let Some(g) = v1.genre.filter(|&g| g != 255)
        && let Some(Value::Text(genre)) = id3_genres(&[Value::Text(g.to_string())]).pop()
    {
        add("TCON", genre);
    }
}

fn atom_key(ident: &AtomIdent<'_>) -> String {
    match ident {
        AtomIdent::Fourcc(code) => code.iter().map(|&b| b as char).collect(),
        AtomIdent::Freeform { mean, name } => format!("----:{mean}:{name}"),
    }
}

/// mutagen's MP4 keys and values, in the order of the atoms in the file.
fn mp4_items(ilst: &Ilst) -> Vec<Item> {
    let mut items = Vec::new();
    for atom in ilst {
        let key = atom_key(atom.ident());
        if key == "covr" {
            continue;
        }
        let number_pair = key == "trkn" || key == "disk";
        let mut values = Vec::new();
        for data in atom.data() {
            match data {
                AtomData::UTF8(t) | AtomData::UTF16(t) => values.push(Value::Text(t.clone())),
                AtomData::SignedInteger(n) => values.push(Value::Int(i64::from(*n))),
                AtomData::UnsignedInteger(n) => values.push(Value::Int(i64::from(*n))),
                AtomData::Bool(b) => {
                    values.push(Value::Text(if *b { "True" } else { "False" }.to_owned()));
                }
                AtomData::Unknown { data, .. } if number_pair => {
                    if data.len() >= 6 {
                        let number = u16::from_be_bytes([data[2], data[3]]);
                        let total = u16::from_be_bytes([data[4], data[5]]);
                        values.push(Value::Pair(u32::from(number), u32::from(total)));
                    }
                }
                AtomData::Unknown { data, .. } if key.starts_with("----:") => {
                    // mutagen shows these as Python bytes ("b'...'"); the text is more useful.
                    values.push(Value::Text(String::from_utf8_lossy(data).into_owned()));
                }
                _ => {}
            }
        }
        if !values.is_empty() {
            items.push(Item::list(key, values));
        }
    }
    items
}

/// mutagen's Vorbis comment keys: lower case, one per name, with all its values.
fn vorbis_items(tag: &VorbisComments) -> Vec<Item> {
    let mut items: Vec<Item> = Vec::new();
    for (key, value) in tag.items() {
        let key = key.to_ascii_lowercase();
        match items.iter_mut().find(|i| i.key == key) {
            Some(item) => item.values.push(Value::Text(value.to_owned())),
            None => items.push(Item::list(key, vec![Value::Text(value.to_owned())])),
        }
    }
    items
}

/// The format of a song's tags, which decides how mutagen names things.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum TagKind {
    Id3,
    Mp4,
    Vorbis,
}

/// mutagen's tags for a song, or `None` when it has none (`audio.tags is None`).
fn mutagen_items(audio: &Audio) -> Option<(TagKind, Vec<Item>)> {
    match audio {
        Audio::Mpeg(f, _) => {
            let mut items = f.id3v2().map(id3_items);
            if let Some(v1) = f.id3v1() {
                merge_id3v1(items.get_or_insert_with(Vec::new), v1);
            }
            items.map(|i| (TagKind::Id3, i))
        }
        Audio::Wav(f) => f.id3v2().map(|t| (TagKind::Id3, id3_items(t))),
        Audio::Mp4(f) => f.ilst().map(|t| (TagKind::Mp4, mp4_items(t))),
        Audio::Flac(f) => f
            .vorbis_comments()
            .map(|t| (TagKind::Vorbis, vorbis_items(t))),
        Audio::Vorbis(f) => Some((TagKind::Vorbis, vorbis_items(f.vorbis_comments()))),
        Audio::Opus(f, _) => Some((TagKind::Vorbis, vorbis_items(f.vorbis_comments()))),
    }
}

/// `tags.get(key)`: ID3 and MP4 keys are exact, Vorbis keys ignore case.
fn get<'a>(kind: TagKind, items: &'a [Item], keys: &[&str]) -> Option<&'a Item> {
    for key in keys {
        let found = items.iter().find(|i| match kind {
            TagKind::Vorbis => i.key.eq_ignore_ascii_case(key),
            _ => i.key == *key,
        });
        if let Some(item) = found
            && !item.values.is_empty()
        {
            return Some(item);
        }
    }
    None
}

// ---------------------------------------------------------------------------------------
// Title, track and disc (read_track_tags)
// ---------------------------------------------------------------------------------------

/// Title, track and disc number from a song's tags (`None` for whatever is missing).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TrackTags {
    pub title: Option<String>,
    pub track: Option<u64>,
    pub disc: Option<u64>,
}

static LEADING_NUMBER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[\s\x1c-\x1f]*(\d+)").unwrap());

/// The leading number of a track/disc tag in any format: '5', '05/12', MP4's (5, 12).
fn first_number(item: &Item) -> Option<u64> {
    match item.values.first()? {
        Value::Pair(n, _) => (*n > 0).then_some(u64::from(*n)),
        Value::Int(n) => (*n > 0).then_some(*n as u64),
        Value::Text(t) => {
            let caps = LEADING_NUMBER.captures(t)?;
            let digits = py::ascii_digits(&caps[1]);
            let digits = digits.trim_start_matches('0');
            if digits.is_empty() {
                return None;
            }
            Some(digits.parse().unwrap_or(u64::MAX))
        }
    }
}

fn first_text(item: &Item) -> Option<String> {
    let text = item.values.first()?.py_str();
    let text = py::strip(&text);
    (!text.is_empty()).then(|| text.to_owned())
}

/// Title, track and disc number from a song's tags (`read_track_tags`).
///
/// Each format names them differently: ID3 (MP3, WAV) uses TIT2 / TRCK / TPOS, MP4 (M4A)
/// uses ©nam / trkn / disk, and Vorbis comments (FLAC, OGG, Opus) use title /
/// tracknumber / discnumber.
pub fn read_track_tags(path: &Path) -> TrackTags {
    let Ok(audio) = open_audio(path) else {
        return TrackTags::default();
    };
    let Some((kind, items)) = mutagen_items(&audio) else {
        return TrackTags::default();
    };
    TrackTags {
        title: get(kind, &items, &["TIT2", "\u{a9}nam", "title"]).and_then(first_text),
        track: get(kind, &items, &["TRCK", "trkn", "tracknumber"]).and_then(first_number),
        disc: get(kind, &items, &["TPOS", "disk", "discnumber"]).and_then(first_number),
    }
}

// ---------------------------------------------------------------------------------------
// Covers
// ---------------------------------------------------------------------------------------

/// A cover that passed the safety check: its bytes as stored, and the decoded image.
#[derive(Clone)]
pub struct Cover {
    pub data: Vec<u8>,
    pub image: RgbaImage,
}

impl fmt::Debug for Cover {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Cover")
            .field("bytes", &self.data.len())
            .field("width", &self.image.width())
            .field("height", &self.image.height())
            .finish()
    }
}

/// A cover image from its bytes, only if it's a plain raster image (`load_cover_image`).
///
/// Only JPEG, PNG, GIF, BMP and WebP, judged by the content, not a file extension. SVG
/// and some other formats can reference an external path, and on Windows fetching one
/// from a network share can leak the user's password hash, so untrusted files never get
/// that far. Returns `None` when the data isn't one of the safe formats or won't decode.
pub fn load_cover_image(data: &[u8]) -> Option<RgbaImage> {
    let format = image::guess_format(data).ok()?;
    if !matches!(
        format,
        ImageFormat::Jpeg
            | ImageFormat::Png
            | ImageFormat::Gif
            | ImageFormat::Bmp
            | ImageFormat::WebP
    ) {
        return None;
    }
    let img = image::load_from_memory_with_format(data, format).ok()?;
    (img.width() > 0 && img.height() > 0).then(|| img.to_rgba8())
}

fn load_cover(data: Vec<u8>) -> Option<Cover> {
    let image = load_cover_image(&data)?;
    Some(Cover { data, image })
}

/// Lofty turns the old Vorbis `COVERART` field into a picture, which mutagen ignores.
/// Such a picture is the only kind with no description and no size information.
fn from_coverart_field(picture: &Picture, info: &PictureInformation) -> bool {
    picture.pic_type() == PictureType::Other
        && picture.description().is_none()
        && *info == PictureInformation::default()
}

/// Embedded image data of any format, front cover first (`_embedded_pictures`).
fn embedded_pictures(audio: &Audio) -> Vec<Vec<u8>> {
    let mut pics: Vec<(u8, Vec<u8>)> = Vec::new(); // type 3 is the front cover
    let vorbis = |tag: &VorbisComments, pics: &mut Vec<(u8, Vec<u8>)>| {
        for (picture, info) in tag.pictures() {
            if !from_coverart_field(picture, info) {
                pics.push((picture.pic_type().as_u8(), picture.data().to_vec()));
            }
        }
    };
    let id3 = |tag: &Id3v2Tag, pics: &mut Vec<(u8, Vec<u8>)>| {
        for frame in tag {
            if let Frame::Picture(f) = frame {
                pics.push((f.picture.pic_type().as_u8(), f.picture.data().to_vec()));
            }
        }
    };
    match audio {
        Audio::Flac(f) => {
            for (picture, _) in OggPictureStorage::pictures(f) {
                pics.push((picture.pic_type().as_u8(), picture.data().to_vec()));
            }
            if let Some(tag) = f.vorbis_comments() {
                vorbis(tag, &mut pics);
            }
        }
        Audio::Mpeg(f, _) => {
            if let Some(tag) = f.id3v2() {
                id3(tag, &mut pics);
            }
        }
        Audio::Wav(f) => {
            if let Some(tag) = f.id3v2() {
                id3(tag, &mut pics);
            }
        }
        Audio::Mp4(f) => {
            if let Some(ilst) = f.ilst()
                && let Some(pictures) = ilst.pictures()
            {
                for picture in pictures {
                    pics.push((3, picture.data().to_vec()));
                }
            }
        }
        Audio::Vorbis(f) => vorbis(f.vorbis_comments(), &mut pics),
        Audio::Opus(f, _) => vorbis(f.vorbis_comments(), &mut pics),
    }
    pics.sort_by_key(|p| if p.0 == 3 { 0 } else { 1 });
    pics.into_iter().map(|p| p.1).collect()
}

// ---------------------------------------------------------------------------------------
// Everything about a song (extract_metadata_and_cover)
// ---------------------------------------------------------------------------------------

/// What the player shows about a song: the `info` dict of `extract_metadata_and_cover`.
#[derive(Debug, Clone, PartialEq)]
pub struct TrackInfo {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub album_artist: String,
    pub track_number: String,
    pub disc_number: String,
    pub genre: String,
    pub year: String,
    pub duration_sec: u64,
    pub duration_str: String,
    pub mime_type: String,
    pub bitrate_kbps: u64,
    pub sample_rate_khz: f64,
    pub bits_per_sample: u32,
    pub channels: u32,
    pub is_hd: bool,
    pub file_path: PathBuf,
    /// False: the title above is just the file name.
    pub title_from_tags: bool,
}

impl TrackInfo {
    /// The defaults for a song whose tags can't be read.
    pub fn defaults(file_path: &Path) -> Self {
        let title = file_path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        TrackInfo {
            title,
            artist: "Unknown Artist".into(),
            album: "Unknown Album".into(),
            album_artist: "Unknown Album Artist".into(),
            track_number: "-".into(),
            disc_number: "-".into(),
            genre: "Unknown Genre".into(),
            year: "-".into(),
            duration_sec: 0,
            duration_str: "0:00".into(),
            mime_type: "audio/unknown".into(),
            bitrate_kbps: 0,
            sample_rate_khz: 0.0,
            bits_per_sample: 16,
            channels: 2,
            is_hd: false,
            file_path: file_path.to_path_buf(),
            title_from_tags: false,
        }
    }
}

/// A song's lyrics from its tags, its cover, and its details.
#[derive(Debug, Clone)]
pub struct TrackMetadata {
    pub lyrics: Option<String>,
    pub cover: Option<Cover>,
    pub info: TrackInfo,
}

/// The file's extension in lower case, with its dot (`os.path.splitext(p)[1].lower()`).
fn lower_ext(path: &Path) -> String {
    path.extension()
        .map(|e| format!(".{}", e.to_string_lossy().to_lowercase()))
        .unwrap_or_default()
}

/// The stream numbers mutagen reports, where it reports them.
struct StreamInfo {
    sample_rate: Option<u32>,
    channels: Option<u32>,
    bits_per_sample: Option<u32>,
    bitrate_kbps: Option<u64>,
    length_sec: f64,
}

fn stream_info(audio: &Audio) -> StreamInfo {
    let secs = |d: std::time::Duration| d.as_secs_f64();
    match audio {
        Audio::Mpeg(_, m) => StreamInfo {
            sample_rate: Some(m.sample_rate),
            channels: Some(m.channels),
            bits_per_sample: None,
            bitrate_kbps: Some(m.bitrate / 1000),
            length_sec: m.length,
        },
        Audio::Wav(f) => {
            let p = f.properties();
            let channels = u32::from(p.channels());
            let bits = u32::from(p.bit_depth());
            // mutagen: channels × bits × sample rate.
            let bps = u64::from(channels) * u64::from(bits) * u64::from(p.sample_rate());
            StreamInfo {
                sample_rate: Some(p.sample_rate()),
                channels: Some(channels),
                bits_per_sample: Some(bits),
                bitrate_kbps: Some(bps / 1000),
                length_sec: secs(p.duration()),
            }
        }
        Audio::Flac(f) => {
            let p = f.properties();
            StreamInfo {
                sample_rate: Some(p.sample_rate()),
                channels: Some(u32::from(p.channels())),
                bits_per_sample: Some(u32::from(p.bit_depth())),
                bitrate_kbps: Some(u64::from(p.audio_bitrate())),
                length_sec: secs(p.duration()),
            }
        }
        Audio::Mp4(f) => {
            let p = f.properties();
            StreamInfo {
                // mutagen's MP4 info has 0 for whatever the file doesn't say.
                sample_rate: Some(p.sample_rate().unwrap_or(0)),
                channels: Some(p.channels().map(u32::from).unwrap_or(0)),
                bits_per_sample: Some(p.bit_depth().map(u32::from).unwrap_or(0)),
                bitrate_kbps: Some(p.audio_bitrate().map(u64::from).unwrap_or(0)),
                length_sec: secs(p.duration()),
            }
        }
        Audio::Vorbis(f) => {
            let p = f.properties();
            // mutagen's choice between the nominal, maximum and minimum bitrate.
            let (max, nominal, min) = (
                p.bitrate_max().max(0),
                p.bitrate_nominal().max(0),
                p.bitrate_min().max(0),
            );
            let bps = if nominal == 0 {
                (max + min) / 2
            } else if max != 0 && max < nominal {
                max
            } else if min > nominal {
                min
            } else {
                nominal
            };
            StreamInfo {
                sample_rate: Some(p.sample_rate()),
                channels: Some(u32::from(p.channels())),
                bits_per_sample: None,
                bitrate_kbps: Some(bps as u64 / 1000),
                length_sec: secs(p.duration()),
            }
        }
        Audio::Opus(f, mutagen) => {
            let p = f.properties();
            let (length_sec, bps) = mutagen.unwrap_or((
                p.duration().as_secs_f64(),
                u64::from(p.audio_bitrate()) * 1000,
            ));
            StreamInfo {
                // mutagen has no sample rate for Opus, so the player assumes 44.1 kHz.
                sample_rate: None,
                channels: Some(u32::from(p.channels())),
                bits_per_sample: None,
                bitrate_kbps: Some(bps / 1000),
                length_sec,
            }
        }
    }
}

static TIMESTAMP: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\[\d{1,2}:\d{2}").unwrap());
static NUMBER_PAIR: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\((\d+),[\s\x1c-\x1f]*(\d+)\)$").unwrap());

/// M4A stores track and disc as a pair, which reads as '(5, 12)': show it as '5/12'.
fn number_pair(text: &str) -> String {
    let Some(caps) = NUMBER_PAIR.captures(text) else {
        return text.to_owned();
    };
    let parse = |s: &str| -> u128 { py::ascii_digits(s).parse().unwrap_or(u128::MAX) };
    let (number, total) = (parse(&caps[1]), parse(&caps[2]));
    if number == 0 {
        "-".into()
    } else if total != 0 {
        format!("{number}/{total}")
    } else {
        number.to_string()
    }
}

/// Picture and other binary frames: never text.
fn is_binary_tag(key: &str) -> bool {
    [
        "apic",
        "covr",
        "metadata_block_picture",
        "geob",
        "priv",
        "mcdi",
    ]
    .iter()
    .any(|p| key.starts_with(p))
}

/// Audio details, tags, lyrics and cover of a song (`extract_metadata_and_cover`).
///
/// Never fails: whatever can't be read keeps its default ("Unknown Artist", "-"...).
/// The cover comes from the tags, or else from a `cover.jpg`, `cover.png`, `folder.jpg`
/// or `folder.png` next to the song.
pub fn extract_metadata_and_cover(file_path: &Path) -> TrackMetadata {
    let mut info = TrackInfo::defaults(file_path);
    let Ok(audio) = open_audio(file_path) else {
        return TrackMetadata {
            lyrics: None,
            cover: None,
            info,
        };
    };

    // 1. Technical stream information
    let ext = lower_ext(file_path);
    let s = stream_info(&audio);
    let sr = s.sample_rate.unwrap_or(44100);
    info.sample_rate_khz = format!("{:.1}", f64::from(sr) / 1000.0)
        .parse()
        .unwrap_or(0.0);
    info.channels = s.channels.unwrap_or(2);
    let mut bps = s.bits_per_sample.unwrap_or(0);
    if bps == 0 {
        bps = if [".mp3", ".ogg", ".m4a"].contains(&ext.as_str()) {
            16
        } else {
            24
        };
    }
    info.bits_per_sample = bps;
    match s.bitrate_kbps {
        Some(kbps) if kbps > 0 => info.bitrate_kbps = kbps,
        _ => {
            if s.length_sec > 0.0 {
                let bytes = std::fs::metadata(file_path).map(|m| m.len()).unwrap_or(0);
                info.bitrate_kbps = ((bytes as f64 * 8.0) / (s.length_sec * 1000.0)) as u64;
            }
        }
    }
    let length_sec = s.length_sec as u64;
    info.duration_sec = length_sec;
    info.duration_str = format!("{}:{:02}", length_sec / 60, length_sec % 60);
    if bps >= 24 || sr >= 48000 || info.bitrate_kbps >= 1000 {
        info.is_hd = true;
    }
    info.mime_type = match ext.as_str() {
        ".flac" => "audio/flac",
        ".mp3" => "audio/mpeg",
        ".m4a" => "audio/mp4",
        ".ogg" => "audio/ogg",
        ".opus" => "audio/opus",
        ".wav" => "audio/wav",
        _ => "audio/unknown",
    }
    .into();

    // 2. Raw tag pairs, without pictures and other binary frames: turning their bytes
    // into text could feed a whole image to the lyrics scan below.
    let tag_pairs: Vec<(String, String)> = mutagen_items(&audio)
        .map(|(_, items)| items)
        .unwrap_or_default()
        .into_iter()
        .map(|i| (i.key.to_lowercase(), i))
        .filter(|(k, _)| !is_binary_tag(k))
        .map(|(k, i)| (k, i.text()))
        .collect();

    let find = |keys: &[&str]| -> Option<String> {
        for key in keys {
            for (k, v) in &tag_pairs {
                if k == key {
                    let clean = py::strip(v);
                    if !clean.is_empty() {
                        return Some(clean.to_owned());
                    }
                }
            }
        }
        None
    };

    if let Some(t) = find(&["title", "tit2", "\u{a9}nam"]) {
        info.title = t;
        info.title_from_tags = true;
    }
    if let Some(v) = find(&["artist", "tpe1", "\u{a9}art", "performer"]) {
        info.artist = v;
    }
    if let Some(v) = find(&["album", "talb", "\u{a9}alb"]) {
        info.album = v;
    }
    if let Some(v) = find(&["albumartist", "album artist", "tpe2", "aart"]) {
        info.album_artist = v;
    }
    if let Some(v) = find(&["tracknumber", "trck", "trkn"]) {
        info.track_number = number_pair(&v);
    }
    if let Some(v) = find(&["discnumber", "tpos", "disk"]) {
        info.disc_number = number_pair(&v);
    }
    if let Some(v) = find(&["genre", "tcon", "\u{a9}gen"]) {
        info.genre = v;
    }
    if let Some(v) = find(&["date", "year", "tdrc", "tyer", "\u{a9}day"]) {
        info.year = v.chars().take(4).collect();
    }

    // 3. Lyrics: first any tag that holds timestamps, else the usual lyrics fields.
    // USLT is where MP3 (ID3) keeps unsynced lyrics, under a key like "USLT::eng".
    let mut lyrics = tag_pairs
        .iter()
        .find(|(_, text)| TIMESTAMP.is_match(text))
        .map(|(_, text)| text.clone());
    if lyrics.is_none() {
        lyrics = tag_pairs
            .iter()
            .find(|(k, text)| {
                (k.contains("lyric")
                    || k.contains("\u{a9}lyr")
                    || k.contains("unsynced")
                    || k.starts_with("uslt"))
                    && !py::strip(text).is_empty()
            })
            .map(|(_, text)| text.clone());
    }

    // 4. Cover
    let mut cover = embedded_pictures(&audio).into_iter().find_map(load_cover);
    if cover.is_none() {
        let folder = file_path.parent().unwrap_or(Path::new(""));
        for name in ["cover.jpg", "cover.png", "folder.jpg", "folder.png"] {
            let path = folder.join(name);
            if path.exists()
                && let Some(c) = std::fs::read(&path).ok().and_then(load_cover)
            {
                cover = Some(c);
                break;
            }
        }
    }

    TrackMetadata {
        lyrics,
        cover,
        info,
    }
}

// ---------------------------------------------------------------------------------------
// Writing lyrics into a song (write_embedded_lyrics)
// ---------------------------------------------------------------------------------------

/// Why lyrics couldn't be written into a song. The original song is unchanged.
#[derive(Debug)]
pub enum EmbedError {
    Unsupported,
    ReadBack,
    Io(io::Error),
    /// Lofty couldn't read or write the tags; its message.
    Tag(String),
}

impl fmt::Display for EmbedError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unsupported => f.write_str("This audio format isn't supported for embedding."),
            Self::ReadBack => f.write_str(
                "The updated file couldn't be read back, so the original was left unchanged.",
            ),
            Self::Io(e) => write!(f, "{e}"),
            Self::Tag(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for EmbedError {}

impl From<io::Error> for EmbedError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

fn embed_error(e: impl fmt::Display) -> EmbedError {
    EmbedError::Tag(e.to_string())
}

impl From<OpenError> for EmbedError {
    fn from(e: OpenError) -> Self {
        match e {
            OpenError::Unsupported => Self::Unsupported,
            OpenError::Io(e) => Self::Io(e),
            OpenError::Tag(e) => Self::Tag(e),
        }
    }
}

/// ID3: every USLT goes, and one in UTF-8 with language `eng` and no description comes in.
fn set_id3_lyrics(tag: &mut Id3v2Tag, text: &str) {
    tag.retain(|f| f.id_str() != "USLT");
    tag.insert(Frame::UnsynchronizedText(UnsynchronizedTextFrame::new(
        TextEncoding::UTF8,
        *b"eng",
        String::new(),
        text.to_owned(),
    )));
}

/// Vorbis comments: LYRICS, and no UNSYNCEDLYRICS to compete with it.
fn set_vorbis_lyrics(tag: &mut VorbisComments, text: &str) {
    let items: Vec<(String, String)> = tag.take_items().collect();
    for (key, value) in items {
        if !key.eq_ignore_ascii_case("UNSYNCEDLYRICS") {
            tag.push(key, value);
        }
    }
    tag.insert("LYRICS".to_owned(), text.to_owned());
}

/// Writes lyrics into the song's tags (`write_embedded_lyrics`).
///
/// MP4 gets `©lyr`; ID3 (MP3, WAV) gets one `USLT`, saved as ID3v2.4 like mutagen does;
/// Vorbis comments (FLAC, OGG, Opus) get `LYRICS`. Works on a copy in the same folder
/// and swaps it in only after the copy reads back fine, so a failure can never leave a
/// half-written song behind.
pub fn write_embedded_lyrics(path: &Path, text: &str) -> Result<(), EmbedError> {
    let dir = match path.parent() {
        Some(d) if !d.as_os_str().is_empty() => d.to_path_buf(),
        _ => PathBuf::from("."),
    };
    let suffix = path
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()))
        .unwrap_or_default();
    // Closed right away: copying onto a file that is still open fails on Windows. The
    // path is deleted when `tmp` is dropped, unless it was swapped in.
    let tmp = tempfile::Builder::new()
        .prefix(".embedding_")
        .suffix(&suffix)
        .tempfile_in(&dir)?
        .into_temp_path();
    std::fs::copy(path, &tmp)?;

    let options = WriteOptions::default();
    match open_audio(&tmp)? {
        Audio::Mp4(mut f) => {
            if f.ilst().is_none() {
                f.set_ilst(Ilst::default());
            }
            let ilst = f.ilst_mut().expect("just added");
            let ident = AtomIdent::Fourcc(*b"\xa9lyr");
            ilst.replace_atom(Atom::new(ident, AtomData::UTF8(text.to_owned())));
            ilst.save_to_path(&tmp, options).map_err(embed_error)?;
        }
        Audio::Mpeg(mut f, _) => {
            if f.id3v2().is_none() {
                f.set_id3v2(Id3v2Tag::new());
            }
            let tag = f.id3v2_mut().expect("just added");
            set_id3_lyrics(tag, text);
            tag.save_to_path(&tmp, options).map_err(embed_error)?;
        }
        Audio::Wav(mut f) => {
            if f.id3v2().is_none() {
                f.set_id3v2(Id3v2Tag::new());
            }
            let tag = f.id3v2_mut().expect("just added");
            set_id3_lyrics(tag, text);
            tag.save_to_path(&tmp, options).map_err(embed_error)?;
        }
        Audio::Flac(mut f) => {
            if f.vorbis_comments().is_none() {
                f.set_vorbis_comments(VorbisComments::default());
            }
            set_vorbis_lyrics(f.vorbis_comments_mut().expect("just added"), text);
            // Saved as a whole file, so the FLAC picture blocks are written back too.
            f.save_to_path(&tmp, options).map_err(embed_error)?;
        }
        Audio::Vorbis(mut f) => {
            set_vorbis_lyrics(f.vorbis_comments_mut(), text);
            f.vorbis_comments()
                .save_to_path(&tmp, options)
                .map_err(embed_error)?;
        }
        Audio::Opus(mut f, _) => {
            set_vorbis_lyrics(f.vorbis_comments_mut(), text);
            f.vorbis_comments()
                .save_to_path(&tmp, options)
                .map_err(embed_error)?;
        }
    }

    if open_audio(&tmp).is_err() {
        return Err(EmbedError::ReadBack);
    }
    tmp.persist(path).map_err(|e| EmbedError::Io(e.error))?;
    Ok(())
}
