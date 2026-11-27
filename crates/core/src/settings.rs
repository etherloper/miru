//! User preferences and library folder management.

use rusqlite::{OptionalExtension, params};

use crate::db::{Db, LibraryRoot, Result};
use crate::ratings::rating_site;

/// How multichannel audio reaches the output device.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AudioOutput {
    /// Downmix to stereo: centre-channel dialogue is always audible.
    #[default]
    Stereo,
    /// Send the file's own layout (5.1 stays 5.1).
    Source,
    /// Upmix to 7.1, the layout headset virtualisers usually expect.
    Surround71,
}

impl AudioOutput {
    pub const ALL: [Self; 3] = [Self::Stereo, Self::Source, Self::Surround71];

    pub fn label(self) -> &'static str {
        match self {
            Self::Stereo => "Stereo",
            Self::Source => "Source",
            Self::Surround71 => "7.1",
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Stereo => "stereo",
            Self::Source => "source",
            Self::Surround71 => "7.1",
        }
    }

    fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|v| v.as_str() == s)
    }
}

/// Size of text subtitles, relative to mpv's default.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SubtitleSize {
    Small,
    #[default]
    Medium,
    Large,
    ExtraLarge,
}

impl SubtitleSize {
    pub const ALL: [Self; 4] = [Self::Small, Self::Medium, Self::Large, Self::ExtraLarge];

    pub fn label(self) -> &'static str {
        match self {
            Self::Small => "Small",
            Self::Medium => "Medium",
            Self::Large => "Large",
            Self::ExtraLarge => "Extra Large",
        }
    }

    /// Multiplier for mpv's `sub-scale`.
    pub fn scale(self) -> f64 {
        match self {
            Self::Small => 0.7,
            Self::Medium => 0.85,
            Self::Large => 1.0,
            Self::ExtraLarge => 1.2,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Small => "small",
            Self::Medium => "medium",
            Self::Large => "large",
            Self::ExtraLarge => "xl",
        }
    }

    fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|v| v.as_str() == s)
    }
}

/// How text subtitles are drawn against the picture.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SubtitleStyle {
    /// A solid dark outline round each letter.
    #[default]
    Outline,
    /// A thin outline and a soft drop shadow.
    Shadow,
    /// Text on a translucent dark box.
    Box,
}

impl SubtitleStyle {
    pub const ALL: [Self; 3] = [Self::Outline, Self::Shadow, Self::Box];

    pub fn label(self) -> &'static str {
        match self {
            Self::Outline => "Outline",
            Self::Shadow => "Drop Shadow",
            Self::Box => "Background",
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Outline => "outline",
            Self::Shadow => "shadow",
            Self::Box => "box",
        }
    }

    fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|v| v.as_str() == s)
    }
}

/// Colour of text subtitles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SubtitleColor {
    #[default]
    White,
    Yellow,
}

impl SubtitleColor {
    pub const ALL: [Self; 2] = [Self::White, Self::Yellow];

    pub fn label(self) -> &'static str {
        match self {
            Self::White => "White",
            Self::Yellow => "Yellow",
        }
    }

    /// 0xRRGGBB.
    pub fn rgb(self) -> u32 {
        match self {
            Self::White => 0xffffff,
            Self::Yellow => 0xffe14d,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::White => "white",
            Self::Yellow => "yellow",
        }
    }

    fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|v| v.as_str() == s)
    }
}

/// How a library grid is ordered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LibrarySort {
    #[default]
    Title,
    RecentlyAdded,
    Newest,
    Oldest,
    /// Highest TMDB audience rating first.
    TopRated,
    /// Highest rating on one outside site first (an id from `RATING_SITES`).
    Site(&'static str),
}

impl LibrarySort {
    /// The orders in the Sort menu; site orders are picked from the Rating menu.
    pub const ALL: [Self; 5] = [
        Self::Title,
        Self::RecentlyAdded,
        Self::Newest,
        Self::Oldest,
        Self::TopRated,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Title => "A–Z",
            Self::RecentlyAdded => "Recently Added",
            Self::Newest => "Newest",
            Self::Oldest => "Oldest",
            Self::TopRated => "Top Rated",
            Self::Site(id) => rating_site(id).map_or(id, |site| site.name),
        }
    }

    fn stored(self) -> String {
        match self {
            Self::Title => "title".into(),
            Self::RecentlyAdded => "recent".into(),
            Self::Newest => "newest".into(),
            Self::Oldest => "oldest".into(),
            Self::TopRated => "rating".into(),
            Self::Site(id) => format!("site:{id}"),
        }
    }

    fn parse(s: &str) -> Option<Self> {
        if let Some(id) = s.strip_prefix("site:") {
            return rating_site(id).map(|site| Self::Site(site.id));
        }
        Self::ALL.into_iter().find(|v| v.stored() == s)
    }
}

/// Sort key for titles that ignores a leading article, so "The Matrix" files under M.
pub fn title_sort_key(title: &str) -> String {
    let lower = title.trim().to_lowercase();
    for article in ["the ", "a ", "an "] {
        if let Some(rest) = lower.strip_prefix(article)
            && !rest.is_empty()
        {
            return rest.to_string();
        }
    }
    lower
}

/// A sidebar destination the user can reorder (Settings always stays at the bottom).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavItem {
    Search,
    Home,
    Movies,
    Shows,
    Watchlist,
}

impl NavItem {
    pub const ALL: [Self; 5] = [
        Self::Search,
        Self::Home,
        Self::Movies,
        Self::Shows,
        Self::Watchlist,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Search => "Search",
            Self::Home => "Home",
            Self::Movies => "Movies",
            Self::Shows => "TV Shows",
            Self::Watchlist => "Watchlist",
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Search => "search",
            Self::Home => "home",
            Self::Movies => "movies",
            Self::Shows => "shows",
            Self::Watchlist => "watchlist",
        }
    }

    fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|v| v.as_str() == s)
    }

    /// Reads a stored order: unknown names and repeats are dropped, and anything missing is
    /// added at the end, so every item appears exactly once.
    fn parse_order(s: &str) -> Vec<Self> {
        let mut order: Vec<Self> = Vec::new();
        for item in s.split(',').filter_map(|v| Self::parse(v.trim())) {
            if !order.contains(&item) {
                order.push(item);
            }
        }
        for item in Self::ALL {
            if !order.contains(&item) {
                order.push(item);
            }
        }
        order
    }
}

/// Seek steps offered in Settings, in seconds.
pub const SEEK_STEPS: [u32; 3] = [10, 15, 30];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    /// Seconds jumped by the skip buttons and arrow keys.
    pub seek_step: u32,
    /// Turn subtitles on automatically when a file has them.
    pub subtitles_by_default: bool,
    /// How multichannel audio is sent to the output device.
    pub audio_output: AudioOutput,
    /// ISO 639 code, e.g. "en". `None` uses the file's default track.
    pub audio_language: Option<String>,
    pub subtitle_language: Option<String>,
    pub subtitle_size: SubtitleSize,
    pub subtitle_style: SubtitleStyle,
    pub subtitle_color: SubtitleColor,
    /// Cycle through featured titles on Home.
    pub rotate_featured: bool,
    /// Feature titles from the whole library on Home, not just the recently added.
    pub feature_whole_library: bool,
    /// Sidebar destinations, top to bottom.
    pub sidebar_order: Vec<NavItem>,
    /// Start the next episode automatically when one finishes.
    pub autoplay_next: bool,
    /// Sidebar shows icons only.
    pub sidebar_collapsed: bool,
    pub movie_sort: LibrarySort,
    pub show_sort: LibrarySort,
    /// Player volume (0–100), kept from one video to the next.
    pub volume: u32,
    /// The user's own TMDB key, used instead of any built-in one.
    pub tmdb_key: Option<String>,
    /// MDBList key, for ratings from IMDb, Rotten Tomatoes, Metacritic and other sites.
    pub mdblist_key: Option<String>,
    /// Which sites' ratings detail pages show (MDBList names, e.g. "imdb", "tomatoes").
    pub rating_sources: Vec<String>,
    /// Check for a newer version on launch (at most once a day).
    pub auto_update: bool,
    /// When updates were last checked (unix seconds; 0 = never).
    pub update_checked_at: i64,
    /// Show pages list episodes in rows instead of a row of cards.
    pub episode_list: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            seek_step: 10,
            subtitles_by_default: true,
            audio_output: AudioOutput::Stereo,
            audio_language: None,
            subtitle_language: None,
            subtitle_size: SubtitleSize::Medium,
            subtitle_style: SubtitleStyle::Outline,
            subtitle_color: SubtitleColor::White,
            rotate_featured: true,
            feature_whole_library: false,
            sidebar_order: NavItem::ALL.to_vec(),
            autoplay_next: true,
            sidebar_collapsed: false,
            movie_sort: LibrarySort::Title,
            show_sort: LibrarySort::Title,
            volume: 100,
            tmdb_key: None,
            mdblist_key: None,
            rating_sources: DEFAULT_RATING_SOURCES
                .iter()
                .map(|s| s.to_string())
                .collect(),
            auto_update: true,
            update_checked_at: 0,
            episode_list: false,
        }
    }
}

const SEEK_STEP: &str = "playback.seek_step";
const SUBTITLES: &str = "playback.subtitles_by_default";
const AUDIO_OUTPUT: &str = "playback.audio_output";
const AUDIO_LANG: &str = "playback.audio_language";
const SUB_LANG: &str = "playback.subtitle_language";
const SUB_SIZE: &str = "playback.subtitle_size";
const SUB_STYLE: &str = "playback.subtitle_style";
const SUB_COLOR: &str = "playback.subtitle_color";
const ROTATE: &str = "home.rotate_featured";
const WHOLE_LIBRARY: &str = "home.feature_whole_library";
const SIDEBAR_ORDER: &str = "ui.sidebar_order";
const AUTOPLAY: &str = "playback.autoplay_next";
const EPISODE_LIST: &str = "library.episode_list";
const SIDEBAR: &str = "ui.sidebar_collapsed";
const PARSER_VERSION_KEY: &str = "scan.parser_version";
const MOVIE_SORT: &str = "library.movie_sort";
const SHOW_SORT: &str = "library.show_sort";
const VOLUME: &str = "playback.volume";
const TMDB_KEY: &str = "metadata.tmdb_key";
const MDBLIST_KEY: &str = "metadata.mdblist_key";
const RATING_SOURCES: &str = "metadata.rating_sources";
/// Ratings shown until the user picks their own.
pub const DEFAULT_RATING_SOURCES: [&str; 3] = ["imdb", "tomatoes", "metacritic"];
const AUTO_UPDATE: &str = "updates.auto";
const UPDATE_CHECKED: &str = "updates.checked_at";

/// A library folder with how many files it contributes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RootStats {
    pub root: LibraryRoot,
    pub movies: u32,
    pub episodes: u32,
}

impl Db {
    /// Loads settings, falling back to defaults for anything unset or unreadable.
    pub fn settings(&self) -> Result<Settings> {
        let defaults = Settings::default();
        let bool_of = |v: Option<String>, d: bool| v.map_or(d, |v| v == "true");
        Ok(Settings {
            seek_step: self
                .setting(SEEK_STEP)?
                .and_then(|v| v.parse().ok())
                .filter(|s| SEEK_STEPS.contains(s))
                .unwrap_or(defaults.seek_step),
            subtitles_by_default: bool_of(self.setting(SUBTITLES)?, defaults.subtitles_by_default),
            audio_output: self
                .setting(AUDIO_OUTPUT)?
                .and_then(|v| AudioOutput::parse(&v))
                .unwrap_or_default(),
            audio_language: self.setting(AUDIO_LANG)?.filter(|v| !v.is_empty()),
            subtitle_language: self.setting(SUB_LANG)?.filter(|v| !v.is_empty()),
            subtitle_size: self
                .setting(SUB_SIZE)?
                .and_then(|v| SubtitleSize::parse(&v))
                .unwrap_or_default(),
            subtitle_style: self
                .setting(SUB_STYLE)?
                .and_then(|v| SubtitleStyle::parse(&v))
                .unwrap_or_default(),
            subtitle_color: self
                .setting(SUB_COLOR)?
                .and_then(|v| SubtitleColor::parse(&v))
                .unwrap_or_default(),
            rotate_featured: bool_of(self.setting(ROTATE)?, defaults.rotate_featured),
            feature_whole_library: bool_of(
                self.setting(WHOLE_LIBRARY)?,
                defaults.feature_whole_library,
            ),
            sidebar_order: self
                .setting(SIDEBAR_ORDER)?
                .map_or(defaults.sidebar_order, |v| NavItem::parse_order(&v)),
            autoplay_next: bool_of(self.setting(AUTOPLAY)?, defaults.autoplay_next),
            episode_list: bool_of(self.setting(EPISODE_LIST)?, defaults.episode_list),
            sidebar_collapsed: bool_of(self.setting(SIDEBAR)?, defaults.sidebar_collapsed),
            movie_sort: self
                .setting(MOVIE_SORT)?
                .and_then(|v| LibrarySort::parse(&v))
                .unwrap_or_default(),
            show_sort: self
                .setting(SHOW_SORT)?
                .and_then(|v| LibrarySort::parse(&v))
                .unwrap_or_default(),
            volume: self
                .setting(VOLUME)?
                .and_then(|v| v.parse::<u32>().ok())
                .filter(|v| *v <= 100)
                .unwrap_or(defaults.volume),
            tmdb_key: self
                .setting(TMDB_KEY)?
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty()),
            mdblist_key: self
                .setting(MDBLIST_KEY)?
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty()),
            rating_sources: self.setting(RATING_SOURCES)?.map_or_else(
                || defaults.rating_sources.clone(),
                |v| {
                    v.split(',')
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(str::to_string)
                        .collect()
                },
            ),
            auto_update: bool_of(self.setting(AUTO_UPDATE)?, defaults.auto_update),
            update_checked_at: self
                .setting(UPDATE_CHECKED)?
                .and_then(|v| v.parse().ok())
                .unwrap_or(0),
        })
    }

    pub fn save_settings(&self, s: &Settings) -> Result<()> {
        let pairs = [
            (SEEK_STEP, s.seek_step.to_string()),
            (SUBTITLES, s.subtitles_by_default.to_string()),
            (AUDIO_OUTPUT, s.audio_output.as_str().to_string()),
            (AUDIO_LANG, s.audio_language.clone().unwrap_or_default()),
            (SUB_LANG, s.subtitle_language.clone().unwrap_or_default()),
            (SUB_SIZE, s.subtitle_size.as_str().to_string()),
            (SUB_STYLE, s.subtitle_style.as_str().to_string()),
            (SUB_COLOR, s.subtitle_color.as_str().to_string()),
            (ROTATE, s.rotate_featured.to_string()),
            (WHOLE_LIBRARY, s.feature_whole_library.to_string()),
            (
                SIDEBAR_ORDER,
                s.sidebar_order
                    .iter()
                    .map(|i| i.as_str())
                    .collect::<Vec<_>>()
                    .join(","),
            ),
            (AUTOPLAY, s.autoplay_next.to_string()),
            (EPISODE_LIST, s.episode_list.to_string()),
            (SIDEBAR, s.sidebar_collapsed.to_string()),
            (MOVIE_SORT, s.movie_sort.stored()),
            (SHOW_SORT, s.show_sort.stored()),
            (VOLUME, s.volume.to_string()),
            (TMDB_KEY, s.tmdb_key.clone().unwrap_or_default()),
            (MDBLIST_KEY, s.mdblist_key.clone().unwrap_or_default()),
            (RATING_SOURCES, s.rating_sources.join(",")),
            (AUTO_UPDATE, s.auto_update.to_string()),
            (UPDATE_CHECKED, s.update_checked_at.to_string()),
        ];
        for (key, value) in pairs {
            self.conn.execute(
                "INSERT INTO settings (key, value) VALUES (?1, ?2)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![key, value],
            )?;
        }
        Ok(())
    }

    /// Parser version the stored titles were produced with (0 if never recorded).
    pub fn parser_version(&self) -> Result<u32> {
        Ok(self
            .setting(PARSER_VERSION_KEY)?
            .and_then(|v| v.parse().ok())
            .unwrap_or(0))
    }

    pub fn set_parser_version(&self, version: u32) -> Result<()> {
        self.conn.execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![PARSER_VERSION_KEY, version.to_string()],
        )?;
        Ok(())
    }

    fn setting(&self, key: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row("SELECT value FROM settings WHERE key = ?1", [key], |r| {
                r.get(0)
            })
            .optional()?)
    }

    /// Library folders with file counts, alphabetically.
    pub fn root_stats(&self) -> Result<Vec<RootStats>> {
        let mut stmt = self.conn.prepare(
            "SELECT r.id, r.path,
                    COUNT(CASE WHEN f.kind = 'movie' THEN 1 END),
                    COUNT(CASE WHEN f.kind = 'episode' THEN 1 END)
             FROM library_roots r LEFT JOIN media_files f ON f.root_id = r.id
             GROUP BY r.id ORDER BY r.path COLLATE NOCASE",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(RootStats {
                root: LibraryRoot {
                    id: r.get(0)?,
                    path: r.get(1)?,
                },
                movies: r.get(2)?,
                episodes: r.get(3)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Removes a library folder and everything indexed from it (including watch progress).
    /// Files on disk are not touched.
    pub fn remove_root(&self, root_id: i64) -> Result<()> {
        self.conn
            .execute("DELETE FROM library_roots WHERE id = ?1", [root_id])?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Fingerprint;
    use crate::parse::ParsedMedia;

    #[test]
    fn settings_round_trip_with_defaults() {
        let db = Db::open_in_memory().unwrap();
        assert_eq!(db.settings().unwrap(), Settings::default());

        let custom = Settings {
            seek_step: 30,
            subtitles_by_default: false,
            audio_output: AudioOutput::Surround71,
            audio_language: Some("ja".into()),
            subtitle_language: Some("en".into()),
            subtitle_size: SubtitleSize::ExtraLarge,
            subtitle_style: SubtitleStyle::Box,
            subtitle_color: SubtitleColor::Yellow,
            rotate_featured: false,
            feature_whole_library: true,
            sidebar_order: vec![
                NavItem::Home,
                NavItem::Shows,
                NavItem::Movies,
                NavItem::Search,
                NavItem::Watchlist,
            ],
            autoplay_next: false,
            sidebar_collapsed: true,
            movie_sort: LibrarySort::Newest,
            show_sort: LibrarySort::RecentlyAdded,
            volume: 40,
            tmdb_key: Some("abc".into()),
            mdblist_key: Some("def".into()),
            rating_sources: vec!["letterboxd".into(), "imdb".into()],
            auto_update: false,
            update_checked_at: 1_700_000_000,
            episode_list: true,
        };
        db.save_settings(&custom).unwrap();
        assert_eq!(db.settings().unwrap(), custom);

        // Clearing a language stores empty text, read back as None.
        db.save_settings(&Settings {
            audio_language: None,
            ..custom.clone()
        })
        .unwrap();
        assert_eq!(db.settings().unwrap().audio_language, None);
    }

    #[test]
    fn sidebar_order_is_repaired() {
        use NavItem::*;
        assert_eq!(
            NavItem::parse_order("shows,home"),
            vec![Shows, Home, Search, Movies, Watchlist]
        );
        assert_eq!(
            NavItem::parse_order("movies, bogus,movies,search"),
            vec![Movies, Search, Home, Shows, Watchlist]
        );
        assert_eq!(NavItem::parse_order(""), NavItem::ALL.to_vec());
    }

    #[test]
    fn invalid_seek_step_falls_back() {
        let db = Db::open_in_memory().unwrap();
        db.conn
            .execute(
                "INSERT INTO settings (key, value) VALUES ('playback.seek_step', '7')",
                [],
            )
            .unwrap();
        assert_eq!(db.settings().unwrap().seek_step, 10);
    }

    #[test]
    fn root_stats_and_removal() {
        let mut db = Db::open_in_memory().unwrap();
        let movies = db.add_root("/movies").unwrap();
        let tv = db.add_root("/tv").unwrap();
        let empty = db.add_root("/empty").unwrap();
        let fp = Fingerprint { size: 1, mtime: 1 };
        let b = db.batch().unwrap();
        b.upsert_file(
            movies.id,
            "/movies/a.mkv",
            fp,
            &ParsedMedia::Movie {
                title: "A".into(),
                year: None,
            },
        )
        .unwrap();
        let ep = ParsedMedia::Episode {
            show: "S".into(),
            season: 1,
            episode: 1,
            episode_end: None,
            title: None,
        };
        b.upsert_file(tv.id, "/tv/s1e1.mkv", fp, &ep).unwrap();
        b.commit().unwrap();
        let tv_file = db.file_by_path("/tv/s1e1.mkv").unwrap().unwrap().id;
        db.save_progress(tv_file, 100., 1000.).unwrap();

        let stats = db.root_stats().unwrap();
        let summary: Vec<_> = stats
            .iter()
            .map(|s| (s.root.path.as_str(), s.movies, s.episodes))
            .collect();
        assert_eq!(
            summary,
            [("/empty", 0, 0), ("/movies", 1, 0), ("/tv", 0, 1)]
        );

        db.remove_root(tv.id).unwrap();
        assert!(db.file_by_path("/tv/s1e1.mkv").unwrap().is_none());
        assert_eq!(
            db.progress(tv_file).unwrap(),
            None,
            "progress goes with the file"
        );
        assert_eq!(db.root_stats().unwrap().len(), 2);
        db.remove_root(empty.id).unwrap();
        assert_eq!(db.roots().unwrap(), [movies]);
    }
}

#[cfg(test)]
mod sort_tests {
    use super::*;

    #[test]
    fn title_sort_ignores_leading_articles() {
        assert_eq!(title_sort_key("The Matrix"), "matrix");
        assert_eq!(title_sort_key("A Quiet Place"), "quiet place");
        assert_eq!(title_sort_key("An Education"), "education");
        assert_eq!(title_sort_key("Theodore"), "theodore");
        assert_eq!(title_sort_key("The"), "the");
    }

    #[test]
    fn site_sorts_round_trip_and_unknown_sites_are_dropped() {
        let sort = LibrarySort::Site("imdb");
        assert_eq!(LibrarySort::parse(&sort.stored()), Some(sort));
        assert_eq!(sort.label(), "IMDb");
        assert_eq!(LibrarySort::parse("site:nowhere"), None);
        assert_eq!(LibrarySort::parse("rating"), Some(LibrarySort::TopRated));
    }
}
