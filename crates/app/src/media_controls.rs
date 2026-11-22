//! System media controls: what's playing in the Windows volume flyout and lock screen, and
//! media keys / flyout buttons routed back to the player. A no-op on other platforms for now.

/// A button the viewer pressed outside the app.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MediaCommand {
    Play,
    Pause,
    Toggle,
    Next,
    Previous,
    Stop,
    /// Seek by this many seconds (negative for back).
    SeekBy(f64),
    /// Seek to this position in seconds.
    SeekTo(f64),
}

/// Track details shown by the system.
pub struct NowPlayingInfo<'a> {
    pub title: &'a str,
    pub subtitle: Option<&'a str>,
    /// Local image file for the thumbnail.
    pub artwork: Option<&'a str>,
    pub duration: f64,
}

#[cfg(windows)]
mod imp {
    use std::time::Duration;

    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use souvlaki::{
        MediaControlEvent, MediaControls, MediaMetadata, MediaPlayback, MediaPosition,
        PlatformConfig, SeekDirection,
    };

    use super::{MediaCommand, NowPlayingInfo};

    /// Seek step for the flyout's fast-forward and rewind buttons.
    const SEEK_STEP: f64 = 10.;

    pub struct NowPlaying {
        controls: MediaControls,
    }

    impl NowPlaying {
        pub fn new(
            window: &impl HasWindowHandle,
            commands: async_channel::Sender<MediaCommand>,
        ) -> Option<Self> {
            let hwnd = match window.window_handle().map(|h| h.as_raw()) {
                Ok(RawWindowHandle::Win32(handle)) => handle.hwnd.get() as *mut std::ffi::c_void,
                _ => return None,
            };
            let config = PlatformConfig {
                display_name: "Miru",
                dbus_name: "miru",
                hwnd: Some(hwnd),
            };
            let mut controls = MediaControls::new(config)
                .inspect_err(|e| tracing::warn!("media controls unavailable: {e:?}"))
                .ok()?;
            let attached = controls.attach(move |event| {
                let command = match event {
                    MediaControlEvent::Play => MediaCommand::Play,
                    MediaControlEvent::Pause => MediaCommand::Pause,
                    MediaControlEvent::Toggle => MediaCommand::Toggle,
                    MediaControlEvent::Next => MediaCommand::Next,
                    MediaControlEvent::Previous => MediaCommand::Previous,
                    MediaControlEvent::Stop => MediaCommand::Stop,
                    MediaControlEvent::Seek(SeekDirection::Forward) => {
                        MediaCommand::SeekBy(SEEK_STEP)
                    }
                    MediaControlEvent::Seek(SeekDirection::Backward) => {
                        MediaCommand::SeekBy(-SEEK_STEP)
                    }
                    MediaControlEvent::SeekBy(direction, by) => {
                        let by = by.as_secs_f64();
                        MediaCommand::SeekBy(match direction {
                            SeekDirection::Forward => by,
                            SeekDirection::Backward => -by,
                        })
                    }
                    MediaControlEvent::SetPosition(MediaPosition(at)) => {
                        MediaCommand::SeekTo(at.as_secs_f64())
                    }
                    _ => return,
                };
                // Closed means the player is gone; nothing left to control.
                let _ = commands.try_send(command);
            });
            if let Err(e) = attached {
                tracing::warn!("cannot attach media controls: {e:?}");
                return None;
            }
            Some(Self { controls })
        }

        pub fn set_info(&mut self, info: &NowPlayingInfo) {
            let cover = info.artwork.map(|path| format!("file://{path}"));
            let metadata = MediaMetadata {
                title: Some(info.title),
                artist: info.subtitle,
                album: None,
                cover_url: cover.as_deref(),
                duration: (info.duration > 0.).then(|| Duration::from_secs_f64(info.duration)),
            };
            if let Err(e) = self.controls.set_metadata(metadata) {
                tracing::warn!("cannot update media info: {e:?}");
            }
        }

        pub fn set_state(&mut self, paused: bool, position: f64) {
            let progress = Some(MediaPosition(Duration::from_secs_f64(position.max(0.))));
            let playback = if paused {
                MediaPlayback::Paused { progress }
            } else {
                MediaPlayback::Playing { progress }
            };
            if let Err(e) = self.controls.set_playback(playback) {
                tracing::warn!("cannot update media state: {e:?}");
            }
        }
    }

    impl Drop for NowPlaying {
        fn drop(&mut self) {
            // Leave nothing behind in the flyout once the player closes.
            self.controls.set_playback(MediaPlayback::Stopped).ok();
            self.controls.detach().ok();
        }
    }
}

#[cfg(not(windows))]
mod imp {
    use super::{MediaCommand, NowPlayingInfo};

    pub struct NowPlaying;

    impl NowPlaying {
        pub fn new<W>(_: &W, _: async_channel::Sender<MediaCommand>) -> Option<Self> {
            None
        }
        pub fn set_info(&mut self, _: &NowPlayingInfo) {}
        pub fn set_state(&mut self, _: bool, _: f64) {}
    }
}

pub use imp::NowPlaying;
