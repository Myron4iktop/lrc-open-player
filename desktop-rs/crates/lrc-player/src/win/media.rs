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

use std::time::{Duration, Instant};

use image::RgbaImage;
use lrc_core::system::{TimelineThrottle, media_clean, media_track_number, timeline};
use lrc_core::tags::TrackInfo;
use windows::Foundation::{TimeSpan, TypedEventHandler};
use windows::Media::Playback::MediaPlayer;
use windows::Media::{
    MediaPlaybackStatus, MediaPlaybackType, PlaybackPositionChangeRequestedEventArgs,
    SystemMediaTransportControls, SystemMediaTransportControlsButton,
    SystemMediaTransportControlsButtonPressedEventArgs,
    SystemMediaTransportControlsTimelineProperties,
};
use windows::Storage::Streams::{IRandomAccessStream, RandomAccessStreamReference};
use windows::Win32::System::WinRT::{BSOS_DEFAULT, CreateRandomAccessStreamOverStream};
use windows::Win32::UI::Shell::SHCreateMemStream;
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

        Ok(Self {
            _player: player,
            smtc,
            button_token,
            seek_token,
            throttle: TimelineThrottle::default(),
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

        // A cover that can't be handed over is left out; the title still shows.
        match cover.and_then(cover_png).map(|png| thumbnail(&png)) {
            Some(Ok(reference)) => updater.SetThumbnail(&reference)?,
            Some(Err(e)) => {
                eprintln!("SMTC cover error: {e}");
                updater.SetThumbnail(None)?;
            }
            None => updater.SetThumbnail(None)?,
        }
        updater.Update()?;
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
    }
}

impl Drop for MediaControls {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// `_write_cover`: the cover as a PNG, at most 512 × 512.
fn cover_png(cover: &RgbaImage) -> Option<Vec<u8>> {
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
    let mut png = std::io::Cursor::new(Vec::new());
    match scaled.write_to(&mut png, image::ImageFormat::Png) {
        Ok(()) => Some(png.into_inner()),
        Err(e) => {
            eprintln!("SMTC cover write error: {e}");
            None
        }
    }
}

/// The picture for the flyout, kept in memory. The Python app writes it to a temp file and
/// passes a `file:` address, and with that the flyout showed no cover here.
fn thumbnail(png: &[u8]) -> windows::core::Result<RandomAccessStreamReference> {
    // SAFETY: `SHCreateMemStream` copies `png`, and the stream it returns is a valid
    // `IStream` for `CreateRandomAccessStreamOverStream`.
    let stream: IRandomAccessStream = unsafe {
        let memory = SHCreateMemStream(Some(png))
            .ok_or_else(|| windows::core::Error::from(windows::Win32::Foundation::E_OUTOFMEMORY))?;
        CreateRandomAccessStreamOverStream(&memory, BSOS_DEFAULT)?
    };
    RandomAccessStreamReference::CreateFromStream(&stream)
}
