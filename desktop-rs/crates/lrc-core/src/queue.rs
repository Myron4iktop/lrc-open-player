//! The playlist and its playing order: shuffle, repeat, next and previous.
//!
//! Ports the queue logic of the main window: `_set_playlist`, `_rebuild_order`,
//! `_start_index`, `play_index`, `_has_next`, `play_next`, `play_previous`,
//! `set_shuffle`, `cycle_repeat` and the order bookkeeping in `load_track_by_index`.
//! The window asks this what to play and then plays it.

use std::path::{Path, PathBuf};

use crate::session::same_path;

/// Repeat mode (`repeat_mode`): off, the current track, or the whole album.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Repeat {
    #[default]
    Off,
    Track,
    Album,
}

impl Repeat {
    /// `REPEAT_CYCLE`: off → track → album → off.
    pub fn next(self) -> Self {
        match self {
            Repeat::Off => Repeat::Track,
            Repeat::Track => Repeat::Album,
            Repeat::Album => Repeat::Off,
        }
    }

    /// The key the settings use: "off", "track" or "album".
    pub fn key(self) -> &'static str {
        match self {
            Repeat::Off => "off",
            Repeat::Track => "track",
            Repeat::Album => "album",
        }
    }

    /// From a settings key; anything unknown is off (`set_repeat`).
    pub fn from_key(key: &str) -> Self {
        match key {
            "track" => Repeat::Track,
            "album" => Repeat::Album,
            _ => Repeat::Off,
        }
    }

    /// The repeat button's tooltip (`REPEAT_TIPS`).
    pub fn tooltip(self) -> &'static str {
        match self {
            Repeat::Off => "Repeat: off   ( L )",
            Repeat::Track => "Repeat: this track   ( L )",
            Repeat::Album => "Repeat: whole album   ( L )",
        }
    }
}

/// Where "previous" goes (`play_previous`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Previous {
    /// Back to the start of the current song.
    Restart,
    /// Play this playlist index.
    Play(usize),
}

/// What the playlist was built from (`source_kind` and `source_path`).
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub enum Source {
    #[default]
    None,
    /// One song.
    File(PathBuf),
    /// Several songs dropped at once, or a playlist that songs were added to; the path
    /// is the first one's folder (the album folder, for an album that songs were added to).
    Files(PathBuf),
    /// An album folder.
    Folder(PathBuf),
}

/// The playlist, the current song, and the order songs play in.
#[derive(Debug)]
pub struct Queue {
    playlist: Vec<PathBuf>,
    source: Source,
    current: Option<usize>,
    /// Playlist indices in playing order: album order, or shuffled.
    order: Vec<usize>,
    /// Where the current song is in `order`.
    order_pos: Option<usize>,
    shuffle: bool,
    repeat: Repeat,
    rng: fastrand::Rng,
}

impl Default for Queue {
    fn default() -> Self {
        Self::new()
    }
}

impl Queue {
    pub fn new() -> Self {
        Self::with_rng(fastrand::Rng::new())
    }

    /// A queue that shuffles with `rng`, so tests are repeatable.
    pub fn with_rng(rng: fastrand::Rng) -> Self {
        Queue {
            playlist: Vec::new(),
            source: Source::None,
            current: None,
            order: Vec::new(),
            order_pos: None,
            shuffle: false,
            repeat: Repeat::Off,
            rng,
        }
    }

    pub fn playlist(&self) -> &[PathBuf] {
        &self.playlist
    }

    pub fn source(&self) -> &Source {
        &self.source
    }

    pub fn is_empty(&self) -> bool {
        self.playlist.is_empty()
    }

    /// The playlist index of the current song.
    pub fn current(&self) -> Option<usize> {
        self.current
    }

    pub fn current_path(&self) -> Option<&PathBuf> {
        self.current.and_then(|i| self.playlist.get(i))
    }

    pub fn order(&self) -> &[usize] {
        &self.order
    }

    pub fn shuffle(&self) -> bool {
        self.shuffle
    }

    pub fn repeat(&self) -> Repeat {
        self.repeat
    }

    /// A new playlist, with nothing playing yet (`_set_playlist`).
    pub fn set_playlist(&mut self, files: Vec<PathBuf>, source: Source) {
        self.playlist = files;
        self.source = source;
        self.current = None;
        self.rebuild_order(None);
    }

    /// Adds songs to the end of the playlist without disturbing the song that plays
    /// (not in the Python app). Songs already in the playlist are left out, as are
    /// repeats among `files`. Returns how many were added.
    ///
    /// The playlist is then a list of songs rather than one folder or one song, so its
    /// source becomes [`Source::Files`] and the whole list is remembered. Under shuffle
    /// each new song lands somewhere after the current one, so it is still heard in
    /// this round.
    pub fn append(&mut self, files: Vec<PathBuf>) -> usize {
        let old = self.playlist.len();
        for file in files {
            if !self.playlist.iter().any(|p| same_path(p, &file)) {
                self.playlist.push(file);
            }
        }
        let added = self.playlist.len() - old;
        if added == 0 {
            return 0;
        }
        let folder_of = |p: &Path| p.parent().map(Path::to_path_buf).unwrap_or_default();
        self.source = match std::mem::take(&mut self.source) {
            Source::None => Source::Files(folder_of(&self.playlist[0])),
            Source::File(song) => Source::Files(folder_of(&song)),
            Source::Files(folder) | Source::Folder(folder) => Source::Files(folder),
        };
        for index in old..self.playlist.len() {
            if self.shuffle {
                let after = self.order_pos.map_or(0, |p| p + 1).min(self.order.len());
                let at = self.rng.usize(after..=self.order.len());
                self.order.insert(at, index);
            } else {
                self.order.push(index);
            }
        }
        added
    }

    /// Playing order: album order, or a shuffled copy that starts with `first`
    /// (`_rebuild_order`).
    fn rebuild_order(&mut self, first: Option<usize>) {
        let n = self.playlist.len();
        if self.shuffle {
            let mut rest: Vec<usize> = (0..n).filter(|&i| Some(i) != first).collect();
            self.rng.shuffle(&mut rest);
            self.order = first.filter(|&f| f < n).into_iter().chain(rest).collect();
        } else {
            self.order = (0..n).collect();
        }
        self.order_pos = self
            .current
            .filter(|&c| c < n)
            .and_then(|c| self.order.iter().position(|&i| i == c));
    }

    /// Where a freshly opened playlist begins: a random song when shuffling
    /// (`_start_index`).
    pub fn start_index(&self) -> usize {
        self.order.first().copied().unwrap_or(0)
    }

    /// Makes `index` the current song, as `load_track_by_index` does before it plays.
    /// Returns false for an index outside the playlist, which changes nothing.
    pub fn set_current(&mut self, index: usize) -> bool {
        if index >= self.playlist.len() {
            return false;
        }
        self.current = Some(index);
        match self.order.iter().position(|&i| i == index) {
            Some(pos) => self.order_pos = Some(pos),
            None => self.rebuild_order(Some(index)),
        }
        true
    }

    /// A song the user picked (`play_index`): under shuffle, the rest of the album is
    /// reshuffled to come after it. Call [`set_current`](Self::set_current) next.
    pub fn pick(&mut self, index: usize) {
        if self.shuffle {
            self.rebuild_order(Some(index));
        }
    }

    /// True if "next" has somewhere to go (`_has_next`).
    pub fn has_next(&self) -> bool {
        !self.order.is_empty()
            && (self.next_pos() < self.order.len() || self.repeat == Repeat::Album)
    }

    fn next_pos(&self) -> usize {
        self.order_pos.map_or(0, |p| p + 1)
    }

    /// The song "next" plays, or `None` to stay (`play_next`).
    ///
    /// At the end of the order with repeat album, the album starts over; under shuffle,
    /// in a new order where the song that just played lands in the second half, so it
    /// never plays twice in a row.
    pub fn next_track(&mut self) -> Option<usize> {
        if self.playlist.is_empty() {
            return None;
        }
        let pos = self.next_pos();
        if pos < self.order.len() {
            return Some(self.order[pos]);
        }
        if self.repeat != Repeat::Album {
            return None;
        }
        if self.shuffle && self.playlist.len() > 1 {
            // The Python app reads `current_track_idx`, which is -1 before anything played;
            // `range(n)` then keeps every song and `insert` puts -1 in the order. That
            // can't happen from the window (next needs a song), so the Rust queue only
            // reshuffles around a real current song.
            if let Some(cur) = self.current {
                let mut rest: Vec<usize> = (0..self.playlist.len()).filter(|&i| i != cur).collect();
                self.rng.shuffle(&mut rest);
                let at = self.rng.usize(rest.len() / 2 + 1..=rest.len());
                rest.insert(at, cur);
                self.order = rest;
            }
        }
        self.order.first().copied()
    }

    /// Where "previous" goes (`play_previous`): after 3 seconds, back to the start of
    /// the song; otherwise the song before it, or the last one when repeating the
    /// album, or the start again. `None` with an empty playlist.
    pub fn previous_track(&self, position_ms: u64) -> Option<Previous> {
        if self.playlist.is_empty() {
            return None;
        }
        if position_ms > 3000 {
            return Some(Previous::Restart);
        }
        match self.order_pos {
            Some(pos) if pos > 0 => Some(Previous::Play(self.order[pos - 1])),
            _ if self.repeat == Repeat::Album && self.order.len() > 1 => {
                Some(Previous::Play(*self.order.last().unwrap()))
            }
            _ => Some(Previous::Restart),
        }
    }

    /// Shuffle on or off (`set_shuffle`). Turning it on keeps the current song first.
    pub fn set_shuffle(&mut self, on: bool) {
        self.shuffle = on;
        let cur = self.current.filter(|&c| c < self.playlist.len());
        self.rebuild_order(cur);
    }

    pub fn set_repeat(&mut self, repeat: Repeat) {
        self.repeat = repeat;
    }

    /// The next repeat mode (`cycle_repeat`), now set.
    pub fn cycle_repeat(&mut self) -> Repeat {
        self.repeat = self.repeat.next();
        self.repeat
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn queue(n: usize) -> Queue {
        let mut q = Queue::with_rng(fastrand::Rng::with_seed(7));
        q.set_playlist(
            (0..n).map(|i| PathBuf::from(format!("{i}.mp3"))).collect(),
            Source::Folder("album".into()),
        );
        q
    }

    fn play(q: &mut Queue, i: usize) {
        assert!(q.set_current(i));
    }

    #[test]
    fn album_order_plays_through_and_stops() {
        let mut q = queue(3);
        assert_eq!(q.start_index(), 0);
        play(&mut q, 0);
        assert!(q.has_next());
        assert_eq!(q.next_track(), Some(1));
        play(&mut q, 1);
        play(&mut q, 2);
        assert!(!q.has_next());
        assert_eq!(q.next_track(), None);
    }

    #[test]
    fn repeat_album_wraps_both_ways() {
        let mut q = queue(3);
        q.set_repeat(Repeat::Album);
        play(&mut q, 2);
        assert!(q.has_next());
        assert_eq!(q.next_track(), Some(0));
        play(&mut q, 0);
        assert_eq!(q.previous_track(0), Some(Previous::Play(2)));
    }

    #[test]
    fn previous_rules() {
        let mut q = queue(3);
        assert_eq!(q.previous_track(0), Some(Previous::Restart));
        play(&mut q, 1);
        assert_eq!(q.previous_track(3001), Some(Previous::Restart));
        assert_eq!(q.previous_track(3000), Some(Previous::Play(0)));
        play(&mut q, 0);
        assert_eq!(q.previous_track(0), Some(Previous::Restart));
        // One song repeating as an album: nothing to go back to.
        let mut one = queue(1);
        one.set_repeat(Repeat::Album);
        play(&mut one, 0);
        assert_eq!(one.previous_track(0), Some(Previous::Restart));
        assert_eq!(Queue::new().previous_track(0), None);
    }

    #[test]
    fn shuffle_keeps_the_current_song_first() {
        let mut q = queue(20);
        play(&mut q, 5);
        q.set_shuffle(true);
        assert_eq!(q.order()[0], 5);
        let mut sorted = q.order().to_vec();
        sorted.sort();
        assert_eq!(sorted, (0..20).collect::<Vec<_>>());
        // Off again: album order, still on the same song.
        q.set_shuffle(false);
        assert_eq!(q.order(), (0..20).collect::<Vec<_>>());
        assert_eq!(q.next_track(), Some(6));
    }

    #[test]
    fn picking_a_song_under_shuffle_reshuffles_after_it() {
        let mut q = queue(10);
        q.set_shuffle(true);
        q.pick(7);
        play(&mut q, 7);
        assert_eq!(q.order()[0], 7);
        assert!(q.has_next());
    }

    #[test]
    fn new_shuffled_round_never_repeats_the_last_song() {
        for seed in 0..200 {
            let mut q = Queue::with_rng(fastrand::Rng::with_seed(seed));
            q.set_playlist(
                (0..5).map(|i| PathBuf::from(format!("{i}"))).collect(),
                Source::None,
            );
            q.set_shuffle(true);
            q.set_repeat(Repeat::Album);
            let last = *q.order().last().unwrap();
            play(&mut q, last);
            let first = q.next_track().unwrap();
            assert_ne!(first, last, "seed {seed}");
            // The song that just played is in the second half of the new round.
            let at = q.order().iter().position(|&i| i == last).unwrap();
            assert!(at >= 3, "seed {seed}: at {at}");
            assert_eq!(q.order().len(), 5);
        }
    }

    #[test]
    fn two_song_shuffled_round_alternates() {
        let mut q = queue(2);
        q.set_shuffle(true);
        q.set_repeat(Repeat::Album);
        let last = q.order()[1];
        play(&mut q, last);
        let first = q.next_track().unwrap();
        assert_ne!(first, last);
    }

    #[test]
    fn repeat_cycles_and_reads_back() {
        let mut q = queue(1);
        assert_eq!(q.cycle_repeat(), Repeat::Track);
        assert_eq!(q.cycle_repeat(), Repeat::Album);
        assert_eq!(q.cycle_repeat(), Repeat::Off);
        for r in [Repeat::Off, Repeat::Track, Repeat::Album] {
            assert_eq!(Repeat::from_key(r.key()), r);
        }
        assert_eq!(Repeat::from_key("bogus"), Repeat::Off);
    }

    fn songs(names: &[&str]) -> Vec<PathBuf> {
        names.iter().map(PathBuf::from).collect()
    }

    #[test]
    fn added_songs_play_after_the_album() {
        let mut q = queue(2);
        play(&mut q, 1);
        assert!(!q.has_next());
        assert_eq!(q.append(songs(&["/x/a.flac", "/y/b.flac"])), 2);
        assert_eq!(q.playlist().len(), 4);
        // Still on the same song, which now has one after it.
        assert_eq!(q.current(), Some(1));
        assert!(q.has_next());
        assert_eq!(q.next_track(), Some(2));
        play(&mut q, 2);
        assert_eq!(q.next_track(), Some(3));
        assert_eq!(q.previous_track(0), Some(Previous::Play(1)));
        // An album with songs added is a list of songs, kept under the album's folder.
        assert_eq!(q.source(), &Source::Files("album".into()));
    }

    #[test]
    fn songs_already_in_the_playlist_are_not_added_twice() {
        let mut q = queue(2);
        assert_eq!(q.append(songs(&["1.mp3", "./0.mp3"])), 0);
        // Nothing new: the playlist is still the album it was.
        assert_eq!(q.source(), &Source::Folder("album".into()));
        assert_eq!(q.append(songs(&["new.mp3", "1.mp3", "new.mp3"])), 1);
        assert_eq!(q.playlist().len(), 3);
        assert_eq!(q.order(), [0, 1, 2]);
    }

    #[test]
    fn adding_to_one_song_or_to_nothing_makes_a_list() {
        let mut q = Queue::new();
        q.set_playlist(songs(&["/m/one.mp3"]), Source::File("/m/one.mp3".into()));
        play(&mut q, 0);
        assert_eq!(q.append(songs(&["/other/two.mp3"])), 1);
        assert_eq!(q.source(), &Source::Files("/m".into()));
        assert_eq!(q.next_track(), Some(1));

        let mut q = Queue::new();
        assert_eq!(q.append(songs(&["/m/a.mp3", "/m/b.mp3"])), 2);
        assert_eq!(q.source(), &Source::Files("/m".into()));
        assert_eq!(q.current(), None);
        assert_eq!(q.start_index(), 0);
    }

    #[test]
    fn songs_added_under_shuffle_come_after_the_current_one() {
        for seed in 0..200 {
            let mut q = Queue::with_rng(fastrand::Rng::with_seed(seed));
            q.set_playlist(
                (0..6).map(|i| PathBuf::from(format!("{i}"))).collect(),
                Source::Folder("album".into()),
            );
            q.set_shuffle(true);
            // Two songs into the shuffled round.
            let (first, second) = (q.order()[0], q.order()[1]);
            play(&mut q, first);
            play(&mut q, second);
            let played = q.order()[..2].to_vec();
            assert_eq!(q.append(songs(&["new-a", "new-b", "new-c"])), 3);
            assert_eq!(&q.order()[..2], played, "seed {seed}");
            let mut sorted = q.order().to_vec();
            sorted.sort();
            assert_eq!(sorted, (0..9).collect::<Vec<_>>(), "seed {seed}");
            // The round goes on from the current song and reaches every new one.
            let mut heard = Vec::new();
            while let Some(next) = q.next_track() {
                heard.push(next);
                play(&mut q, next);
            }
            assert_eq!(heard.len(), 7, "seed {seed}");
            assert!([6, 7, 8].iter().all(|i| heard.contains(i)), "seed {seed}");
        }
    }

    #[test]
    fn songs_added_under_shuffle_before_anything_played() {
        let mut q = queue(3);
        q.set_shuffle(true);
        assert_eq!(q.append(songs(&["new"])), 1);
        let mut sorted = q.order().to_vec();
        sorted.sort();
        assert_eq!(sorted, [0, 1, 2, 3]);
    }

    #[test]
    fn out_of_range_songs_are_ignored() {
        let mut q = queue(2);
        assert!(!q.set_current(2));
        assert_eq!(q.current(), None);
    }
}
