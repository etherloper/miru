//! Library browsing: a cinematic home (rotating hero + shelves), movie and show grids, and
//! full-bleed detail pages.

use std::collections::{BTreeSet, HashMap, VecDeque};
use std::hash::{BuildHasher, Hash, Hasher};
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui::{
    Animation, AnimationExt as _, AnyElement, ClickEvent, Context, Div, ElementId, EventEmitter,
    FocusHandle, FontWeight, KeyDownEvent, ObjectFit, PathPromptOptions, ScrollHandle,
    SharedString, UniformListScrollHandle, Window, WindowControlArea, div, ease_in_out,
    ease_out_quint, linear_color_stop, linear_gradient, prelude::*, px, rgba, svg, uniform_list,
};
use miru_core::db::{Db, MediaFile};
use miru_core::episodes::{ArtSource, EpisodeArt};
use miru_core::library::{ItemKind, LibraryItem, Metadata, TitleKey};
use miru_core::parse::ParsedMedia;
use miru_core::playback::{ContinueReason, Progress};
use miru_core::settings::{
    AudioOutput, LibrarySort, RootStats, SEEK_STEPS, Settings, SubtitleSize, title_sort_key,
};
use miru_metadata::Tmdb;
use miru_player::format_time;

use crate::genres::GenreFilter;
use crate::image_cache::art_image;
use crate::library_sync::{self, AppPaths};
use crate::player_view::PlaybackRequest;
use crate::theme::{self, glass_button, icon, primary_button, round_button};

mod about;
mod cards;
mod cast;
mod collections;
mod detail;
mod fix_match;
mod grid;
mod hero;
mod home;
mod menus;
mod nav_order;
mod poster_drag;
mod search;
mod settings_page;
mod sidebar;

use collections::{CardMenu, CollectionEdit};
use detail::*;
use grid::*;
use menus::MenuEntry;
use miru_core::collections::Collection;
use poster_drag::{DropTarget, Landing, PosterDrag};
use settings_page::*;

const PAGE_PADDING: f32 = 48.;
const NAV_HEIGHT: f32 = 64.;
const SIDEBAR_W: f32 = 216.;
const SIDEBAR_COLLAPSED_W: f32 = 72.;
const NAV_ITEM_H: f32 = 38.;
const NAV_ITEM_GAP: f32 = 2.;
/// How many pages Back remembers.
const HISTORY_DEPTH: usize = 30;
const SHELF_GAP: f32 = 18.;
/// Room above and below shelf cards for the hover lift and its shadow.
const SHELF_PAD_TOP: f32 = 16.;
const SHELF_PAD_BOTTOM: f32 = 24.;
const SHELF_ARROW_W: f32 = 96.;
/// Box the row chevron sits in.
const ARROW_BOX: f32 = 40.;
const POSTER_W: f32 = 240.;
const MIN_GRID_TILE: f32 = 236.;
const CONTINUE_W: f32 = 392.;
const CAPTION_H: f32 = 46.;
const HERO_ITEMS: usize = 5;
const HERO_INTERVAL: Duration = Duration::from_secs(9);
/// How often the rotation checks whether the interval has passed.
const HERO_TICK: Duration = Duration::from_millis(250);
/// Distance of the banner's page dots from its bottom edge (the shelves overlap below).
const HERO_DOTS_BOTTOM: f32 = 116.;
/// Left/Right page the banner only while the home page is scrolled less than this.
const HERO_KEYS_MAX_SCROLL: f32 = 320.;
const HERO_FADE: Duration = Duration::from_millis(900);
const RECENT_LIMIT: u32 = 20;
/// Cards on the Movies and TV Shows shelves of the home page.
const SHELF_LIMIT: usize = 24;
const CONTINUE_LIMIT: usize = 12;
const METADATA_BATCH: u32 = 6;
const HISTORY_LIMIT: u32 = 50;
/// Live results shown in the search panel.
const PANEL_TITLE_HITS: usize = 6;
const PANEL_EPISODE_HITS: usize = 4;
const EPISODE_ART_BATCH: u32 = 4;
const EPISODE_CARD_W: f32 = 352.;
const SYNOPSIS_LINE_H: f32 = 20.;
/// Height of everything under a show's hero, so a page that fits doesn't scroll: heading row
/// (34) + gap (8) + shelf padding (16 + 24) + episode card with a two-line synopsis (304) +
/// bottom padding (32). The band the row overlaps is added to the hero separately.
const EPISODES_ROW_H: f32 = 34. + 8. + 16. + 24. + 304. + 32.;
/// How far the episode row rises into the bottom of a show's hero.
const EPISODES_OVERLAP: f32 = 56.;
/// Widest each kind of artwork is decoded at: about twice its layout width, for high-DPI
/// screens. Full-bleed backdrops are capped near 1440p width.
const ART_POSTER: u32 = 560;
const ART_BACKDROP: u32 = 2560;
const ART_LOGO: u32 = 1040;
const ART_STILL: u32 = 720;
const ART_CARD: u32 = 800;
const ART_THUMB: u32 = 160;
/// Hover lift: cards grow by this fraction.
const LIFT: f32 = 0.05;
/// Library renders slower than this are logged (debug level).
const SLOW_RENDER: Duration = Duration::from_millis(6);
type ClickHandler = Box<dyn Fn(&ClickEvent, &mut Window, &mut gpui::App)>;

/// Shelves with their own horizontal scroll handle.
const SHELVES: [&str; 5] = [
    "shelf-continue",
    "shelf-recent",
    "shelf-movies",
    "shelf-shows",
    "shelf-episodes",
];

gpui::actions!(
    miru,
    [
        FocusNext,
        FocusPrev,
        GoBack,
        GoForward,
        ToggleSidebar,
        OpenSearch,
        GoHome,
        GoMovies,
        GoShows
    ]
);

/// Keyboard shortcuts for the library (registered once at startup).
pub fn key_bindings() -> Vec<gpui::KeyBinding> {
    use gpui::KeyBinding;
    vec![
        KeyBinding::new("tab", FocusNext, None),
        KeyBinding::new("shift-tab", FocusPrev, None),
        KeyBinding::new("alt-left", GoBack, None),
        KeyBinding::new("alt-right", GoForward, None),
        KeyBinding::new("ctrl-b", ToggleSidebar, None),
        KeyBinding::new("ctrl-k", OpenSearch, None),
        KeyBinding::new("ctrl-f", OpenSearch, None),
        KeyBinding::new("ctrl-1", GoHome, None),
        KeyBinding::new("ctrl-2", GoMovies, None),
        KeyBinding::new("ctrl-3", GoShows, None),
    ]
}

pub enum LibraryEvent {
    Play(PlaybackRequest),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Home,
    Movies,
    Shows,
    Search,
}

#[derive(Clone)]
enum Page {
    Browse(Tab),
    Settings,
    Movie {
        item: LibraryItem,
        progress: Option<Progress>,
    },
    Show {
        item: LibraryItem,
        episodes: Vec<MediaFile>,
        progress: HashMap<i64, Progress>,
        season: Option<u16>,
        art: HashMap<i64, EpisodeArt>,
        /// Episode the play button offers (see `Db::show_up_next`).
        up_next: Option<i64>,
    },
    /// A collection's titles (those still in the library).
    Collection {
        id: i64,
        items: Vec<LibraryItem>,
    },
}

#[derive(Clone, PartialEq)]
enum SyncStatus {
    Idle,
    Scanning,
    FetchingMetadata,
    Failed(SharedString),
}

/// A ready-to-render "Continue watching" entry.
#[derive(Clone, PartialEq)]
struct ContinueCard {
    request: PlaybackRequest,
    /// Show whose episode artwork hasn't been fetched yet (this card shows the backdrop until
    /// it has).
    art_pending: Option<String>,
    title: SharedString,
    subtitle: SharedString,
    /// Landscape artwork: backdrop, falling back to the poster.
    art: Option<String>,
    accent: Option<u32>,
    fraction: f32,
}

/// Height of the artwork on a shelf's cards, for placing its arrows.
fn shelf_art_height(id: &str) -> f32 {
    match id {
        "shelf-continue" => CONTINUE_W * 9. / 16.,
        "shelf-episodes" => EPISODE_CARD_W * 9. / 16.,
        _ => POSTER_W * 1.5,
    }
}

/// A shelf row being dragged with the left mouse button.
struct ShelfDrag {
    id: &'static str,
    scroll: ScrollHandle,
    /// Pointer x and the row's scroll offset when the button went down.
    start_x: f32,
    start_offset: f32,
    /// Moved past the threshold: it's a drag, not a click.
    moved: bool,
    /// Recent pointer positions, to measure how fast the row was flung on release.
    samples: VecDeque<(Instant, f32)>,
}

/// Pointer travel before a press on a row becomes a drag.
const DRAG_THRESHOLD: f32 = 5.;
/// Pointer movement this recent sets the speed of a fling.
const FLING_WINDOW: Duration = Duration::from_millis(100);
/// Holding still this long before letting go cancels the fling.
const FLING_STALE: Duration = Duration::from_millis(60);
/// Slower releases just stop where they are.
const FLING_MIN_SPEED: f32 = 250.;
const FLING_MAX_SPEED: f32 = 8000.;
/// How far a fling carries: its release speed times this many seconds.
const FLING_REACH: f32 = 0.3;

/// A shelf scrolling to `to` after an arrow press.
struct ShelfGlide {
    id: &'static str,
    scroll: ScrollHandle,
    from: f32,
    to: f32,
    /// Offset set on the previous frame, to notice the user scrolling meanwhile.
    last: f32,
    started: Instant,
    duration: f32,
}

pub struct LibraryView {
    paths: AppPaths,
    db: Db,
    tmdb: Option<Tmdb>,
    /// Where the TMDB key came from (shown in Settings).
    tmdb_source: Option<crate::config::KeySource>,
    page: Page,
    /// Bumped on every navigation so the new page fades in.
    page_epoch: usize,
    /// When the current page appeared; its fade-in only plays if this is recent, so returning
    /// from the player doesn't replay it.
    page_changed_at: Instant,
    /// Pages to return to (Alt+Left, mouse back) and to go forward to again.
    back_stack: Vec<Page>,
    forward_stack: Vec<Page>,
    /// A video is playing in front of the library; background animations pause.
    playing: bool,
    hero_changed_at: Instant,
    sidebar_epoch: usize,
    sidebar_changed_at: Instant,
    /// Columns in the current grid, for Up/Down keyboard navigation.
    grid_cols: std::cell::Cell<usize>,
    movies: Rc<Vec<LibraryItem>>,
    shows: Rc<Vec<LibraryItem>>,
    /// Genre filters for the Movies and TV Shows grids.
    movie_filter: GenreFilter,
    show_filter: GenreFilter,
    /// Decoded artwork, bounded so visiting many pages doesn't grow memory without limit.
    image_cache: gpui::Entity<crate::image_cache::BoundedImageCache>,
    /// Cards as cached views, kept across frames (see `cards`).
    cards: cards::Cards,
    /// The home hero as a cached view (see `hero`).
    hero: hero::Hero,
    /// Watched movie files and fully watched shows (lowercased), for badges and filters.
    watched_movies: std::collections::HashSet<i64>,
    watched_shows: std::collections::HashSet<String>,
    /// Grid whose genre dropdown is open, and when it last closed (so the click that closed it
    /// by landing on the trigger doesn't reopen it).
    genre_menu: Option<Tab>,
    genre_menu_closed: Instant,
    /// Detail page "⋯" menu: open, and when it last closed (see `genre_menu_closed`).
    more_menu: bool,
    more_menu_closed: Instant,
    /// Fix match panel and its keyboard focus.
    fix_match: Option<fix_match::FixMatch>,
    /// Full cast sheet, and an epoch per opening (for its entrance).
    cast_pane: Option<cast::CastPane>,
    cast_epoch: usize,
    fix_focus: FocusHandle,
    fix_focus_pending: bool,
    /// Return focus to the library after the Fix match panel closes itself.
    fix_focus_restore: bool,
    collections: Vec<Collection>,
    collection_edit: Option<CollectionEdit>,
    collection_focus: FocusHandle,
    collection_focus_pending: bool,
    /// A collection that just received a title, and when (its sidebar row glows).
    collection_flash: Option<(i64, Instant)>,
    collection_flash_epoch: usize,
    /// A collection whose delete is waiting for confirmation.
    collection_delete: Option<i64>,
    card_menu: Option<CardMenu>,
    card_menu_closed: Instant,
    poster_drag: Option<PosterDrag>,
    poster_press_epoch: usize,
    poster_drag_ended: Option<Instant>,
    drop_target: Option<DropTarget>,
    landing: Option<Landing>,
    landing_epoch: usize,
    nav_drag: Option<nav_order::NavDrag>,
    /// Sidebar-order rows' eased offsets while reordering.
    nav_shift: Vec<f32>,
    nav_frame: Instant,
    /// Watches the library folders; a change while a scan runs queues another one.
    folder_watch: Option<crate::folder_watch::FolderWatch>,
    rescan_pending: bool,
    /// Update check and install progress (Settings → About).
    update: about::UpdateState,
    /// TMDB key being typed in Settings, and its field's focus.
    key_draft: Option<crate::text_edit::TextEdit>,
    key_focus: FocusHandle,
    key_focus_pending: bool,
    recent: Vec<LibraryItem>,
    featured: Vec<LibraryItem>,
    continue_cards: Vec<ContinueCard>,
    hero_index: usize,
    /// The title shown before the current one, faded out over the change.
    hero_previous: Option<LibraryItem>,
    /// Whole-library mode: the shuffle (new each launch) and how many sets have been dealt.
    hero_seed: u64,
    hero_round: u64,
    /// Left out of the next set: the title on screen when it was dealt.
    hero_avoid: Option<TitleKey>,
    hero_epoch: usize,
    hovered: Option<SharedString>,
    unhovered: Option<SharedString>,
    hover_epoch: usize,
    root_count: usize,
    sync: SyncStatus,
    focus: FocusHandle,
    grid_scroll: UniformListScrollHandle,
    /// Show whose episode thumbnails are being fetched, so a revisit doesn't start a second job.
    episode_job: Option<String>,
    /// Shows whose episode frames were fetched for Continue Watching this session.
    continue_art_tried: std::collections::HashSet<String>,
    settings: Settings,
    /// Library folders with counts, shown in Settings.
    roots: Vec<RootStats>,
    /// Recently watched files with progress, for the Watch History section in Settings.
    history: Vec<(MediaFile, Progress)>,
    confirm_clear_history: bool,
    /// Folder awaiting confirmation before removal.
    confirm_remove: Option<i64>,
    // Search: the query being typed, its input focus, and the current results.
    search_query: String,
    /// Floating search panel: whether it is open and the text being typed into it.
    search_overlay: bool,
    search_draft: crate::text_edit::TextEdit,
    /// Highlighted row in the panel's live results.
    search_selected: usize,
    search_focus: FocusHandle,
    search_focus_pending: bool,
    search_titles: Vec<LibraryItem>,
    search_episodes: Vec<MediaFile>,
    search_art: HashMap<i64, EpisodeArt>,
    /// Horizontal scroll state per shelf, for the chevron buttons and edge fades.
    shelf_scrolls: HashMap<&'static str, ScrollHandle>,
    /// Shelf under the pointer, and shelf whose arrow button is under the pointer.
    hovered_shelf: Option<&'static str>,
    hovered_arrow: Option<&'static str>,
    /// Shelf scroll started by an arrow press, advanced each frame.
    shelf_glide: Option<ShelfGlide>,
    /// A shelf being dragged with the mouse, and when a drag that moved last ended (the click
    /// that ends a drag must not open the card under the pointer).
    shelf_drag: Option<ShelfDrag>,
    shelf_drag_ended: Option<Instant>,
    /// Download fill drawn on the sidebar update button, and when (see `glide_fill`).
    update_fill: std::cell::Cell<(f32, Instant)>,
    /// Logo image sizes read from file headers, keyed by path.
    image_sizes: std::cell::RefCell<HashMap<String, Option<(u32, u32)>>>,
    /// Scroll positions of the home and detail pages; the nav turns solid once scrolled.
    home_scroll: ScrollHandle,
    detail_scroll: ScrollHandle,
}

impl EventEmitter<LibraryEvent> for LibraryView {}

impl LibraryView {
    pub fn new(paths: AppPaths, db: Db, cx: &mut Context<Self>) -> Self {
        let image_cache = crate::image_cache::BoundedImageCache::new(&paths.artwork, cx);
        let settings = db.settings().unwrap_or_else(|e| {
            tracing::error!("reading settings, using defaults: {e}");
            Settings::default()
        });
        let key = crate::config::tmdb_key(&paths, settings.tmdb_key.as_deref());
        let tmdb_source = key.as_ref().map(|(_, source)| *source);
        let tmdb = key.map(|(key, _)| Tmdb::new(&key));
        let mut view = Self {
            paths,
            db,
            tmdb,
            tmdb_source,
            page: Page::Browse(Tab::Home),
            page_epoch: 0,
            page_changed_at: Instant::now(),
            back_stack: Vec::new(),
            forward_stack: Vec::new(),
            playing: false,
            hero_changed_at: Instant::now(),
            sidebar_epoch: 0,
            sidebar_changed_at: Instant::now() - Duration::from_secs(60),
            grid_cols: std::cell::Cell::new(1),
            movies: Rc::default(),
            shows: Rc::default(),
            movie_filter: GenreFilter::default(),
            show_filter: GenreFilter::default(),
            image_cache,
            cards: Default::default(),
            hero: hero::Hero::new(cx),
            watched_movies: Default::default(),
            watched_shows: Default::default(),
            genre_menu: None,
            genre_menu_closed: Instant::now(),
            more_menu: false,
            more_menu_closed: Instant::now(),
            fix_match: None,
            cast_pane: None,
            cast_epoch: 0,
            fix_focus: cx.focus_handle(),
            fix_focus_pending: false,
            fix_focus_restore: false,
            collections: Vec::new(),
            collection_edit: None,
            collection_focus: cx.focus_handle(),
            collection_focus_pending: false,
            collection_flash: None,
            collection_flash_epoch: 0,
            collection_delete: None,
            card_menu: None,
            card_menu_closed: Instant::now(),
            poster_drag: None,
            poster_press_epoch: 0,
            poster_drag_ended: None,
            drop_target: None,
            landing: None,
            landing_epoch: 0,
            nav_drag: None,
            nav_shift: Vec::new(),
            nav_frame: Instant::now(),
            folder_watch: None,
            rescan_pending: false,
            update: Default::default(),
            key_draft: None,
            key_focus: cx.focus_handle(),
            key_focus_pending: false,
            recent: Vec::new(),
            featured: Vec::new(),
            continue_cards: Vec::new(),
            hero_index: 0,
            hero_previous: None,
            hero_seed: std::collections::hash_map::RandomState::new().hash_one(0u8),
            hero_round: 0,
            hero_avoid: None,
            hero_epoch: 0,
            hovered: None,
            unhovered: None,
            hover_epoch: 0,
            root_count: 0,
            sync: SyncStatus::Idle,
            focus: cx.focus_handle(),
            grid_scroll: UniformListScrollHandle::new(),
            episode_job: None,
            continue_art_tried: Default::default(),
            settings,
            roots: Vec::new(),
            history: Vec::new(),
            confirm_clear_history: false,
            confirm_remove: None,
            search_query: String::new(),
            search_overlay: false,
            search_draft: crate::text_edit::TextEdit::new(""),
            search_selected: 0,
            search_focus: cx.focus_handle(),
            search_focus_pending: false,
            search_titles: Vec::new(),
            search_episodes: Vec::new(),
            search_art: HashMap::new(),
            shelf_scrolls: SHELVES
                .iter()
                .map(|id| (*id, ScrollHandle::new()))
                .collect(),
            hovered_shelf: None,
            hovered_arrow: None,
            shelf_glide: None,
            shelf_drag: None,
            shelf_drag_ended: None,
            update_fill: std::cell::Cell::new((0., Instant::now())),
            image_sizes: Default::default(),
            home_scroll: ScrollHandle::new(),
            detail_scroll: ScrollHandle::new(),
        };
        view.start_folder_watch(cx);
        view.reload();
        view.start_sync(cx);
        view.start_hero_rotation(cx);
        view.auto_check_for_updates(cx);
        view
    }

    pub fn focus_handle(&self) -> &FocusHandle {
        &self.focus
    }

    pub fn show_error(&mut self, message: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.sync = SyncStatus::Failed(message.into());
        cx.notify();
    }

    pub fn record_progress(&mut self, file_id: i64, position: f64, duration: f64) {
        match self.db.save_progress(file_id, position, duration) {
            Ok(saved) => tracing::debug!(file_id, position, duration, ?saved, "saved progress"),
            Err(e) => tracing::error!(file_id, "saving playback progress: {e}"),
        }
    }

    /// Logs process memory and decoded-artwork totals (debug level), now and again a few
    /// seconds later once background work has settled.
    pub fn log_memory(&self, context: &'static str, cx: &mut Context<Self>) {
        fn log(view: &LibraryView, context: &str, cx: &gpui::App) {
            let (image_bytes, images) = view.image_cache.read(cx).usage();
            tracing::debug!(
                context,
                private_mb = crate::memory::private_bytes().map(crate::memory::mb),
                images,
                image_mb = crate::memory::mb(image_bytes),
                "memory"
            );
        }
        log(self, context, cx);
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_secs(5)).await;
            this.update(cx, |view, cx| log(view, context, cx)).ok();
        })
        .detach();
    }

    /// Marks files watched or unwatched by hand and refreshes everything that shows it.
    fn mark_watched(&mut self, file_ids: Vec<i64>, watched: bool, cx: &mut Context<Self>) {
        if let Err(e) = self.db.set_watched(&file_ids, watched) {
            tracing::error!("marking watched: {e}");
            self.sync = SyncStatus::Failed(format!("Could not update watched state: {e}").into());
        }
        self.refresh(cx);
        cx.notify();
    }

    /// Reloads lists and the open detail page, e.g. after playback ends.
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        self.reload();
        self.fill_continue_art(cx);
        let result = match &mut self.page {
            Page::Browse(_) | Page::Settings | Page::Collection { .. } => Ok(()),
            Page::Movie { item, progress } => self.db.progress(item.file_id).map(|p| *progress = p),
            Page::Show {
                item,
                progress,
                art,
                up_next,
                ..
            } => self.db.show_progress(&item.key.title).and_then(|p| {
                *progress = p;
                *art = self.db.episode_art(&item.key.title)?;
                *up_next = self.db.show_up_next(&item.key.title)?;
                Ok(())
            }),
        };
        if let Err(e) = result {
            self.sync = SyncStatus::Failed(format!("Could not read progress: {e}").into());
        }
        cx.notify();
    }

    fn reload(&mut self) {
        let result = (|| -> miru_core::db::Result<()> {
            let mut movies = self.db.library(ItemKind::Movie)?;
            sort_items(&mut movies, self.settings.movie_sort);
            self.movies = Rc::new(movies);
            let mut shows = self.db.library(ItemKind::Show)?;
            sort_items(&mut shows, self.settings.show_sort);
            self.shows = Rc::new(shows);
            (self.watched_movies, self.watched_shows) = self.db.watched_titles()?;
            self.apply_filters();
            self.prune_cards();
            let mut recent = self.db.recently_added(ItemKind::Movie, RECENT_LIMIT)?;
            recent.extend(self.db.recently_added(ItemKind::Show, RECENT_LIMIT)?);
            recent.sort_by_key(|i| std::cmp::Reverse(i.added_at));
            recent.truncate(RECENT_LIMIT as usize);
            self.recent = recent;
            self.continue_cards = self.build_continue_cards()?;
            self.reload_collections();
            self.roots = self.db.root_stats()?;
            self.history = self.db.watch_history(HISTORY_LIMIT)?;
            self.root_count = self.roots.len();
            if let Some(watch) = &mut self.folder_watch {
                let roots: Vec<PathBuf> = self
                    .db
                    .roots()?
                    .into_iter()
                    .map(|r| PathBuf::from(r.path))
                    .collect();
                watch.set_roots(&roots);
            }
            Ok(())
        })();
        if let Err(e) = result {
            tracing::error!("reloading library: {e}");
            self.sync = SyncStatus::Failed(format!("Could not read library: {e}").into());
        }
        self.refresh_featured();
    }

    /// Titles for the hero: the recently added, or with "whole library" on, a shuffle of
    /// everything that changes each launch and each time the banner comes back round.
    fn pick_featured(&self) -> Vec<LibraryItem> {
        let pool: Vec<&LibraryItem> = if self.settings.feature_whole_library {
            let mut all: Vec<&LibraryItem> = self.movies.iter().chain(self.shows.iter()).collect();
            if all.len() > HERO_ITEMS {
                all.retain(|i| self.hero_avoid.as_ref() != Some(&i.key));
            }
            all.sort_by_cached_key(|i| {
                let mut hasher = std::collections::hash_map::DefaultHasher::new();
                (self.hero_seed, self.hero_round, &i.key).hash(&mut hasher);
                hasher.finish()
            });
            all
        } else {
            self.recent.iter().collect()
        };
        // Prefer titles with backdrops; fill up with the rest.
        let has_backdrop = |i: &LibraryItem| {
            i.metadata
                .as_ref()
                .is_some_and(|m| m.backdrop_file.is_some())
        };
        let (with, without): (Vec<&LibraryItem>, Vec<&LibraryItem>) =
            pool.into_iter().partition(|i| has_backdrop(i));
        with.into_iter()
            .chain(without)
            .take(HERO_ITEMS)
            .cloned()
            .collect()
    }

    /// Re-picks the hero's titles, starting from the first if the set changed.
    fn refresh_featured(&mut self) {
        let featured = self.pick_featured();
        if featured
            .iter()
            .map(|i| &i.key)
            .ne(self.featured.iter().map(|i| &i.key))
        {
            self.hero_index = 0;
            self.hero_previous = None;
        }
        self.featured = featured;
    }

    fn build_continue_cards(&self) -> miru_core::db::Result<Vec<ContinueCard>> {
        let mut cards = Vec::new();
        for entry in self.db.continue_watching(CONTINUE_LIMIT)? {
            let key = match &entry.file.media {
                ParsedMedia::Movie { title, year } => TitleKey {
                    kind: ItemKind::Movie,
                    title: title.clone(),
                    year: *year,
                },
                ParsedMedia::Episode { show, .. } => TitleKey {
                    kind: ItemKind::Show,
                    title: show.clone(),
                    year: None,
                },
            };
            let meta = self.db.metadata(&key)?.unwrap_or_default();
            // Episodes show their own frame (TMDB still or a grab from the file); movies, and
            // episodes whose frame isn't fetched yet, the title's backdrop.
            let episode_art = match &entry.file.media {
                ParsedMedia::Episode { show, .. } => {
                    Some(self.db.episode_art(show)?.remove(&entry.file.id))
                }
                ParsedMedia::Movie { .. } => None,
            };
            let still = episode_art
                .as_ref()
                .and_then(|a| a.as_ref())
                .filter(|a| a.source != ArtSource::None)
                .and_then(|a| a.still_file.clone());
            let art_pending = matches!(episode_art, Some(None)).then(|| key.title.clone());
            let display = meta
                .display_title
                .clone()
                .unwrap_or_else(|| key.title.clone());
            let (subtitle, player_title) = match &entry.file.media {
                ParsedMedia::Movie { .. } => (remaining(&entry.progress), display.clone()),
                ParsedMedia::Episode { .. } => {
                    let episode = episode_label(&entry.file.media);
                    let subtitle = match entry.reason {
                        ContinueReason::NextEpisode => format!("Up next · {episode}"),
                        ContinueReason::Resume => {
                            format!(
                                "{} · {}",
                                short_episode(&entry.file.media),
                                remaining(&entry.progress)
                            )
                        }
                    };
                    (subtitle, format!("{display} · {episode}"))
                }
            };
            cards.push(ContinueCard {
                request: PlaybackRequest {
                    path: entry.file.path.clone(),
                    title: player_title.into(),
                    file_id: Some(entry.file.id),
                    start: entry.progress.resume_at(),
                },
                title: display.into(),
                subtitle: subtitle.into(),
                art: still
                    .or_else(|| meta.backdrop_file.clone())
                    .or_else(|| meta.poster_file.clone()),
                art_pending,
                accent: meta.accent_rgb,
                fraction: entry
                    .progress
                    .resume_at()
                    .map_or(0., |_| entry.progress.fraction()),
            });
        }
        Ok(cards)
    }

    /// Scans all folders, then fills in metadata in small batches so posters appear progressively.
    fn start_sync(&mut self, cx: &mut Context<Self>) {
        if matches!(
            self.sync,
            SyncStatus::Scanning | SyncStatus::FetchingMetadata
        ) {
            return;
        }
        self.sync = SyncStatus::Scanning;
        cx.notify();
        let paths = self.paths.clone();
        let tmdb = self.tmdb.clone();

        cx.spawn(async move |this, cx| {
            let scan_paths = paths.clone();
            let scanned = cx
                .background_executor()
                .spawn(async move { library_sync::scan_all(&scan_paths) })
                .await;
            let scan_ok = this
                .update(cx, |view, cx| {
                    view.reload();
                    view.fill_continue_art(cx);
                    match &scanned {
                        Ok(report) => {
                            tracing::info!(?report, "library scan finished");
                            view.sync = SyncStatus::Idle;
                        }
                        Err(e) => {
                            view.sync = SyncStatus::Failed(format!("Scan failed: {e:#}").into())
                        }
                    }
                    cx.notify();
                    scanned.is_ok()
                })
                .unwrap_or(false);
            if !scan_ok {
                this.update(cx, |view, cx| view.finish_sync(cx)).ok();
                return;
            }

            loop {
                if this
                    .update(cx, |view, cx| {
                        view.sync = SyncStatus::FetchingMetadata;
                        cx.notify();
                    })
                    .is_err()
                {
                    return;
                }
                let (batch_paths, batch_tmdb) = (paths.clone(), tmdb.clone());
                let saved = cx
                    .background_executor()
                    .spawn(async move {
                        library_sync::enrich_batch(
                            &batch_paths,
                            batch_tmdb.as_ref(),
                            METADATA_BATCH,
                        )
                    })
                    .await;
                let more = this
                    .update(cx, |view, cx| {
                        view.reload();
                        let more = match saved {
                            Ok(n) => n > 0,
                            Err(e) => {
                                view.sync =
                                    SyncStatus::Failed(format!("Metadata failed: {e:#}").into());
                                false
                            }
                        };
                        if !more && view.sync == SyncStatus::FetchingMetadata {
                            view.sync = SyncStatus::Idle;
                        }
                        cx.notify();
                        more
                    })
                    .unwrap_or(false);
                if !more {
                    this.update(cx, |view, cx| view.finish_sync(cx)).ok();
                    return;
                }
            }
        })
        .detach();
    }

    /// Runs the rescan a folder change asked for while the last sync was busy.
    fn finish_sync(&mut self, cx: &mut Context<Self>) {
        if std::mem::take(&mut self.rescan_pending) {
            self.start_sync(cx);
        }
    }

    /// Starts watching the library folders; each settled change triggers a rescan.
    fn start_folder_watch(&mut self, cx: &mut Context<Self>) {
        let (changed, on_change) = async_channel::bounded(1);
        match crate::folder_watch::FolderWatch::start(changed) {
            Ok(watch) => self.folder_watch = Some(watch),
            Err(e) => {
                tracing::warn!("folder watching unavailable: {e}");
                return;
            }
        }
        cx.spawn(async move |this, cx| {
            while on_change.recv().await.is_ok() {
                let alive = this.update(cx, |view, cx| {
                    tracing::info!("library folder changed, rescanning");
                    if matches!(
                        view.sync,
                        SyncStatus::Scanning | SyncStatus::FetchingMetadata
                    ) {
                        view.rescan_pending = true;
                    } else {
                        view.start_sync(cx);
                    }
                });
                if alive.is_err() {
                    return;
                }
            }
        })
        .detach();
    }

    fn start_hero_rotation(&self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            loop {
                // Checked often against the time of the last change, rather than waiting a full
                // interval per tick: a tick landing a hair early would otherwise be skipped,
                // doubling the gap. Any change (automatic or manual) restarts the wait.
                cx.background_executor().timer(HERO_TICK).await;
                let alive = this.update(cx, |view, cx| {
                    let on_home = matches!(view.page, Page::Browse(Tab::Home));
                    let due = view.hero_changed_at.elapsed() >= HERO_INTERVAL;
                    if on_home
                        && due
                        && view.featured.len() > 1
                        && theme::motion_enabled()
                        && view.settings.rotate_featured
                        && !view.playing
                    {
                        tracing::debug!("hero: automatic change");
                        view.step_hero(1, cx);
                    }
                });
                if alive.is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    /// Steps the featured banner forwards or backwards, wrapping at either end.
    fn step_hero(&mut self, direction: isize, cx: &mut Context<Self>) {
        let count = self.featured.len();
        if direction > 0 && self.hero_index + 1 == count && self.deal_featured(cx) {
            return;
        }
        if count > 1 {
            let next = (self.hero_index as isize + direction).rem_euclid(count as isize);
            self.show_hero(next as usize, cx);
        }
    }

    /// Whole-library mode: after the last title, deals a fresh set instead of going back to
    /// the first. Returns false when that doesn't apply.
    fn deal_featured(&mut self, cx: &mut Context<Self>) -> bool {
        if !self.settings.feature_whole_library
            || self.movies.len() + self.shows.len() <= HERO_ITEMS
        {
            return false;
        }
        let current = self.featured.get(self.hero_index).cloned();
        self.hero_round += 1;
        self.hero_avoid = current.as_ref().map(|i| i.key.clone());
        self.featured = self.pick_featured();
        tracing::debug!(round = self.hero_round, "hero: new set");
        self.hero_previous = current;
        self.hero_index = 0;
        self.hero_epoch += 1;
        self.hero_changed_at = Instant::now();
        self.redraw_hero(cx);
        cx.notify();
        true
    }

    fn show_hero(&mut self, index: usize, cx: &mut Context<Self>) {
        if index == self.hero_index || index >= self.featured.len() {
            return;
        }
        tracing::debug!(from = self.hero_index, to = index, "hero changed");
        self.hero_previous = self.featured.get(self.hero_index).cloned();
        self.hero_index = index;
        self.hero_epoch += 1;
        self.hero_changed_at = Instant::now();
        self.redraw_hero(cx);
        cx.notify();
    }

    /// Tells the cached hero view to redraw. Done right where its state changes: a request made
    /// while the library itself is drawing can be dropped, which left the hero frozen while
    /// hovered and then jumping ahead.
    fn redraw_hero(&self, cx: &mut Context<Self>) {
        // Marks the view dirty by id rather than updating it: this runs from the hero's own
        // hover handler, where updating the hero again would be a re-entrant update (a crash).
        let id = self.hero.view().entity_id();
        let app: &mut gpui::App = cx;
        app.notify(id);
    }

    fn add_folders(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        let picked = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: true,
            prompt: Some("Add to library".into()),
        });
        cx.spawn(async move |this, cx| {
            let paths = match picked.await {
                Ok(Ok(Some(paths))) => paths,
                Ok(Ok(None)) | Err(_) => return,
                Ok(Err(e)) => {
                    this.update(cx, |view, cx| {
                        view.show_error(format!("Folder picker failed: {e}"), cx)
                    })
                    .ok();
                    return;
                }
            };
            this.update(cx, |view, cx| {
                for path in paths {
                    let Some(path) = path.to_str() else {
                        view.show_error("Folder path is not valid Unicode", cx);
                        continue;
                    };
                    if let Err(e) = view.db.add_root(path) {
                        view.show_error(format!("Could not add folder: {e}"), cx);
                    }
                }
                view.reload();
                view.start_sync(cx);
            })
            .ok();
        })
        .detach();
    }

    /// Goes to a new page, remembering the current one for Back.
    fn navigate(&mut self, page: Page, cx: &mut Context<Self>) {
        self.log_memory("navigate", cx);
        self.close_genre_menu();
        self.cast_pane = None;
        self.close_more_menu();
        self.close_card_menu();
        self.collection_delete = None;
        let same_tab = matches!((&self.page, &page), (Page::Browse(a), Page::Browse(b)) if a == b);
        if !same_tab {
            let previous = std::mem::replace(&mut self.page, page);
            self.back_stack.push(previous);
            if self.back_stack.len() > HISTORY_DEPTH {
                self.back_stack.remove(0);
            }
            self.forward_stack.clear();
        }
        self.show_page(cx);
    }

    /// Resets per-page state after the page changes.
    fn show_page(&mut self, cx: &mut Context<Self>) {
        self.page_epoch += 1;
        self.page_changed_at = Instant::now();
        self.hovered = None;
        self.unhovered = None;
        cx.notify();
    }

    fn go_back(&mut self, cx: &mut Context<Self>) {
        if let Some(page) = self.back_stack.pop() {
            let current = std::mem::replace(&mut self.page, page);
            self.forward_stack.push(current);
            // Saved pages may predate playback in between; refresh their progress and art.
            self.refresh(cx);
            self.show_page(cx);
        }
    }

    fn go_forward(&mut self, cx: &mut Context<Self>) {
        if let Some(page) = self.forward_stack.pop() {
            let current = std::mem::replace(&mut self.page, page);
            self.back_stack.push(current);
            self.refresh(cx);
            self.show_page(cx);
        }
    }

    /// Called by the shell when playback starts or stops, so background animation pauses.
    pub fn set_playing(&mut self, playing: bool) {
        self.playing = playing;
    }

    fn toggle_sidebar(&mut self, cx: &mut Context<Self>) {
        self.update_settings(|s| s.sidebar_collapsed = !s.sidebar_collapsed, cx);
        self.sidebar_epoch += 1;
        self.sidebar_changed_at = Instant::now();
    }

    fn sidebar_width(&self) -> f32 {
        if self.settings.sidebar_collapsed {
            SIDEBAR_COLLAPSED_W
        } else {
            SIDEBAR_W
        }
    }

    /// Opens the title with this key, looked up in the current lists.
    fn open_key(&mut self, key: &TitleKey, cx: &mut Context<Self>) {
        let list = match key.kind {
            ItemKind::Movie => &self.movies,
            ItemKind::Show => &self.shows,
        };
        if let Some(item) = list.iter().find(|i| &i.key == key).cloned() {
            self.open(&item, cx);
        }
    }

    fn open(&mut self, item: &LibraryItem, cx: &mut Context<Self>) {
        let page = match item.key.kind {
            ItemKind::Movie => self.db.progress(item.file_id).map(|progress| Page::Movie {
                item: item.clone(),
                progress,
            }),
            ItemKind::Show => self.db.episodes(&item.key.title).and_then(|episodes| {
                let progress = self.db.show_progress(&item.key.title)?;
                let art = self.db.episode_art(&item.key.title)?;
                let up_next = self.db.show_up_next(&item.key.title)?;
                // Open on the season of the episode the play button offers.
                let season = offered_episode(&episodes, &progress, up_next)
                    .and_then(|e| season_of(&e.media));
                Ok(Page::Show {
                    item: item.clone(),
                    episodes,
                    progress,
                    season,
                    art,
                    up_next,
                })
            }),
        };
        match page {
            Ok(page) => {
                self.navigate(page, cx);
                if item.key.kind == ItemKind::Show {
                    self.start_episode_art(item.key.title.clone(), cx);
                }
            }
            Err(e) => self.show_error(format!("Could not open title: {e}"), cx),
        }
    }

    /// Fetches stills (TMDB, or frames grabbed from the files) for a show's episodes in small
    /// batches, refreshing the page as each batch lands.
    /// Fetches episode frames for the first Continue Watching show that lacks them; each job
    /// ends by calling this again, so the shows are filled one after another.
    fn fill_continue_art(&mut self, cx: &mut Context<Self>) {
        if self.episode_job.is_some() {
            return;
        }
        // Once per show per session: a show whose frames can't be fetched mustn't loop.
        let next = self
            .continue_cards
            .iter()
            .filter_map(|c| c.art_pending.clone())
            .find(|show| !self.continue_art_tried.contains(show));
        if let Some(show) = next {
            self.continue_art_tried.insert(show.clone());
            self.start_episode_art(show, cx);
        }
    }

    fn start_episode_art(&mut self, show: String, cx: &mut Context<Self>) {
        if self.episode_job.is_some() {
            return;
        }
        self.episode_job = Some(show.clone());
        let (paths, tmdb) = (self.paths.clone(), self.tmdb.clone());
        cx.spawn(async move |this, cx| {
            loop {
                let (job_paths, job_tmdb, job_show) = (paths.clone(), tmdb.clone(), show.clone());
                let saved = cx
                    .background_executor()
                    .spawn(async move {
                        library_sync::enrich_show_episodes(
                            &job_paths,
                            job_tmdb.as_ref(),
                            &job_show,
                            EPISODE_ART_BATCH,
                        )
                    })
                    .await;
                let more = this
                    .update(cx, |view, cx| {
                        if let Page::Show { item, art, .. } = &mut view.page
                            && item.key.title == show
                            && let Ok(fresh) = view.db.episode_art(&show)
                        {
                            *art = fresh;
                            cx.notify();
                        }
                        match saved {
                            Ok(n) => n > 0,
                            Err(e) => {
                                tracing::warn!(show, "episode artwork failed: {e:#}");
                                false
                            }
                        }
                    })
                    .unwrap_or(false);
                if !more {
                    break;
                }
            }
            this.update(cx, |view, cx| {
                view.episode_job = None;
                if let Ok(cards) = view.build_continue_cards() {
                    view.continue_cards = cards;
                    cx.notify();
                }
                view.fill_continue_art(cx);
            })
            .ok();
        })
        .detach();
    }

    fn play_movie(&mut self, item: &LibraryItem, resume: bool, cx: &mut Context<Self>) {
        let start = if resume {
            self.db
                .progress(item.file_id)
                .ok()
                .flatten()
                .and_then(|p| p.resume_at())
        } else {
            None
        };
        match self.db.file_path(item.file_id) {
            Ok(Some(path)) => cx.emit(LibraryEvent::Play(PlaybackRequest {
                path,
                title: item.display_title().to_string().into(),
                file_id: Some(item.file_id),
                start,
            })),
            Ok(None) => self.show_error("That file is no longer in the library", cx),
            Err(e) => self.show_error(format!("Could not open file: {e}"), cx),
        }
    }

    fn set_tab(&mut self, tab: Tab, cx: &mut Context<Self>) {
        if tab == Tab::Search {
            self.search_focus_pending = true;
        }
        self.navigate(Page::Browse(tab), cx);
    }

    /// Follows a shelf drag: the row moves with the pointer, clamped to its ends.
    fn drag_shelf(&mut self, event: &gpui::MouseMoveEvent, cx: &mut Context<Self>) {
        if self.poster_dragging() {
            return;
        }
        let Some(drag) = &mut self.shelf_drag else {
            return;
        };
        // The button came up outside the window: the drag is over.
        if event.pressed_button != Some(gpui::MouseButton::Left) {
            self.end_shelf_drag(cx);
            return;
        }
        let pointer = f32::from(event.position.x);
        let now = Instant::now();
        drag.samples.push_back((now, pointer));
        while drag.samples.len() > 2
            && drag
                .samples
                .front()
                .is_some_and(|(t, _)| now.duration_since(*t) > FLING_WINDOW)
        {
            drag.samples.pop_front();
        }
        let dx = pointer - drag.start_x;
        if !drag.moved && dx.abs() < DRAG_THRESHOLD {
            return;
        }
        drag.moved = true;
        let max = f32::from(drag.scroll.max_offset().x);
        let x = (drag.start_offset + dx).clamp(-max, 0.);
        let y = drag.scroll.offset().y;
        drag.scroll.set_offset(gpui::point(px(x), y));
        cx.notify();
    }

    /// Ends a shelf drag. A quick swipe carries on as a glide that slows to a stop, going
    /// further the faster the row was flung.
    fn end_shelf_drag(&mut self, cx: &mut Context<Self>) {
        let Some(drag) = self.shelf_drag.take() else {
            return;
        };
        if !drag.moved {
            return;
        }
        self.shelf_drag_ended = Some(Instant::now());
        let speed = fling_speed(&drag.samples).clamp(-FLING_MAX_SPEED, FLING_MAX_SPEED);
        if speed.abs() < FLING_MIN_SPEED || !theme::motion_enabled() {
            return;
        }
        // Glides count distance along the row, the opposite sign to the scroll offset.
        let from = -f32::from(drag.scroll.offset().x);
        let max = f32::from(drag.scroll.max_offset().x);
        let to = (from - speed * FLING_REACH).clamp(0., max);
        if (to - from).abs() < 1. {
            return;
        }
        // The glide's ease-out starts at five times its average speed; this duration makes it
        // leave at the speed of the swipe.
        let duration = (5. * (to - from).abs() / speed.abs()).clamp(0.2, 1.6);
        self.shelf_glide = Some(ShelfGlide {
            id: drag.id,
            scroll: drag.scroll,
            from,
            to,
            last: from,
            started: Instant::now(),
            duration,
        });
        cx.notify();
    }

    /// Whether a click should be ignored because it ends a drag.
    pub(super) fn click_ends_drag(&self) -> bool {
        self.shelf_drag.as_ref().is_some_and(|d| d.moved)
            || self
                .shelf_drag_ended
                .is_some_and(|t| t.elapsed() < Duration::from_millis(250))
            || self.click_ends_poster_drag()
    }
    fn set_hover(&mut self, id: &SharedString, hovered: bool, cx: &mut Context<Self>) {
        let touches_hero =
            id.starts_with("hero") || self.hovered.as_ref().is_some_and(|h| h.starts_with("hero"));
        if touches_hero {
            self.redraw_hero(cx);
        }
        if hovered {
            if self.hovered.as_ref() != Some(id) {
                self.unhovered = self.hovered.take();
                self.hovered = Some(id.clone());
                self.hover_epoch += 1;
                cx.notify();
            }
        } else if self.hovered.as_ref() == Some(id) {
            self.unhovered = self.hovered.take();
            self.hover_epoch += 1;
            cx.notify();
        }
    }
}

impl LibraryView {
    /// Arrow keys move keyboard focus between cards and buttons (Up/Down jump a whole row in
    /// grids); Backspace and Esc go back. Other shortcuts are actions (see `key_bindings`).
    fn on_library_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let k = &event.keystroke;
        if k.modifiers.control || k.modifiers.alt || k.modifiers.platform {
            return;
        }
        let in_grid = matches!(self.page, Page::Browse(Tab::Movies | Tab::Shows));
        let row = if in_grid {
            self.grid_cols.get().max(1)
        } else {
            1
        };
        // On Home, with nothing focused and the banner in view, Left/Right page the banner.
        // Once a card or button has focus, they move focus as everywhere else.
        let pages_hero = matches!(self.page, Page::Browse(Tab::Home))
            && self.focus.is_focused(window)
            && -f32::from(self.home_scroll.offset().y) < HERO_KEYS_MAX_SCROLL;
        match k.key.as_str() {
            "right" if pages_hero => self.step_hero(1, cx),
            "left" if pages_hero => self.step_hero(-1, cx),
            "right" => window.focus_next(cx),
            "left" => window.focus_prev(cx),
            "down" => (0..row).for_each(|_| window.focus_next(cx)),
            "up" => (0..row).for_each(|_| window.focus_prev(cx)),
            "escape" if self.genre_menu.is_some() => {
                self.close_genre_menu();
                cx.notify();
            }
            "escape" if self.cast_pane.is_some() => self.cast_back(cx),
            "escape" if self.more_menu => {
                self.close_more_menu();
                cx.notify();
            }
            "escape" if self.card_menu.is_some() => {
                self.close_card_menu();
                cx.notify();
            }
            "escape" if self.collection_delete.is_some() => {
                self.collection_delete = None;
                cx.notify();
            }
            "backspace" | "escape" => self.go_back(cx),
            _ => return,
        }
        cx.stop_propagation();
    }
}

fn next_episode<'a>(
    episodes: &'a [MediaFile],
    progress: &HashMap<i64, Progress>,
) -> Option<&'a MediaFile> {
    episodes
        .iter()
        .find(|e| !progress.get(&e.id).is_some_and(|p| p.watched))
}

/// The episode a show page's play button offers: the one following on from what was played
/// last, else the first unwatched, else the first.
fn offered_episode<'a>(
    episodes: &'a [MediaFile],
    progress: &HashMap<i64, Progress>,
    up_next: Option<i64>,
) -> Option<&'a MediaFile> {
    up_next
        .and_then(|id| episodes.iter().find(|e| e.id == id))
        .or_else(|| next_episode(episodes, progress))
        .or_else(|| episodes.first())
}

fn season_of(media: &ParsedMedia) -> Option<u16> {
    match media {
        ParsedMedia::Episode { season, .. } => Some(*season),
        ParsedMedia::Movie { .. } => None,
    }
}

fn episode_request(show_title: &str, file: &MediaFile, progress: &Progress) -> PlaybackRequest {
    PlaybackRequest {
        path: file.path.clone(),
        title: format!("{show_title} · {}", episode_label(&file.media)).into(),
        file_id: Some(file.id),
        start: progress.resume_at(),
    }
}

/// `S01E02 Half Loop`
fn episode_label(media: &ParsedMedia) -> String {
    match media {
        ParsedMedia::Episode {
            title: Some(title), ..
        } => format!("{} {title}", short_episode(media)),
        _ => short_episode(media),
    }
}

/// `S01E02`
fn short_episode(media: &ParsedMedia) -> String {
    match media {
        ParsedMedia::Episode {
            season, episode, ..
        } => format!("S{season:02}E{episode:02}"),
        ParsedMedia::Movie { title, .. } => title.clone(),
    }
}

fn remaining(progress: &Progress) -> String {
    let left = (progress.duration - progress.position).max(0.);
    if left >= 60. {
        format!("{} min left", (left / 60.).round() as u64)
    } else {
        "Almost done".into()
    }
}

fn format_duration(seconds: f64) -> String {
    if seconds < 60. {
        return "<1m".into();
    }
    let minutes = (seconds / 60.).round() as u64;
    if minutes >= 60 {
        format!("{}h {}m", minutes / 60, minutes % 60)
    } else {
        format!("{minutes}m")
    }
}

/// Full-bleed artwork layer; empty when there is no art.
/// `max_width` is the widest the art is decoded at (see `art_image`).
fn art_layer(path: Option<String>, radius: f32, max_width: u32) -> AnyElement {
    let inner: AnyElement = match path {
        Some(path) => art_image(path, max_width)
            .size_full()
            // Images are not clipped by a parent's rounded corners, so round them directly.
            .rounded(px(radius))
            .object_fit(ObjectFit::Cover)
            .with_fallback(|| div().into_any_element())
            .into_any_element(),
        None => div().into_any_element(),
    };
    div().absolute().inset_0().child(inner).into_any_element()
}

impl Render for LibraryView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let render_started = Instant::now();
        self.step_shelf_glide(window);
        self.step_poster_drag(window);
        if matches!(self.page, Page::Settings) {
            self.step_nav_order(window);
        }
        if std::mem::take(&mut self.collection_focus_pending) {
            window.focus(&self.collection_focus, cx);
        }
        if self.search_focus_pending {
            self.search_focus_pending = false;
            window.focus(&self.search_focus, cx);
        }
        if std::mem::take(&mut self.key_focus_pending) {
            window.focus(&self.key_focus, cx);
        }
        if std::mem::take(&mut self.fix_focus_pending) {
            window.focus(&self.fix_focus, cx);
        }
        if std::mem::take(&mut self.fix_focus_restore) {
            window.focus(&self.focus, cx);
        }
        // Settings stays reachable with an empty library: that is where folders are added.
        let body: AnyElement = if self.root_count == 0 && !matches!(self.page, Page::Settings) {
            self.render_empty(cx).into_any_element()
        } else {
            match &self.page {
                Page::Browse(Tab::Home) => self.render_home(window, cx).into_any_element(),
                Page::Settings => self.render_settings(window, cx).into_any_element(),
                Page::Browse(Tab::Search) => self.render_search(cx).into_any_element(),
                Page::Browse(Tab::Movies) => self.render_grid(
                    Tab::Movies,
                    "Movies",
                    self.movie_filter.items.clone(),
                    window,
                    cx,
                ),
                Page::Browse(Tab::Shows) => self.render_grid(
                    Tab::Shows,
                    "TV Shows",
                    self.show_filter.items.clone(),
                    window,
                    cx,
                ),
                Page::Collection { id, items } => {
                    let (id, items) = (*id, items.clone());
                    self.render_collection(id, &items, window, cx)
                }
                Page::Movie { item, progress } => {
                    let (item, progress) = (item.clone(), *progress);
                    self.render_movie(&item, progress, window, cx)
                        .into_any_element()
                }
                Page::Show {
                    item,
                    episodes,
                    progress,
                    season,
                    art,
                    up_next,
                } => {
                    let (item, episodes, progress, season, art, up_next) = (
                        item.clone(),
                        episodes.clone(),
                        progress.clone(),
                        *season,
                        art.clone(),
                        *up_next,
                    );
                    self.render_show(
                        &item, &episodes, &progress, season, &art, up_next, window, cx,
                    )
                    .into_any_element()
                }
            }
        };
        // Fade in after navigation only, not when the view is shown again after playback.
        let fresh_page = self.page_changed_at.elapsed() < Duration::from_millis(300);
        let body = div().size_full().child(body).with_animation(
            ("page", self.page_epoch),
            Animation::new(theme::motion(260)).with_easing(ease_out_quint()),
            move |el, t| el.opacity(if fresh_page { t } else { 1. }),
        );

        let overlay = self
            .search_overlay
            .then(|| self.render_search_overlay(window, cx).into_any_element());
        let root = div()
            .track_focus(&self.focus)
            .relative()
            .size_full()
            .image_cache(self.image_cache.clone())
            .on_key_down(cx.listener(Self::on_library_key))
            .on_action(cx.listener(|_, _: &FocusNext, window, cx| window.focus_next(cx)))
            .on_action(cx.listener(|_, _: &FocusPrev, window, cx| window.focus_prev(cx)))
            .on_action(cx.listener(|this, _: &GoBack, _, cx| this.go_back(cx)))
            .on_action(cx.listener(|this, _: &GoForward, _, cx| this.go_forward(cx)))
            .on_action(cx.listener(|this, _: &ToggleSidebar, _, cx| this.toggle_sidebar(cx)))
            .on_action(cx.listener(|this, _: &OpenSearch, _, cx| this.open_search(cx)))
            .on_action(cx.listener(|this, _: &GoHome, _, cx| this.set_tab(Tab::Home, cx)))
            .on_action(cx.listener(|this, _: &GoMovies, _, cx| this.set_tab(Tab::Movies, cx)))
            .on_action(cx.listener(|this, _: &GoShows, _, cx| this.set_tab(Tab::Shows, cx)))
            // Mouse back/forward buttons.
            .on_mouse_down(
                gpui::MouseButton::Navigate(gpui::NavigationDirection::Back),
                cx.listener(|this, _, _, cx| this.go_back(cx)),
            )
            .on_mouse_down(
                gpui::MouseButton::Navigate(gpui::NavigationDirection::Forward),
                cx.listener(|this, _, _, cx| this.go_forward(cx)),
            )
            // Shelf drags: follow the pointer anywhere in the window, end on release.
            .on_mouse_move(cx.listener(|this, event: &gpui::MouseMoveEvent, _, cx| {
                this.drag_shelf(event, cx);
                this.drag_poster(event, cx);
                this.drag_nav(event, cx);
            }))
            .on_mouse_up(
                gpui::MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.end_shelf_drag(cx);
                    this.end_poster_drag(cx);
                    this.end_nav_drag(cx);
                }),
            )
            // Re-render on scroll so the nav backdrop follows the scroll position.
            .on_scroll(cx.listener(|_, _, _, cx| cx.notify()))
            .bg(theme::bg())
            .text_color(theme::text())
            .flex()
            .child(self.render_sidebar(window, cx))
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .child(body)
                    .child(self.render_top_bar()),
            )
            .children(overlay)
            .child(self.render_cast_pane(cx))
            .child(self.render_fix_match(window, cx))
            .child(self.render_card_menu(cx))
            .child(self.render_poster_drag());
        // Building the element tree should take a few milliseconds; report slow frames.
        let took = render_started.elapsed();
        if took > SLOW_RENDER {
            tracing::debug!(ms = took.as_secs_f32() * 1000., "slow library render");
        }
        root
    }
}

/// Pointer speed (px/s) over the last moments of a drag; 0 if it was held still before release.
fn fling_speed(samples: &VecDeque<(Instant, f32)>) -> f32 {
    let now = Instant::now();
    let (Some(&(first_at, first_x)), Some(&(last_at, last_x))) = (
        samples
            .iter()
            .find(|(t, _)| now.duration_since(*t) <= FLING_WINDOW),
        samples.back(),
    ) else {
        return 0.;
    };
    if now.duration_since(last_at) > FLING_STALE {
        return 0.;
    }
    let elapsed = last_at.duration_since(first_at).as_secs_f32();
    if elapsed < 0.008 {
        return 0.;
    }
    (last_x - first_x) / elapsed
}
