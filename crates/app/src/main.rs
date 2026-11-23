// Release builds are a windowed app with no console; development builds keep the console
// for logs.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod assets;
mod config;
mod folder_watch;
mod genres;
mod image_cache;
mod library_sync;
mod library_view;
mod media_controls;
mod memory;
mod player_view;
mod text_edit;
mod theme;
mod up_next;
mod updates;
mod video_host;
mod wheel;

use gpui::{
    App, Bounds, Context, Entity, Subscription, TitlebarOptions, Window,
    WindowBackgroundAppearance, WindowBounds, WindowOptions, div, prelude::*, px, size,
};

use crate::library_sync::AppPaths;
use crate::library_view::{LibraryEvent, LibraryView};
use crate::player_view::{PlaybackRequest, PlayerEvent, PlayerView, Session};
use miru_core::db::Db;
use miru_core::settings::AudioOutput;
use miru_player::{AudioChannels, PlaybackOptions};

/// Top-level shell: the library is always alive; a player is created per playback session
/// and dropped on close, which shuts down mpv and its video window.
struct Root {
    library: Entity<LibraryView>,
    player: Option<(Entity<PlayerView>, Subscription)>,
    _library_events: Subscription,
}

impl Root {
    fn new(
        paths: AppPaths,
        db: Db,
        initial: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let library = cx.new(|cx| LibraryView::new(paths, db, cx));
        let library_events =
            cx.subscribe_in(&library, window, |this, _, event, window, cx| match event {
                LibraryEvent::Play(request) => this.play(request.clone(), window, cx),
            });
        let mut root = Self {
            library,
            player: None,
            _library_events: library_events,
        };
        match initial {
            Some(path) => {
                let title = std::path::Path::new(&path)
                    .file_stem()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let request = PlaybackRequest {
                    path,
                    title: title.into(),
                    file_id: None,
                    start: None,
                };
                root.play(request, window, cx);
            }
            None => root.focus_library(window, cx),
        }
        root
    }

    fn play(&mut self, request: PlaybackRequest, window: &mut Window, cx: &mut Context<Self>) {
        // Tear down any previous session before creating a new video window.
        self.player = None;
        let settings = self.library.read(cx).settings().clone();
        // A file brings back the exact tracks it was last watched with; failing that, a show
        // brings back the language and variant last picked for its episodes.
        let (prefs, file_prefs) = request
            .file_id
            .map(|id| self.library.read(cx).track_prefs(id))
            .unwrap_or_default();
        let file_subtitle = file_prefs
            .subtitle_track
            .filter(|_| file_prefs.subtitles_on == Some(true));
        let options = PlaybackOptions {
            subtitles: file_prefs
                .subtitles_on
                .or(prefs.subtitles_on)
                .unwrap_or(settings.subtitles_by_default),
            audio_channels: match settings.audio_output {
                AudioOutput::Stereo => AudioChannels::Stereo,
                AudioOutput::Source => AudioChannels::Source,
                AudioOutput::Surround71 => AudioChannels::Surround71,
            },
            audio_language: prefs
                .audio_lang
                .clone()
                .or_else(|| settings.audio_language.clone()),
            subtitle_language: prefs
                .subtitle_lang
                .clone()
                .or_else(|| settings.subtitle_language.clone()),
            // A show's track ids only stand in for choices whose tracks had no language tag
            // (ids differ between episodes); a file's own ids are exact.
            audio_track: file_prefs
                .audio_track
                .or(prefs.audio_track.filter(|_| prefs.audio_lang.is_none())),
            subtitle_track: file_subtitle.or(prefs
                .subtitle_track
                .filter(|_| prefs.subtitle_lang.is_none())),
            audio_title: prefs.audio_title.clone(),
            subtitle_title: prefs.subtitle_title.clone(),
            volume: settings.volume,
            subtitle_scale_pct: (settings.subtitle_size.scale() * 100.).round() as u32,
        };
        let session = match Session::start(&request, &options, window) {
            Ok(session) => session,
            Err(e) => {
                tracing::error!("cannot start playback: {e:#}");
                self.library.update(cx, |lib, cx| {
                    lib.show_error(format!("Cannot play: {e:#}"), cx)
                });
                return;
            }
        };
        let next = request
            .file_id
            .and_then(|id| self.library.read(cx).next_episode_request(id));
        let artwork = request
            .file_id
            .and_then(|id| self.library.read(cx).artwork_for(id));
        let player = cx.new(|cx| {
            let mut view =
                PlayerView::new(session, request, f64::from(settings.seek_step), window, cx);
            view.set_up_next(next, settings.autoplay_next);
            view.set_artwork(artwork);
            view
        });
        let events = cx.subscribe_in(
            &player,
            window,
            |this, player, event, window, cx| match event {
                PlayerEvent::Progress {
                    file_id,
                    position,
                    duration,
                } => {
                    let (file_id, position, duration) = (*file_id, *position, *duration);
                    this.library.update(cx, |lib, _| {
                        lib.record_progress(file_id, position, duration)
                    });
                }
                PlayerEvent::TrackChosen {
                    file_id,
                    kind,
                    lang,
                    title,
                    id,
                } => {
                    let (file_id, kind, id) = (*file_id, *kind, *id);
                    let (lang, title) = (lang.clone(), title.clone());
                    this.library.update(cx, |lib, _| {
                        lib.remember_track(file_id, kind, lang.as_deref(), title.as_deref(), id)
                    });
                }
                PlayerEvent::PlayNext(next) => {
                    // Carry the volume over to the next episode.
                    let volume = player.read(cx).volume();
                    this.library
                        .update(cx, |lib, _| lib.remember_volume(volume));
                    // Start the next episode after this event finishes dispatching.
                    let next = next.clone();
                    cx.defer_in(window, move |this, window, cx| this.play(next, window, cx));
                }
                PlayerEvent::Close => {
                    let volume = player.read(cx).volume();
                    this.player = None;
                    this.library.update(cx, |lib, cx| {
                        lib.remember_volume(volume);
                        lib.set_playing(false);
                        lib.refresh(cx);
                        lib.log_memory("player closed", cx);
                    });
                    this.focus_library(window, cx);
                    cx.notify();
                }
            },
        );
        self.player = Some((player, events));
        self.library.update(cx, |lib, _| lib.set_playing(true));
        cx.notify();
    }

    fn focus_library(&self, window: &mut Window, cx: &mut Context<Self>) {
        let handle = self.library.read(cx).focus_handle().clone();
        window.focus(&handle, cx);
    }
}

impl Render for Root {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let screen = match &self.player {
            Some((player, _)) => player.clone().into_any_element(),
            None => self.library.clone().into_any_element(),
        };
        div().size_full().font_family(theme::FONT).child(screen)
    }
}

/// Logs to the console in development. Release builds have no console window, so they write
/// to `miru.log` beside the library database instead (replaced on each launch).
fn init_logging(paths: Option<&AppPaths>) {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    let builder = tracing_subscriber::fmt().with_env_filter(filter);
    let log_file = paths
        .filter(|_| cfg!(not(debug_assertions)))
        .and_then(|p| std::fs::File::create(p.db.with_file_name("miru.log")).ok());
    match log_file {
        Some(file) => builder
            .with_ansi(false)
            .with_writer(std::sync::Mutex::new(file))
            .init(),
        None => builder.init(),
    }
}

fn main() {
    let resolved = AppPaths::resolve();
    init_logging(resolved.as_ref().ok());
    // Release builds have no console, so a panic would vanish without a trace; log it first.
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        tracing::error!("panic: {info}");
        default_hook(info);
    }));
    let initial = std::env::args().nth(1);
    let (paths, db) = match resolved.and_then(|p| Ok((Db::open(&p.db)?, p))) {
        Ok((db, paths)) => (paths, db),
        Err(e) => {
            tracing::error!("cannot open library: {e:#}");
            std::process::exit(1);
        }
    };

    gpui_platform::application()
        .with_assets(assets::Assets)
        .run(move |cx: &mut App| {
            cx.bind_keys(library_view::key_bindings());
            let bounds = Bounds::centered(None, size(px(1440.), px(900.)), cx);
            let opened = cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    titlebar: Some(TitlebarOptions {
                        title: Some("Miru".into()),
                        // The app draws its own title bar so artwork can run edge to edge.
                        appears_transparent: true,
                        ..Default::default()
                    }),
                    // Transparent so the player's mpv child window can show through the UI.
                    window_background: WindowBackgroundAppearance::Transparent,
                    window_min_size: Some(size(px(720.), px(480.))),
                    ..Default::default()
                },
                move |window, cx| cx.new(|cx| Root::new(paths, db, initial, window, cx)),
            );
            if let Err(e) = opened {
                tracing::error!("failed to open window: {e:#}");
                cx.quit();
                return;
            }
            cx.activate(true);
        });
}
