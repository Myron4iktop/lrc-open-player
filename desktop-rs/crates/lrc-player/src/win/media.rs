//! The Windows media flyout, lock screen and media keys (`WindowsMediaControls`), through
//! SystemMediaTransportControls.
//!
//! The plan named the `souvlaki` crate for this. It always enables Next and Previous and
//! can't set the album artist or track number, which the Python app does, so this calls
//! the same WinRT API directly through the `windows` crate. As in the Python app, a WinRT
//! `MediaPlayer` that plays nothing is the way to get the controls in a desktop app.
//!
//! Windows calls the button and seek handlers on a thread-pool thread, so they only pass
//! a [`MediaCommand`] on; the app acts on it on the UI thread.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use image::RgbaImage;
use lrc_core::system::{TimelineThrottle, media_clean, media_track_number, timeline};
use lrc_core::tags::TrackInfo;
use windows::Foundation::{TimeSpan, TypedEventHandler, Uri};
use windows::Media::Playback::MediaPlayer;
use windows::Media::{
    MediaPlaybackStatus, MediaPlaybackType, PlaybackPositionChangeRequestedEventArgs,
    SystemMediaTransportControls, SystemMediaTransportControlsButton,
    SystemMediaTransportControlsButtonPressedEventArgs,
    SystemMediaTransportControlsTimelineProperties,
};
use windows::Storage::Streams::RandomAccessStreamReference;
use windows::core::{HSTRING, Ref};

use crate::audio::State;
use crate::media::MediaCommand;

pub struct MediaControls {
    /// Only there to own the controls; the audio engine plays the music.
    _player: MediaPlayer,
    smtc: SystemMediaTransportControls,
    button_token: i64,
    seek_token: i64,
    throttle: TimelineThrottle,
    /// The private temp folder (`private_temp_dir`), removed when the app exits.
    temp: Option<tempfile::TempDir>,
    cover_file: Option<PathBuf>,
    cover_counter: u32,
    available: bool,
}

impl MediaControls {
    /// Connects to the media flyout. `None` if Windows refuses, and the player then works
    /// without it, as the Python app does.
    pub fn new(on_command: impl Fn(MediaCommand) + Send + Sync + 'static) -> Option<Self> {
        match Self::try_new(on_command) {
            Ok(controls) => Some(controls),
            Err(e) => {
                eprintln!("SMTC initialisation failed: {e}");
                None
            }
        }
    }

    fn try_new(
        on_command: impl Fn(MediaCommand) + Send + Sync + 'static,
    ) -> windows::core::Result<Self> {
        let player = MediaPlayer::new()?;
        player.CommandManager()?.SetIsEnabled(false)?;
        let smtc = player.SystemMediaTransportControls()?;
        smtc.SetIsEnabled(true)?;
        smtc.SetIsPlayEnabled(true)?;
        smtc.SetIsPauseEnabled(true)?;
        smtc.SetIsNextEnabled(false)?;
        smtc.SetIsPreviousEnabled(false)?;
        smtc.SetPlaybackStatus(MediaPlaybackStatus::Closed)?;

        let on_command = std::sync::Arc::new(on_command);
        let send = on_command.clone();
        let button_token = smtc.ButtonPressed(&TypedEventHandler::new(
            move |_: Ref<SystemMediaTransportControls>,
                  args: Ref<SystemMediaTransportControlsButtonPressedEventArgs>| {
                let button = args.ok()?.Button()?;
                use SystemMediaTransportControlsButton as B;
                let command = match button {
                    B::Play => MediaCommand::Play,
                    B::Pause | B::Stop => MediaCommand::Pause,
                    B::Next => MediaCommand::Next,
                    B::Previous => MediaCommand::Previous,
                    _ => return Ok(()),
                };
                send(command);
                Ok(())
            },
        ))?;
        let send = on_command;
        let seek_token = smtc.PlaybackPositionChangeRequested(&TypedEventHandler::new(
            move |_: Ref<SystemMediaTransportControls>,
                  args: Ref<PlaybackPositionChangeRequestedEventArgs>| {
                let position = args.ok()?.RequestedPlaybackPosition()?;
                let ms = (position.Duration.max(0) / 10_000) as u64;
                send(MediaCommand::Seek(ms));
                Ok(())
            },
        ))?;

        let temp = tempfile::Builder::new()
            .prefix("lrcopenplayer-")
            .tempdir()
            .ok();
        Ok(Self {
            _player: player,
            smtc,
            button_token,
            seek_token,
            throttle: TimelineThrottle::default(),
            temp,
            cover_file: None,
            cover_counter: 0,
            available: true,
        })
    }

    /// A new song: its tags, cover and whether Previous and Next can be pressed.
    pub fn set_track(
        &mut self,
        info: &TrackInfo,
        cover: Option<&RgbaImage>,
        has_previous: bool,
        has_next: bool,
    ) {
        if !self.available {
            return;
        }
        if let Err(e) = self.try_set_track(info, cover, has_previous, has_next) {
            eprintln!("SMTC set_track error: {e}");
        }
    }

    fn try_set_track(
        &mut self,
        info: &TrackInfo,
        cover: Option<&RgbaImage>,
        has_previous: bool,
        has_next: bool,
    ) -> windows::core::Result<()> {
        let updater = self.smtc.DisplayUpdater()?;
        updater.SetType(MediaPlaybackType::Music)?;
        let props = updater.MusicProperties()?;
        props.SetTitle(&HSTRING::from(info.title.as_str()))?;
        props.SetArtist(&HSTRING::from(media_clean(&info.artist, "Unknown Artist")))?;
        props.SetAlbumTitle(&HSTRING::from(media_clean(&info.album, "Unknown Album")))?;
        props.SetAlbumArtist(&HSTRING::from(media_clean(
            &info.album_artist,
            "Unknown Album Artist",
        )))?;
        props.SetTrackNumber(media_track_number(&info.track_number))?;

        let previous_cover = self.cover_file.clone();
        let cover_path = self.write_cover(cover);
        match &cover_path {
            Some(path) => {
                let uri = Uri::CreateUri(&HSTRING::from(file_uri(path)))?;
                updater.SetThumbnail(&RandomAccessStreamReference::CreateFromUri(&uri)?)?;
            }
            None => updater.SetThumbnail(None)?,
        }
        updater.Update()?;
        if let Some(old) = previous_cover
            && Some(&old) != cover_path.as_ref()
        {
            let _ = std::fs::remove_file(old);
        }
        self.smtc.SetIsPreviousEnabled(has_previous)?;
        self.smtc.SetIsNextEnabled(has_next)?;
        Ok(())
    }

    /// `set_navigation`: after the playlist, shuffle or repeat changed.
    pub fn set_navigation(&mut self, has_previous: bool, has_next: bool) {
        if !self.available {
            return;
        }
        let result = self
            .smtc
            .SetIsPreviousEnabled(has_previous)
            .and_then(|()| self.smtc.SetIsNextEnabled(has_next));
        if let Err(e) = result {
            eprintln!("SMTC navigation error: {e}");
        }
    }

    pub fn set_playback_state(&mut self, state: State) {
        if !self.available {
            return;
        }
        let status = match state {
            State::Playing => MediaPlaybackStatus::Playing,
            State::Paused => MediaPlaybackStatus::Paused,
            State::Stopped => MediaPlaybackStatus::Stopped,
        };
        if let Err(e) = self.smtc.SetPlaybackStatus(status) {
            eprintln!("SMTC playback state error: {e}");
        }
    }

    /// Position and length. About once a second unless `force` (seek, pause, new song).
    pub fn update_timeline(&mut self, position_ms: u64, duration_ms: u64, force: bool) {
        if !self.available || !self.throttle.ready(Instant::now(), force) {
            return;
        }
        if let Err(e) = self.try_update_timeline(position_ms, duration_ms) {
            eprintln!("SMTC timeline error: {e}");
        }
    }

    fn try_update_timeline(&self, position_ms: u64, duration_ms: u64) -> windows::core::Result<()> {
        let (position_ms, duration_ms) = timeline(position_ms, duration_ms);
        let span = |ms: u64| TimeSpan::from(Duration::from_millis(ms));
        let tl = SystemMediaTransportControlsTimelineProperties::new()?;
        tl.SetStartTime(span(0))?;
        tl.SetMinSeekTime(span(0))?;
        tl.SetEndTime(span(duration_ms))?;
        tl.SetMaxSeekTime(span(duration_ms))?;
        tl.SetPosition(span(position_ms))?;
        self.smtc.UpdateTimelineProperties(&tl)
    }

    /// The app is quitting: the flyout lets go of the player.
    pub fn shutdown(&mut self) {
        if !self.available {
            return;
        }
        self.available = false;
        let _ = self.smtc.RemoveButtonPressed(self.button_token);
        let _ = self
            .smtc
            .RemovePlaybackPositionChangeRequested(self.seek_token);
        let _ = self.smtc.SetPlaybackStatus(MediaPlaybackStatus::Closed);
        let _ = self.smtc.SetIsEnabled(false);
        if let Some(path) = self.cover_file.take() {
            let _ = std::fs::remove_file(path);
        }
        self.temp = None;
    }

    /// `_write_cover`: the flyout loads its picture from a file, so the cover goes into a
    /// fresh PNG, at most 512 × 512, in the private temp folder.
    fn write_cover(&mut self, cover: Option<&RgbaImage>) -> Option<PathBuf> {
        self.cover_file = None;
        let cover = cover?;
        let folder = self.temp.as_ref()?.path();
        let (w, h) = cover.dimensions();
        let scaled = if w > 512 || h > 512 {
            let k = 512.0 / f64::from(w.max(h));
            let size = |v: u32| ((f64::from(v) * k).round() as u32).max(1);
            image::imageops::resize(
                cover,
                size(w),
                size(h),
                image::imageops::FilterType::Lanczos3,
            )
        } else {
            cover.clone()
        };
        self.cover_counter += 1;
        let path = folder.join(format!("cover_{}.png", self.cover_counter));
        match scaled.save_with_format(&path, image::ImageFormat::Png) {
            Ok(()) => {
                self.cover_file = Some(path.clone());
                Some(path)
            }
            Err(e) => {
                eprintln!("SMTC cover write error: {e}");
                None
            }
        }
    }
}

impl Drop for MediaControls {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// `Path.as_uri()`: `file:///C:/Users/…`, with anything unusual percent-encoded.
fn file_uri(path: &Path) -> String {
    let text = path.to_string_lossy().replace('\\', "/");
    let mut out = String::from("file:///");
    for b in text.trim_start_matches('/').bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' | b':' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_uris_like_pathlib() {
        assert_eq!(
            file_uri(Path::new(r"C:\Users\Mý Name\AppData\cover_1.png")),
            "file:///C:/Users/M%C3%BD%20Name/AppData/cover_1.png"
        );
    }
}
