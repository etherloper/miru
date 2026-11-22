//! Full-window playback: mpv renders in a child window underneath, GPUI draws controls on top.

use std::cell::Cell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result};
use gpui::{
    Animation, AnimationExt as _, Bounds, ClickEvent, Context, EventEmitter, FocusHandle,
    FontWeight, KeyDownEvent, MouseButton, MouseDownEvent, MouseMoveEvent, Pixels, SharedString,
    Window, WindowControlArea, canvas, div, ease_in_out, linear_color_stop, linear_gradient,
    prelude::*, px, relative, rgba,
};
use miru_player::{Chapter, PlaybackOptions, PlaybackState, Player, Track, TrackKind, format_time};

use crate::theme::{self, glass_round_button, icon};
use crate::video_host::VideoHost;

const CONTROLS_IDLE_TIMEOUT: Duration = Duration::from_millis(1800);
/// Long enough to read as a gentle fade rather than a cut.
const CONTROLS_FADE_MS: u64 = 480;
/// Fade when the pointer leaves the window: the viewer has moved on, so don't linger.
const CONTROLS_LEAVE_FADE_MS: u64 = 140;
const POLL_INTERVAL: Duration = Duration::from_millis(200);
const PROGRESS_SAVE_INTERVAL: Duration = Duration::from_secs(5);
/// How long the volume indicator stays up after a change.
const VOLUME_OSD: Duration = Duration::from_millis(1200);
const VOLUME_STEP: f64 = 5.0;
const EDGE: f32 = 56.;
/// Volume slider width, the drawer that holds it (slider, gaps, level), and its open/close time.
const VOLUME_TRACK_W: f32 = 104.;
const VOLUME_DRAWER_W: f32 = 10. + VOLUME_TRACK_W + 10. + 24. + 14.;
const VOLUME_EXPAND: Duration = Duration::from_millis(260);
/// The black cover stays this long after the video output starts (its first frame may still
/// be on the way), then fades out over `COVER_FADE`.
const COVER_HOLD: Duration = Duration::from_millis(80);
const COVER_FADE: Duration = Duration::from_millis(220);
/// Countdown before the next episode starts automatically.
const UP_NEXT_COUNTDOWN: Duration = Duration::from_secs(10);
/// How often the position shown by the system media controls is refreshed.
const MEDIA_STATE_INTERVAL: Duration = Duration::from_secs(5);

/// What to play and where to start.
#[derive(Clone, PartialEq)]
pub struct PlaybackRequest {
    pub path: String,
    pub title: SharedString,
    /// Library file id, used to save progress. `None` for files opened outside the library.
    pub file_id: Option<i64>,
    pub start: Option<f64>,
}

pub enum PlayerEvent {
    /// The viewer picked an audio or subtitle track (`id` is `None` for subtitles off).
    TrackChosen {
        file_id: i64,
        kind: TrackKind,
        lang: Option<String>,
        id: Option<i64>,
    },
    /// The current episode finished and the next one should start.
    PlayNext(PlaybackRequest),
    Progress {
        file_id: i64,
        position: f64,
        duration: f64,
    },
    Close,
}

/// A running mpv instance and the native window it renders into.
pub struct Session {
    // Field order matters: mpv must shut down before its host window is destroyed.
    player: Player,
    host: VideoHost,
    load_error: Option<SharedString>,
}

impl Session {
    pub fn start(
        request: &PlaybackRequest,
        options: &PlaybackOptions,
        window: &Window,
    ) -> Result<Self> {
        let host = VideoHost::new(window).context("creating video host window")?;
        let player = Player::new(host.wid(), options).context("starting mpv")?;
        let load_error = player
            .load(&request.path, request.start)
            .err()
            .map(|e| e.to_string().into());
        Ok(Self {
            player,
            host,
            load_error,
        })
    }
}

pub struct PlayerView {
    session: Session,
    title: SharedString,
    /// Seconds jumped by the skip buttons and arrow keys (a user setting).
    seek_step: f64,
    file_id: Option<i64>,
    state: PlaybackState,
    focus: FocusHandle,
    last_activity: Instant,
    last_saved: Instant,
    menu: Option<TrackKind>,
    tracks: Vec<Track>,
    /// Channel layout mpv is sending to the audio device, shown in the Audio menu.
    audio_output: Option<String>,
    /// Screen bounds of the seek bar, captured during paint for click-to-seek.
    seek_bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
    /// Screen bounds of the volume slider, for click and drag.
    volume_bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
    /// When the volume last changed, for the brief on-screen indicator.
    volume_changed_at: Instant,
    /// Wheel input turned into 5% volume steps.
    wheel: crate::wheel::WheelSteps,
    chapters: Vec<Chapter>,
    chapters_loaded: bool,
    current_chapter: Option<usize>,
    chapters_open: bool,
    /// Pointer position over the seek bar (0–1), for the scrub handle and time preview.
    seek_hover: Option<f32>,
    /// The episode to play next, and whether to start it automatically.
    up_next: Option<PlaybackRequest>,
    autoplay: bool,
    /// Where the credits start (see `up_next::credits_start`), once the length is known.
    credits_at: Option<f64>,
    /// Autoplay countdown still to run; it only runs while the picture is playing.
    up_next_remaining: Option<Duration>,
    /// Whether the card is showing, and an epoch per appearance (for its entrance).
    up_next_shown: bool,
    up_next_epoch: usize,
    last_tick: Instant,
    /// System media controls (flyout, lock screen, media keys), the artwork shown there, and
    /// whether the track details and the latest play state have been sent.
    now_playing: Option<crate::media_controls::NowPlaying>,
    artwork: Option<String>,
    media_info_sent: bool,
    media_state_sent: Option<(bool, Instant)>,
    /// Whether subtitles are currently raised above the controls.
    subtitles_lifted: bool,
    /// Controls fade: current target, an epoch per change (animation id), and when it changed.
    controls_shown: bool,
    controls_epoch: usize,
    controls_changed: Instant,
    /// Length of the current show/hide fade (shorter when the pointer left the window).
    controls_fade_ms: u64,
    /// Set when the pointer leaves the window, so the hide that follows uses the quick fade.
    left_window: bool,
    /// Pointer over the volume pill: the track thickens and the knob appears.
    volume_hover: bool,
    /// The slider is being dragged; the drawer stays open until the button is released.
    volume_dragging: bool,
    /// The volume drawer is open, with when it last opened or closed (for the animation).
    volume_open: bool,
    volume_open_epoch: usize,
    volume_open_changed: Instant,
    /// Whether the pointer was inside the window at the last render.
    window_hovered: bool,
    /// When mpv's video output came up; the black cover fades out shortly after.
    video_ready_at: Option<Instant>,
    error: Option<SharedString>,
}

impl EventEmitter<PlayerEvent> for PlayerView {}

impl PlayerView {
    pub fn new(
        session: Session,
        request: PlaybackRequest,
        seek_step: f64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let error = session.load_error.clone();
        let focus = cx.focus_handle();
        window.focus(&focus, cx);

        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(POLL_INTERVAL).await;
                if this.update(cx, |view, cx| view.tick(cx)).is_err() {
                    break;
                }
            }
        })
        .detach();

        let (commands, on_command) = async_channel::bounded(8);
        let now_playing = crate::media_controls::NowPlaying::new(window, commands);
        cx.spawn(async move |this, cx| {
            while let Ok(command) = on_command.recv().await {
                if this
                    .update(cx, |view, cx| view.media_command(command, cx))
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();

        Self {
            session,
            title: request.title,
            seek_step,
            file_id: request.file_id,
            state: PlaybackState::default(),
            focus,
            last_activity: Instant::now(),
            last_saved: Instant::now(),
            menu: None,
            tracks: Vec::new(),
            audio_output: None,
            seek_bounds: Rc::default(),
            volume_bounds: Rc::default(),
            volume_changed_at: Instant::now() - Duration::from_secs(60),
            wheel: Default::default(),
            chapters: Vec::new(),
            chapters_loaded: false,
            current_chapter: None,
            chapters_open: false,
            seek_hover: None,
            up_next: None,
            autoplay: false,
            credits_at: None,
            up_next_remaining: None,
            up_next_shown: false,
            up_next_epoch: 0,
            last_tick: Instant::now(),
            now_playing,
            artwork: None,
            media_info_sent: false,
            media_state_sent: None,
            subtitles_lifted: false,
            controls_shown: true,
            controls_epoch: 0,
            controls_changed: Instant::now(),
            controls_fade_ms: CONTROLS_FADE_MS,
            left_window: false,
            volume_hover: false,
            volume_dragging: false,
            volume_open: false,
            volume_open_epoch: 0,
            volume_open_changed: Instant::now() - Duration::from_secs(60),
            window_hovered: window.is_window_hovered(),
            video_ready_at: None,
            error,
        }
    }

    fn tick(&mut self, cx: &mut Context<Self>) {
        let state = self.session.player.state();
        if state.video_ready && self.video_ready_at.is_none() {
            self.video_ready_at = Some(Instant::now());
        }
        let pause_changed = state.paused != self.state.paused;
        let reached_end = state.eof && !self.state.eof;
        if state != self.state {
            self.state = state;
            cx.notify();
        }
        let playing = !state.paused && !state.idle;
        if pause_changed
            || reached_end
            || (playing && self.last_saved.elapsed() >= PROGRESS_SAVE_INTERVAL)
        {
            self.save_progress(cx);
        }
        // Repaint around the idle timeout and while a fade is finishing.
        let fade = Duration::from_millis(self.controls_fade_ms) + POLL_INTERVAL;
        if self.last_activity.elapsed() < CONTROLS_IDLE_TIMEOUT + POLL_INTERVAL
            || self.controls_changed.elapsed() < fade
            || self.volume_changed_at.elapsed() < VOLUME_OSD + fade
        {
            cx.notify();
        }
        // Chapters are known once the file has opened.
        if !self.chapters_loaded && state.duration > 0. {
            self.chapters = self.session.player.chapters();
            self.chapters_loaded = true;
            self.credits_at = crate::up_next::credits_start(state.duration, &self.chapters);
        }
        if !self.chapters.is_empty() {
            self.current_chapter = self.session.player.current_chapter();
        }
        if self.step_up_next(&state, cx) {
            return;
        }
        self.update_now_playing(&state);
        let visible = self.controls_visible();
        if self.subtitles_lifted != visible {
            self.subtitles_lifted = visible;
            if let Err(e) = self.session.player.lift_subtitles(visible) {
                tracing::warn!("cannot reposition subtitles: {e}");
            }
        }
    }

    fn save_progress(&mut self, cx: &mut Context<Self>) {
        self.last_saved = Instant::now();
        // At EOF mpv's time-pos can lag the duration slightly; report the end exactly.
        let position = if self.state.eof {
            self.state.duration
        } else {
            self.state.position
        };
        if let Some(file_id) = self.file_id
            && self.state.duration > 0.0
        {
            cx.emit(PlayerEvent::Progress {
                file_id,
                position,
                duration: self.state.duration,
            });
        }
    }

    fn close(&mut self, cx: &mut Context<Self>) {
        self.state = self.session.player.state();
        if self.up_next_shown {
            self.state.position = self.state.duration;
        }
        self.save_progress(cx);
        cx.emit(PlayerEvent::Close);
    }

    /// Sets the episode that follows this one (and whether to start it automatically).
    pub fn set_up_next(&mut self, next: Option<PlaybackRequest>, autoplay: bool) {
        self.up_next = next;
        self.autoplay = autoplay;
    }

    /// Up next: once the credits start, show the card and run the autoplay countdown (paused
    /// with the picture), then play the next episode. Returns true when it started playing.
    fn step_up_next(&mut self, state: &PlaybackState, cx: &mut Context<Self>) -> bool {
        let elapsed = self.last_tick.elapsed();
        self.last_tick = Instant::now();
        if self.up_next.is_none() {
            return false;
        }
        let in_credits = state.eof
            || self
                .credits_at
                .is_some_and(|at| state.position >= at && state.duration > 0.);
        if !in_credits {
            // Seeking back before the credits re-arms the countdown.
            self.up_next_remaining = None;
        } else if self.autoplay && self.up_next_remaining.is_none() {
            self.up_next_remaining = Some(UP_NEXT_COUNTDOWN);
        }
        let shown = in_credits;
        if shown != self.up_next_shown {
            self.up_next_shown = shown;
            self.up_next_epoch += 1;
            cx.notify();
        }
        if let Some(remaining) = self.up_next_remaining {
            if !state.paused || state.eof {
                let left = remaining.saturating_sub(elapsed);
                if left.is_zero() {
                    self.play_next(cx);
                    return true;
                }
                self.up_next_remaining = Some(left);
            }
            cx.notify();
        }
        false
    }

    /// The thumbnail for the system media controls (poster or episode still).
    pub fn set_artwork(&mut self, artwork: Option<String>) {
        self.artwork = artwork;
        self.media_info_sent = false;
    }

    /// Keeps the system media controls in step: details once the length is known, the play
    /// state when it changes and the position every few seconds.
    fn update_now_playing(&mut self, state: &PlaybackState) {
        let Some(now_playing) = &mut self.now_playing else {
            return;
        };
        if !self.media_info_sent && state.duration > 0. {
            now_playing.set_info(&crate::media_controls::NowPlayingInfo {
                title: &self.title,
                subtitle: None,
                artwork: self.artwork.as_deref(),
                duration: state.duration,
            });
            self.media_info_sent = true;
        }
        let paused = state.paused || state.eof;
        let stale = self
            .media_state_sent
            .is_none_or(|(was, at)| was != paused || at.elapsed() >= MEDIA_STATE_INTERVAL);
        if stale {
            now_playing.set_state(paused, state.position);
            self.media_state_sent = Some((paused, Instant::now()));
        }
    }

    /// A media key or flyout button.
    fn media_command(
        &mut self,
        command: crate::media_controls::MediaCommand,
        cx: &mut Context<Self>,
    ) {
        use crate::media_controls::MediaCommand;
        let paused = self.state.paused;
        match command {
            MediaCommand::Play if paused => self.run(|p| p.toggle_pause(), cx),
            MediaCommand::Pause if !paused => self.run(|p| p.toggle_pause(), cx),
            MediaCommand::Play | MediaCommand::Pause => {}
            MediaCommand::Toggle => self.run(|p| p.toggle_pause(), cx),
            MediaCommand::Next => self.play_next(cx),
            MediaCommand::Previous => self.run(|p| p.seek_absolute(0.), cx),
            MediaCommand::Stop => self.close(cx),
            MediaCommand::SeekBy(by) => self.run(|p| p.seek_relative(by), cx),
            MediaCommand::SeekTo(at) => self.run(|p| p.seek_absolute(at), cx),
        }
        // Report the new state promptly rather than at the next interval.
        self.media_state_sent = None;
    }

    fn play_next(&mut self, cx: &mut Context<Self>) {
        self.up_next_remaining = None;
        if let Some(next) = self.up_next.clone() {
            self.state = self.session.player.state();
            // Moving on from the credits counts as finishing the episode.
            if self.up_next_shown {
                self.state.position = self.state.duration;
            }
            self.save_progress(cx);
            cx.emit(PlayerEvent::PlayNext(next));
        }
    }

    fn toggle_chapters(&mut self, cx: &mut Context<Self>) {
        self.chapters_open = !self.chapters_open;
        self.menu = None;
        self.poke(cx);
    }

    fn go_to_chapter(&mut self, index: usize, cx: &mut Context<Self>) {
        self.run(|p| p.seek_chapter(index), cx);
        self.current_chapter = Some(index);
        self.chapters_open = false;
    }

    /// The pointer left the window: fade the controls now instead of after the idle delay.
    /// The pointer left the window: fade the controls now instead of after the idle delay.
    ///
    /// Elements' own hover state can't tell: once the pointer is outside, the window gets no
    /// more mouse moves, so it stays as it was. The window's hover flag does change (from
    /// `WM_MOUSELEAVE`) and redraws, so `render` checks it.
    /// Current volume (0–100), remembered for the next video.
    pub fn volume(&self) -> u32 {
        self.state.volume.round().clamp(0., 100.) as u32
    }

    fn track_window_hover(&mut self, window: &Window) {
        let hovered = window.is_window_hovered();
        if hovered == self.window_hovered {
            return;
        }
        self.window_hovered = hovered;
        if !hovered && self.menu.is_none() && !self.chapters_open {
            self.last_activity = Instant::now() - CONTROLS_IDLE_TIMEOUT;
            self.seek_hover = None;
            self.volume_hover = false;
            self.volume_dragging = false;
            if self.volume_open {
                self.volume_open = false;
                self.volume_open_epoch += 1;
                self.volume_open_changed = Instant::now();
            }
            self.left_window = self.controls_shown;
        }
    }

    fn seek_fraction(&self, x: Pixels) -> Option<f32> {
        let bounds = self.seek_bounds.get()?;
        (bounds.size.width > px(0.))
            .then(|| ((x - bounds.origin.x) / bounds.size.width).clamp(0., 1.))
    }

    /// Chapter whose marker is within a few pixels of `fraction` along the seek bar.
    fn chapter_near(&self, fraction: f32) -> Option<usize> {
        let bounds = self.seek_bounds.get()?;
        let width = f32::from(bounds.size.width);
        if self.state.duration <= 0. || width <= 0. {
            return None;
        }
        self.chapters.iter().position(|c| {
            let at = (c.time / self.state.duration) as f32;
            c.time > 0. && ((at - fraction) * width).abs() <= 6.
        })
    }

    /// Controls show on mouse activity and fade after a moment of idleness, paused or not.
    /// They stay up at the end of the file and while a track menu is open.
    fn controls_visible(&self) -> bool {
        self.state.eof
            || self.menu.is_some()
            || self.chapters_open
            || self.last_activity.elapsed() < CONTROLS_IDLE_TIMEOUT
    }

    fn poke(&mut self, cx: &mut Context<Self>) {
        self.last_activity = Instant::now();
        cx.notify();
    }

    fn run(
        &mut self,
        action: impl FnOnce(&Player) -> miru_player::Result<()>,
        cx: &mut Context<Self>,
    ) {
        if let Err(e) = action(&self.session.player) {
            self.error = Some(e.to_string().into());
        }
        self.poke(cx);
    }

    fn toggle_menu(&mut self, kind: TrackKind, cx: &mut Context<Self>) {
        if self.menu == Some(kind) {
            self.menu = None;
        } else {
            self.tracks = self.session.player.tracks();
            self.audio_output = self.session.player.audio_output_layout();
            self.menu = Some(kind);
        }
        self.poke(cx);
    }

    fn choose_track(&mut self, kind: TrackKind, id: Option<i64>, cx: &mut Context<Self>) {
        if let Some(file_id) = self.file_id
            && (kind == TrackKind::Subtitle || id.is_some())
        {
            let lang = id.and_then(|id| {
                self.tracks
                    .iter()
                    .find(|t| t.kind == kind && t.id == id)
                    .and_then(|t| t.lang.clone())
            });
            cx.emit(PlayerEvent::TrackChosen {
                file_id,
                kind,
                lang,
                id,
            });
        }
        self.run(
            |p| match (kind, id) {
                (TrackKind::Audio, Some(id)) => p.select_audio(id),
                (TrackKind::Audio, None) => Ok(()),
                (TrackKind::Subtitle, id) => p.select_subtitle(id),
            },
            cx,
        );
        self.menu = None;
    }

    fn on_key(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        match event.keystroke.key.as_str() {
            "space" | "k" => self.run(Player::toggle_pause, cx),
            "left" | "j" => {
                let step = self.seek_step;
                self.run(|p| p.seek_relative(-step), cx)
            }
            "right" | "l" => {
                let step = self.seek_step;
                self.run(|p| p.seek_relative(step), cx)
            }
            "home" => self.run(|p| p.seek_absolute(0.0), cx),
            "escape" if self.menu.is_some() || self.chapters_open => {
                self.menu = None;
                self.chapters_open = false;
                self.poke(cx);
            }
            "escape" | "backspace" => self.close(cx),
            _ => {}
        }
    }

    fn on_mouse_move(&mut self, _: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.poke(cx);
    }

    /// `osd`: flash the volume readout. Wheel changes do, since the controls may be hidden;
    /// the slider and mute button don't, as the controls already show the level.
    fn set_volume(&mut self, volume: f64, osd: bool, cx: &mut Context<Self>) {
        let volume = volume.clamp(0., 100.);
        self.run(|p| p.set_volume(volume), cx);
        // Reflect the change immediately rather than on the next poll.
        self.state.volume = volume;
        self.state.muted = false;
        if osd {
            self.volume_changed_at = Instant::now();
        }
    }

    /// Mouse wheel over the picture: each notch changes the volume by 5% (see `wheel`).
    fn on_wheel(&mut self, event: &gpui::ScrollEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.menu.is_some() {
            return; // Let an open track list scroll instead.
        }
        let now = Instant::now();
        let steps = match event.delta {
            gpui::ScrollDelta::Lines(lines) => {
                self.wheel
                    .lines(lines.y, theme::wheel_lines_per_notch(), now)
            }
            gpui::ScrollDelta::Pixels(pixels) => self.wheel.pixels(f32::from(pixels.y), now),
        };
        if steps != 0 {
            let base = if self.state.muted {
                0.
            } else {
                self.state.volume
            };
            self.set_volume(base + f64::from(steps) * VOLUME_STEP, true, cx);
        }
        cx.stop_propagation();
    }

    fn volume_at(&self, x: Pixels) -> Option<f64> {
        let bounds = self.volume_bounds.get()?;
        if bounds.size.width <= px(0.) {
            return None;
        }
        let fraction = ((x - bounds.origin.x) / bounds.size.width).clamp(0., 1.);
        Some((f64::from(fraction) * 100.).round())
    }

    /// Click on the seek bar: jump to that point, or to a chapter when clicking its marker.
    fn on_seek_bar(&mut self, event: &MouseDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let Some(fraction) = self.seek_fraction(event.position.x) else {
            return;
        };
        if self.state.duration <= 0.0 {
            return;
        }
        if let Some(chapter) = self.chapter_near(fraction) {
            self.go_to_chapter(chapter, cx);
            return;
        }
        let target = f64::from(fraction) * self.state.duration;
        self.run(|p| p.seek_absolute(target), cx);
    }

    fn render_top_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .absolute()
            .top_0()
            .left_0()
            .right_0()
            .pl(px(EDGE))
            .pb_16()
            .flex()
            .items_start()
            .bg(linear_gradient(
                180.,
                linear_color_stop(rgba(0x000000b3), 0.),
                linear_color_stop(rgba(0x00000000), 1.),
            ))
            .child(
                div()
                    .pt_6()
                    .flex()
                    .items_center()
                    .gap_4()
                    .child(glass_round_button(
                        "close",
                        "icons/chevron-left.svg",
                        44.,
                        false,
                        cx.listener(|this, _, _, cx| this.close(cx)),
                    )),
            )
            .child(
                div()
                    .flex_1()
                    .h(px(72.))
                    .window_control_area(WindowControlArea::Drag),
            )
            .child(theme::window_controls())
    }

    fn render_transport(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let (play_icon, play_id) = if self.state.paused || self.state.eof {
            ("icons/play.svg", "play")
        } else {
            ("icons/pause.svg", "pause")
        };
        let skip =
            |id: &'static str, icon_path: &'static str, step: f64, cx: &mut Context<Self>| {
                div()
                    .relative()
                    .child(glass_round_button(
                        id,
                        icon_path,
                        64.,
                        false,
                        cx.listener(move |this, _, _, cx| this.run(|p| p.seek_relative(step), cx)),
                    ))
                    .child(
                        div()
                            .absolute()
                            .inset_0()
                            .flex()
                            .items_center()
                            .justify_center()
                            .pt(px(2.))
                            .text_size(px(10.))
                            .font_weight(FontWeight::BOLD)
                            .child(format!("{}", self.seek_step as u32)),
                    )
            };
        div()
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .gap_10()
            .child(skip("rewind", "icons/skip-back.svg", -self.seek_step, cx))
            .child(glass_round_button(
                play_id,
                play_icon,
                92.,
                false,
                cx.listener(|this, _, _, cx| this.run(Player::toggle_pause, cx)),
            ))
            .child(skip(
                "forward",
                "icons/skip-forward.svg",
                self.seek_step,
                cx,
            ))
    }

    /// Seek bar split into chapter segments. Hovering grows the segment under the pointer and
    /// shows a scrub handle at the playhead and a time preview (with the chapter name).
    fn render_seek_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let duration = self.state.duration;
        let progress = if duration > 0.0 {
            (self.state.position / duration).clamp(0.0, 1.0) as f32
        } else {
            0.0
        };
        let hover = self.seek_hover.filter(|_| duration > 0.);
        let bounds_slot = self.seek_bounds.clone();

        // The track is split at chapter starts into segments with small gaps; the segment under
        // the pointer grows taller so it reads as the chapter you'd land in.
        let mut starts: Vec<f32> = vec![0.];
        if duration > 0. {
            starts.extend(
                self.chapters
                    .iter()
                    .map(|c| (c.time / duration).clamp(0., 1.) as f32)
                    .filter(|&at| at > 0.005 && at < 0.995),
            );
        }
        starts.dedup_by(|a, b| (*a - *b).abs() < 0.002);
        let segments: Vec<(f32, f32)> = starts
            .iter()
            .enumerate()
            .map(|(i, &a)| (a, starts.get(i + 1).copied().unwrap_or(1.)))
            .collect();
        let hovered_segment =
            hover.and_then(|f| segments.iter().position(|&(a, b)| f >= a && f <= b));
        let gap = if segments.len() > 1 { 1.5 } else { 0. };
        let track = segments.iter().enumerate().map(|(i, &(a, b))| {
            let span = (b - a).max(f32::EPSILON);
            let filled = ((progress - a) / span).clamp(0., 1.);
            let previewed = hover.map(|f| ((f - a) / span).clamp(0., 1.));
            let hovered = hovered_segment == Some(i);
            let rest_h = if hover.is_some() { 6. } else { 5. };
            let bar = div()
                .w_full()
                .h(px(rest_h))
                .rounded_full()
                .overflow_hidden()
                .relative()
                .bg(rgba(0xffffff40))
                // Where the pointer would seek to, previewed in a lighter fill.
                .children(previewed.map(|p| {
                    div()
                        .absolute()
                        .top_0()
                        .bottom_0()
                        .left_0()
                        .w(relative(p))
                        .bg(rgba(0xffffff33))
                }))
                .child(
                    div()
                        .absolute()
                        .top_0()
                        .bottom_0()
                        .left_0()
                        .w(relative(filled))
                        .bg(theme::text()),
                );
            let bar = if hovered {
                bar.with_animation(
                    ("seek-segment-grow", i),
                    Animation::new(theme::motion(140)).with_easing(gpui::ease_out_quint()),
                    move |el, t| el.h(px(rest_h + (11. - rest_h) * t)),
                )
                .into_any_element()
            } else {
                bar.into_any_element()
            };
            div()
                .absolute()
                .top_0()
                .bottom_0()
                .left(relative(a))
                .w(relative(span))
                .px(px(gap))
                .flex()
                .items_center()
                .child(bar)
        });

        let preview = hover.map(|fraction| {
            let seconds = f64::from(fraction) * duration;
            let chapter = self
                .chapter_near(fraction)
                .or_else(|| self.chapters.iter().rposition(|c| c.time <= seconds))
                .and_then(|i| self.chapters.get(i).map(|c| (i, c)))
                .map(|(i, c)| {
                    c.title
                        .clone()
                        .unwrap_or_else(|| format!("Chapter {}", i + 1))
                });
            div()
                .absolute()
                .bottom(px(26.))
                .left(relative(fraction))
                .ml(px(-80.))
                .w(px(160.))
                .flex()
                .justify_center()
                .child(
                    div()
                        .px_3()
                        .py_1()
                        .rounded(px(8.))
                        .bg(rgba(0x14141ce6))
                        .border_1()
                        .border_color(rgba(0xffffff33))
                        .flex()
                        .flex_col()
                        .items_center()
                        .child(
                            div()
                                .text_sm()
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(format_time(seconds)),
                        )
                        .children(chapter.map(|c| {
                            div()
                                .max_w(px(150.))
                                .truncate()
                                .text_xs()
                                .text_color(theme::text_secondary())
                                .child(c)
                        })),
                )
        });

        // A tall transparent hit area around a thin visible track.
        div()
            .id("seek")
            .relative()
            .w_full()
            .h(px(22.))
            .flex()
            .items_center()
            .cursor_pointer()
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_seek_bar))
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, cx| {
                let hover = this.seek_fraction(event.position.x);
                if hover != this.seek_hover {
                    this.seek_hover = hover;
                    cx.notify();
                }
            }))
            .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                if !hovered {
                    this.seek_hover = None;
                    cx.notify();
                }
            }))
            .child(
                canvas(
                    move |bounds, _, _| bounds_slot.set(Some(bounds)),
                    |_, _, _, _| {},
                )
                .absolute()
                .size_full(),
            )
            .child(div().relative().w_full().h(px(12.)).children(track))
            .when(hover.is_some(), |d| {
                d.child(
                    // Scrub handle at the playhead.
                    div()
                        .absolute()
                        .top(px(3.))
                        .left(relative(progress))
                        .ml(px(-8.))
                        .size(px(16.))
                        .rounded_full()
                        .bg(theme::text())
                        .shadow(theme::card_shadow(0.4)),
                )
            })
            .children(preview)
    }

    /// Chapter list, opened from the Chapters button.
    fn render_chapters_menu(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let max_list = (f32::from(window.viewport_size().height) - 330.).clamp(140., 420.);
        let rows = self.chapters.iter().enumerate().map(|(ix, chapter)| {
            let current = self.current_chapter == Some(ix);
            div()
                .id(("chapter", ix))
                .flex()
                .flex_shrink_0()
                .items_center()
                .gap_3()
                .px_3()
                .py_2()
                .rounded(px(10.))
                .cursor_pointer()
                .when(current, |d| d.bg(theme::glass()))
                .hover(|s| s.bg(theme::glass_hover()))
                .child(
                    div()
                        .w(px(16.))
                        .flex_shrink_0()
                        .children(current.then(|| icon("icons/check.svg", 16., theme::text()))),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_sm()
                        .font_weight(if current {
                            FontWeight::SEMIBOLD
                        } else {
                            FontWeight::MEDIUM
                        })
                        .child(
                            chapter
                                .title
                                .clone()
                                .unwrap_or_else(|| format!("Chapter {}", ix + 1)),
                        ),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(theme::text_tertiary())
                        .child(format_time(chapter.time)),
                )
                .on_click(
                    cx.listener(move |this, _: &ClickEvent, _, cx| this.go_to_chapter(ix, cx)),
                )
        });
        div()
            .id("chapters-menu")
            // Above the click-outside layer: without this, pressing a row also reached that
            // layer, which closed the menu before the click could land.
            .occlude()
            .absolute()
            .right(px(EDGE))
            .bottom(px(150.))
            .w(px(360.))
            .p_2()
            .rounded(px(18.))
            .bg(theme::panel())
            .border_1()
            .border_color(rgba(0xffffff26))
            .shadow(theme::card_shadow(1.))
            .flex()
            .flex_col()
            .child(
                div()
                    .px_3()
                    .pt_2()
                    .pb_2()
                    .text_xs()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme::text_tertiary())
                    .child(format!("CHAPTERS · {}", self.chapters.len())),
            )
            .child(
                div()
                    .id("chapter-list")
                    .max_h(px(max_list))
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .gap_0p5()
                    .children(rows),
            )
    }

    /// Once the credits start: a "Next episode" button that starts white and, while autoplay
    /// counts down, drains from right to left to a dimmed base. With no countdown it stays white.
    fn render_up_next(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<impl IntoElement> {
        self.up_next.as_ref().filter(|_| self.up_next_shown)?;
        // The countdown advances on the 200 ms poll; between polls it is interpolated here and
        // redrawn every frame so the fill moves smoothly.
        let counting = !self.state.paused || self.state.eof;
        let left = self.up_next_remaining.map(|remaining| {
            if counting {
                window.request_animation_frame();
                remaining.saturating_sub(self.last_tick.elapsed())
            } else {
                remaining
            }
        });
        let progress = left
            .map_or(1., |left| {
                left.as_secs_f32() / UP_NEXT_COUNTDOWN.as_secs_f32()
            })
            .clamp(0., 1.);
        const RADIUS: f32 = 10.;
        // GPUI clips children to the button's rectangle, not its rounded corners, so the fill
        // rounds its own corners: the left ones always, the right ones while it reaches them.
        let fill = div()
            .absolute()
            .top_0()
            .bottom_0()
            .left_0()
            .w(relative(progress))
            .bg(theme::text())
            .rounded_l(px(RADIUS))
            .when(progress > 0.97, |d| d.rounded_r(px(RADIUS)));
        let next = div()
            .id("up-next-play")
            .tab_index(0)
            .focus_visible(theme::focus_ring)
            .occlude()
            .absolute()
            .right(px(EDGE))
            .bottom(px(150.))
            .h(px(48.))
            .pl_5()
            .pr_6()
            .flex()
            .items_center()
            .gap_2()
            .rounded(px(RADIUS))
            .bg(rgba(0xffffff8c))
            .shadow(theme::card_shadow(0.6))
            .cursor_pointer()
            .text_color(theme::bg())
            .font_weight(FontWeight::SEMIBOLD)
            .child(fill)
            .child(
                div()
                    .relative()
                    .child(icon("icons/play.svg", 18., theme::bg())),
            )
            .child(div().relative().child("Next episode"))
            .on_click(cx.listener(|this, _, _, cx| this.play_next(cx)));
        Some(next.with_animation(
            ("up-next-in", self.up_next_epoch),
            Animation::new(theme::motion(280)).with_easing(gpui::ease_out_quint()),
            |el, t| el.opacity(t).bottom(px(134. + 16. * t)),
        ))
    }

    /// Volume control matching the round glass buttons beside it. At rest it is just the
    /// speaker (mute) button; hovering it, or dragging its slider, widens it smoothly to show a
    /// slider (click or drag) and the level as a number.
    fn render_volume(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let level = if self.state.muted {
            0.
        } else {
            (self.state.volume / 100.).clamp(0., 1.) as f32
        };
        let icon_path = if level == 0. {
            "icons/volume-mute.svg"
        } else {
            "icons/audio.svg"
        };
        let open = self.volume_open;
        let bounds_slot = self.volume_bounds.clone();

        let slider = div()
            .id("volume")
            .relative()
            .w(px(VOLUME_TRACK_W))
            .h(px(24.))
            .flex_shrink_0()
            .flex()
            .items_center()
            .cursor_pointer()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, _, cx| {
                    this.volume_dragging = true;
                    if let Some(volume) = this.volume_at(event.position.x) {
                        this.set_volume(volume, false, cx);
                    }
                    cx.stop_propagation();
                }),
            )
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, cx| {
                if event.pressed_button == Some(MouseButton::Left)
                    && let Some(volume) = this.volume_at(event.position.x)
                {
                    this.set_volume(volume, false, cx);
                }
            }))
            .child(
                canvas(
                    move |bounds, _, _| bounds_slot.set(Some(bounds)),
                    |_, _, _, _| {},
                )
                .absolute()
                .size_full(),
            )
            .child(
                div()
                    .relative()
                    .w_full()
                    .h(px(5.))
                    .rounded_full()
                    .bg(rgba(0xffffff2e))
                    .child(
                        div()
                            .h_full()
                            .w(relative(level))
                            .rounded_full()
                            .bg(theme::text()),
                    ),
            )
            .child(
                // Knob, centred on the fill's end.
                div()
                    .absolute()
                    .top(px(5.))
                    .left(relative(level))
                    .ml(px(-7.))
                    .size(px(14.))
                    .rounded_full()
                    .bg(theme::text())
                    .shadow(theme::card_shadow(0.4)),
            );
        // The part that slides out: slider and level, clipped by an animated width.
        let drawer = div()
            .w(px(VOLUME_DRAWER_W))
            .flex_shrink_0()
            .pl(px(10.))
            .pr(px(14.))
            .flex()
            .items_center()
            .gap(px(10.))
            .child(slider)
            .child(
                // Fixed width so the pill keeps its size as the number changes.
                div()
                    .w(px(24.))
                    .flex_shrink_0()
                    .flex()
                    .justify_end()
                    .text_xs()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme::text_secondary())
                    .child(format!("{}", (level * 100.).round() as u32)),
            );
        let reveal = |el: gpui::Div, t: f32| el.w(px(VOLUME_DRAWER_W * t)).opacity(t);
        let drawer_clip = div()
            .relative()
            .h_full()
            .flex()
            .items_center()
            .overflow_hidden()
            .child(drawer);
        let animating =
            self.volume_open_changed.elapsed() < VOLUME_EXPAND + Duration::from_millis(40);
        let drawer: gpui::AnyElement = if animating {
            drawer_clip
                .with_animation(
                    ("volume-drawer", self.volume_open_epoch),
                    Animation::new(theme::motion(VOLUME_EXPAND.as_millis() as u64))
                        .with_easing(ease_in_out),
                    move |el, t| reveal(el, if open { t } else { 1. - t }),
                )
                .into_any_element()
        } else {
            reveal(drawer_clip, if open { 1. } else { 0. }).into_any_element()
        };

        div()
            .id("volume-pill")
            .relative()
            .h(px(44.))
            .p(px(3.))
            .flex()
            .items_center()
            .rounded_full()
            .bg(rgba(0x14141c73))
            .border_1()
            .border_color(rgba(0xffffff40))
            .shadow(theme::frost_shadow())
            .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                this.volume_hover = *hovered;
                this.update_volume_open(cx);
            }))
            // Frost tint, brighter while open (as on the round glass buttons).
            .child(div().absolute().inset_0().rounded_full().bg(if open {
                rgba(0xe6e6f033)
            } else {
                rgba(0xe6e6f024)
            }))
            .child(
                div()
                    .id("mute")
                    .tab_index(0)
                    .focus_visible(theme::focus_ring)
                    .relative()
                    .size(px(36.))
                    .flex_shrink_0()
                    .rounded_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .hover(|s| s.bg(rgba(0xffffff1f)))
                    .child(icon(icon_path, 18., theme::text()))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.run(Player::toggle_mute, cx);
                        this.state.muted = !this.state.muted;
                    })),
            )
            .child(drawer)
    }

    /// Opens the volume drawer while hovered or dragging, closes it otherwise.
    fn update_volume_open(&mut self, cx: &mut Context<Self>) {
        let open = self.volume_hover || self.volume_dragging;
        if open != self.volume_open {
            self.volume_open = open;
            self.volume_open_epoch += 1;
            self.volume_open_changed = Instant::now();
            cx.notify();
        }
    }

    /// Brief volume readout after a change, shown even while the controls are hidden.
    fn render_volume_osd(&self) -> Option<impl IntoElement> {
        let elapsed = self.volume_changed_at.elapsed();
        if elapsed >= VOLUME_OSD + Duration::from_millis(250) {
            return None;
        }
        // Fade out over the last 250 ms.
        let opacity = if elapsed < VOLUME_OSD {
            1.
        } else {
            1. - (elapsed - VOLUME_OSD).as_secs_f32() / 0.25
        };
        let level = if self.state.muted {
            0.
        } else {
            (self.state.volume / 100.).clamp(0., 1.) as f32
        };
        let icon_path = if level == 0. {
            "icons/volume-mute.svg"
        } else {
            "icons/audio.svg"
        };
        Some(
            div()
                .absolute()
                .top(px(96.))
                .left_0()
                .right_0()
                .flex()
                .justify_center()
                .child(
                    div()
                        .opacity(opacity.clamp(0., 1.))
                        .h(px(44.))
                        .px_4()
                        .flex()
                        .items_center()
                        .gap_3()
                        .rounded_full()
                        .bg(rgba(0x14141cb3))
                        .border_1()
                        .border_color(rgba(0xffffff33))
                        .shadow(theme::card_shadow(0.5))
                        .child(icon(icon_path, 18., theme::text()))
                        .child(
                            div()
                                .w(px(120.))
                                .h(px(4.))
                                .rounded_full()
                                .bg(rgba(0xffffff40))
                                .child(
                                    div()
                                        .h_full()
                                        .w(relative(level))
                                        .rounded_full()
                                        .bg(theme::text()),
                                ),
                        )
                        .child(
                            div()
                                .w(px(36.))
                                .text_sm()
                                .font_weight(FontWeight::MEDIUM)
                                .child(format!("{}%", (level * 100.).round() as u32)),
                        ),
                ),
        )
    }

    fn render_bottom_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let menu = self.menu;
        let chapter_name: Option<String> = self
            .current_chapter
            .and_then(|i| self.chapters.get(i))
            .map(|c| {
                c.title
                    .clone()
                    .unwrap_or_else(|| format!("Chapter {}", self.current_chapter.unwrap_or(0) + 1))
            });
        let remaining = (self.state.duration - self.state.position).max(0.);
        let track_button =
            |id: &'static str, icon_path: &'static str, kind: TrackKind, cx: &mut Context<Self>| {
                glass_round_button(
                    id,
                    icon_path,
                    44.,
                    menu == Some(kind),
                    cx.listener(move |this, _, _, cx| this.toggle_menu(kind, cx)),
                )
            };

        div()
            .absolute()
            .bottom_0()
            .left_0()
            .right_0()
            .px(px(EDGE))
            .pt_20()
            .pb_8()
            .flex()
            .flex_col()
            .gap_2()
            .bg(linear_gradient(
                0.,
                linear_color_stop(rgba(0x000000d9), 0.),
                linear_color_stop(rgba(0x00000000), 1.),
            ))
            .child(
                div()
                    .flex()
                    .items_end()
                    .gap_3()
                    // Title (and current chapter) on the left, above the seek bar.
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap_0p5()
                            .children(chapter_name.map(|name| {
                                div()
                                    .text_sm()
                                    .text_color(rgba(0xffffffb3))
                                    .truncate()
                                    .child(name)
                            }))
                            .child(
                                div()
                                    .font_family(theme::FONT_DISPLAY)
                                    .text_size(px(24.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .truncate()
                                    .child(self.title.clone()),
                            ),
                    )
                    .child(self.render_volume(cx))
                    .child(track_button(
                        "audio",
                        "icons/waveform.svg",
                        TrackKind::Audio,
                        cx,
                    ))
                    .child(track_button(
                        "subtitles",
                        "icons/captions.svg",
                        TrackKind::Subtitle,
                        cx,
                    ))
                    .when(!self.chapters.is_empty(), |d| {
                        d.child(glass_round_button(
                            "chapters",
                            "icons/chapters.svg",
                            44.,
                            self.chapters_open,
                            cx.listener(|this, _, _, cx| this.toggle_chapters(cx)),
                        ))
                    }),
            )
            .child(self.render_seek_bar(cx))
            .child(
                div()
                    .flex()
                    .justify_between()
                    .text_xs()
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(rgba(0xffffffcc))
                    .child(format_time(self.state.position))
                    .child(format!("-{}", format_time(remaining))),
            )
    }

    fn render_track_menu(
        &self,
        kind: TrackKind,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        struct Entry {
            primary: SharedString,
            detail: SharedString,
            id: Option<i64>,
            selected: bool,
        }
        let of_kind: Vec<&Track> = self.tracks.iter().filter(|t| t.kind == kind).collect();
        let mut entries = Vec::new();
        if kind == TrackKind::Subtitle {
            entries.push(Entry {
                primary: "Off".into(),
                detail: SharedString::default(),
                id: None,
                selected: !of_kind.iter().any(|t| t.selected),
            });
        }
        entries.extend(of_kind.iter().map(|t| Entry {
            primary: t.primary().into(),
            detail: t.detail().into(),
            id: Some(t.id),
            selected: t.selected,
        }));
        let heading = match kind {
            TrackKind::Audio => "Audio",
            TrackKind::Subtitle => "Subtitles",
        };
        // Long track lists scroll inside the panel instead of running off the top of the window.
        let max_list = (f32::from(window.viewport_size().height) - 330.).clamp(140., 380.);
        let footer = (kind == TrackKind::Audio).then(|| {
            let output = match self.audio_output.as_deref() {
                Some("stereo") => "Stereo".to_string(),
                Some("mono") => "Mono".to_string(),
                Some(layout) => format!("{layout} surround"),
                None => "—".to_string(),
            };
            div()
                .mt_1()
                .pt_2()
                .px_3()
                .pb_1()
                .border_t_1()
                .border_color(theme::glass_border())
                .flex()
                .justify_between()
                .text_xs()
                .text_color(theme::text_tertiary())
                .child("Output")
                .child(div().text_color(theme::text_secondary()).child(output))
        });

        div()
            .id("track-menu")
            // Above the click-outside layer (see the chapters menu).
            .occlude()
            .absolute()
            .right(px(EDGE))
            .bottom(px(150.))
            .w(px(340.))
            .p_2()
            .rounded(px(18.))
            .bg(theme::panel())
            .border_1()
            .border_color(rgba(0xffffff26))
            .shadow(theme::card_shadow(1.))
            .flex()
            .flex_col()
            .child(
                div()
                    .px_3()
                    .pt_2()
                    .pb_2()
                    .text_xs()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme::text_tertiary())
                    .child(format!("{} · {}", heading.to_uppercase(), of_kind.len())),
            )
            .when(entries.is_empty(), |d| {
                d.child(
                    div()
                        .px_3()
                        .py_2()
                        .text_sm()
                        .text_color(theme::text_secondary())
                        .child("No tracks"),
                )
            })
            .child(
                div()
                    .id("track-list")
                    .max_h(px(max_list))
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .gap_0p5()
                    .children(entries.into_iter().enumerate().map(|(ix, entry)| {
                        let Entry {
                            primary,
                            detail,
                            id,
                            selected,
                        } = entry;
                        div()
                            .id(("track", ix))
                            .flex()
                            .flex_shrink_0()
                            .items_center()
                            .gap_3()
                            .px_3()
                            .py_2()
                            .rounded(px(10.))
                            .cursor_pointer()
                            .when(selected, |d| d.bg(theme::glass()))
                            .hover(|s| s.bg(theme::glass_hover()))
                            .child(div().w(px(16.)).flex_shrink_0().children(
                                selected.then(|| icon("icons/check.svg", 16., theme::text())),
                            ))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .flex()
                                    .flex_col()
                                    .child(
                                        div()
                                            .text_sm()
                                            .truncate()
                                            .font_weight(if selected {
                                                FontWeight::SEMIBOLD
                                            } else {
                                                FontWeight::MEDIUM
                                            })
                                            .child(primary),
                                    )
                                    .when(!detail.is_empty(), |d| {
                                        d.child(
                                            div()
                                                .text_xs()
                                                .truncate()
                                                .text_color(theme::text_tertiary())
                                                .child(detail),
                                        )
                                    }),
                            )
                            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                this.choose_track(kind, id, cx)
                            }))
                    })),
            )
            .children(footer)
    }
}

impl Render for PlayerView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let viewport = window.viewport_size();
        self.track_window_hover(window);
        let scale = window.scale_factor();
        self.session.host.resize(
            (f32::from(viewport.width) * scale).round() as i32,
            (f32::from(viewport.height) * scale).round() as i32,
        );

        let visible = self.controls_visible();
        if visible != self.controls_shown {
            self.controls_shown = visible;
            self.controls_epoch += 1;
            self.controls_changed = Instant::now();
            self.controls_fade_ms = if !visible && self.left_window {
                CONTROLS_LEAVE_FADE_MS
            } else {
                CONTROLS_FADE_MS
            };
            self.left_window = false;
            // An idle pointer over the picture disappears along with the controls.
            if !visible {
                crate::video_host::hide_cursor_until_moved();
            }
        }
        let fading_out = !visible
            && self.controls_changed.elapsed() < Duration::from_millis(self.controls_fade_ms + 40);

        // No background: the transparent scene lets mpv's child window show through.
        let mut root = div()
            .id("player")
            .track_focus(&self.focus)
            .size_full()
            .relative()
            .text_color(theme::text())
            .on_key_down(cx.listener(Self::on_key))
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            // Releasing the button anywhere ends a volume drag (the drawer may then close).
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    if this.volume_dragging {
                        this.volume_dragging = false;
                        this.update_volume_open(cx);
                    }
                }),
            )
            // Mouse wheel over the picture changes the volume.
            .on_scroll(cx.listener(Self::on_wheel))
            // Right-click anywhere on the picture toggles play/pause.
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(|this, _, _, cx| this.run(Player::toggle_pause, cx)),
            );

        // Until mpv shows its first frame the transparent scene would reveal whatever is behind
        // the window (the empty video host doesn't paint), so cover it in black, then fade.
        let cover = match self.video_ready_at.map(|at| at.elapsed()) {
            None => Some(1.),
            Some(e) if e < COVER_HOLD => Some(1.),
            Some(e) if e < COVER_HOLD + COVER_FADE => {
                Some(1. - (e - COVER_HOLD).as_secs_f32() / COVER_FADE.as_secs_f32())
            }
            Some(_) => None,
        };
        if let Some(opacity) = cover {
            root = root.child(
                div()
                    .absolute()
                    .inset_0()
                    .bg(gpui::black())
                    .opacity(opacity),
            );
            if self.video_ready_at.is_some() {
                window.request_animation_frame();
            }
        }

        if visible || fading_out {
            // A light dim keeps frosted controls legible over bright video.
            let mut overlay = div()
                .absolute()
                .inset_0()
                .bg(rgba(0x00000040))
                .child(self.render_transport(cx))
                .child(self.render_top_bar(cx))
                .child(self.render_bottom_bar(cx));
            // With a menu open, a click anywhere else just closes it (like a popover), rather
            // than also pressing whatever is underneath.
            if self.menu.is_some() || self.chapters_open {
                overlay = overlay.child(
                    div()
                        .id("menu-dismiss")
                        .absolute()
                        .inset_0()
                        .block_mouse_except_scroll()
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|this, _, _, cx| {
                                this.menu = None;
                                this.chapters_open = false;
                                this.poke(cx);
                                cx.stop_propagation();
                            }),
                        ),
                );
            }
            if let Some(kind) = self.menu {
                overlay = overlay.child(self.render_track_menu(kind, window, cx));
            }
            if self.chapters_open {
                overlay = overlay.child(self.render_chapters_menu(window, cx));
            }
            let animation =
                Animation::new(theme::motion(self.controls_fade_ms)).with_easing(ease_in_out);
            root = root.child(overlay.with_animation(
                ("controls", self.controls_epoch),
                animation,
                move |el, t| el.opacity(if visible { t } else { 1. - t }),
            ));
        }
        if let Some(osd) = self.render_volume_osd() {
            root = root.child(osd);
        }
        if let Some(card) = self.render_up_next(window, cx) {
            root = root.child(card);
        }
        if let Some(error) = &self.error {
            root = root.child(
                div()
                    .absolute()
                    .top(px(96.))
                    .left(px(EDGE))
                    .px_4()
                    .py_2()
                    .rounded(px(10.))
                    .bg(rgba(0x7f1d1de6))
                    .text_sm()
                    .child(error.clone()),
            );
        }
        root
    }
}

impl Drop for PlayerView {
    fn drop(&mut self) {
        // Confirms the session (mpv and its video window) is released when the player closes.
        tracing::debug!(title = %self.title, "player released");
    }
}
