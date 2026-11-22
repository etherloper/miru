//! User preferences and library folder management.

use rusqlite::{OptionalExtension, params};

use crate::db::{Db, LibraryRoot, Result};

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
            Self::Small => 0.8,
            Self::Medium => 1.0,
            Self::Large => 1.25,
            Self::ExtraLarge => 1.5,
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

/// How a library grid is ordered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LibrarySort {
    #[default]
    Title,
    RecentlyAdded,
    Newest,
    Oldest,
}

impl LibrarySort {
    pub const ALL: [Self; 4] = [Self::Title, Self::RecentlyAdded, Self::Newest, Self::Oldest];

    pub fn label(self) -> &'static str {
        match self {
            Self::Title => "A–Z",
            Self::RecentlyAdded => "Recently Added",
            Self::Newest => "Newest",
            Self::Oldest => "Oldest",
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Title => "title",
            Self::RecentlyAdded => "recent",
            Self::Newest => "newest",
            Self::Oldest => "oldest",
        }
    }

    fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|v| v.as_str() == s)
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
    /// Cycle through featured titles on Home.
    pub rotate_featured: bool,
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
    /// Check for a newer version on launch (at most once a day).
    pub auto_update: bool,
    /// When updates were last checked (unix seconds; 0 = never).
    pub update_checked_at: i64,
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
            rotate_featured: true,
            autoplay_next: true,
            sidebar_collapsed: false,
            movie_sort: LibrarySort::Title,
            show_sort: LibrarySort::Title,
            volume: 100,
            tmdb_key: None,
            auto_update: true,
            update_checked_at: 0,
        }
    }
}

const SEEK_STEP: &str = "playback.seek_step";
const SUBTITLES: &str = "playback.subtitles_by_default";
const AUDIO_OUTPUT: &str = "playback.audio_output";
const AUDIO_LANG: &str = "playback.audio_language";
const SUB_LANG: &str = "playback.subtitle_language";
const SUB_SIZE: &str = "playback.subtitle_size";
const ROTATE: &str = "home.rotate_featured";
const AUTOPLAY: &str = "playback.autoplay_next";
const SIDEBAR: &str = "ui.sidebar_collapsed";
const PARSER_VERSION_KEY: &str = "scan.parser_version";
const MOVIE_SORT: &str = "library.movie_sort";
const SHOW_SORT: &str = "library.show_sort";
const VOLUME: &str = "playback.volume";
const TMDB_KEY: &str = "metadata.tmdb_key";
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
            rotate_featured: bool_of(self.setting(ROTATE)?, defaults.rotate_featured),
            autoplay_next: bool_of(self.setting(AUTOPLAY)?, defaults.autoplay_next),
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
            (ROTATE, s.rotate_featured.to_string()),
            (AUTOPLAY, s.autoplay_next.to_string()),
            (SIDEBAR, s.sidebar_collapsed.to_string()),
            (MOVIE_SORT, s.movie_sort.as_str().to_string()),
            (SHOW_SORT, s.show_sort.as_str().to_string()),
            (VOLUME, s.volume.to_string()),
            (TMDB_KEY, s.tmdb_key.clone().unwrap_or_default()),
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
            rotate_featured: false,
            autoplay_next: false,
            sidebar_collapsed: true,
            movie_sort: LibrarySort::Newest,
            show_sort: LibrarySort::RecentlyAdded,
            volume: 40,
            tmdb_key: Some("abc".into()),
            auto_update: false,
            update_checked_at: 1_700_000_000,
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
}
