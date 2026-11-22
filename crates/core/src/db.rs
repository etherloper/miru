//! SQLite storage for the local library.
//!
//! Schema changes are append-only entries in [`MIGRATIONS`], tracked with `PRAGMA user_version`.

use std::collections::HashMap;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, OptionalExtension, Row, Transaction, params};

use crate::parse::ParsedMedia;

#[derive(Debug, thiserror::Error)]
pub enum DbError {
    #[error("database: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("unknown media kind {0:?} in database")]
    BadKind(String),
}

pub type Result<T> = std::result::Result<T, DbError>;

const MIGRATIONS: &[&str] = &[
    r"
    CREATE TABLE library_roots (
        id        INTEGER PRIMARY KEY,
        path      TEXT NOT NULL UNIQUE,
        added_at  INTEGER NOT NULL
    );

    CREATE TABLE media_files (
        id             INTEGER PRIMARY KEY,
        root_id        INTEGER NOT NULL REFERENCES library_roots(id) ON DELETE CASCADE,
        path           TEXT NOT NULL UNIQUE,
        size           INTEGER NOT NULL,
        mtime          INTEGER NOT NULL,
        kind           TEXT NOT NULL CHECK (kind IN ('movie', 'episode')),
        title          TEXT NOT NULL,   -- movie title, or show name for episodes
        year           INTEGER,
        season         INTEGER,
        episode        INTEGER,
        episode_end    INTEGER,
        episode_title  TEXT,
        added_at       INTEGER NOT NULL
    );
    CREATE INDEX media_files_root ON media_files(root_id);
    CREATE INDEX media_files_browse ON media_files(kind, title COLLATE NOCASE, season, episode);

    CREATE TABLE playback_state (
        file_id     INTEGER PRIMARY KEY REFERENCES media_files(id) ON DELETE CASCADE,
        position    REAL NOT NULL,
        duration    REAL NOT NULL,
        watched     INTEGER NOT NULL DEFAULT 0,
        updated_at  INTEGER NOT NULL  -- unix milliseconds
    );

    CREATE VIRTUAL TABLE media_search USING fts5(
        title, episode_title,
        content = 'media_files', content_rowid = 'id',
        tokenize = 'unicode61 remove_diacritics 2'
    );
    CREATE TRIGGER media_files_ai AFTER INSERT ON media_files BEGIN
        INSERT INTO media_search(rowid, title, episode_title)
        VALUES (new.id, new.title, new.episode_title);
    END;
    CREATE TRIGGER media_files_ad AFTER DELETE ON media_files BEGIN
        INSERT INTO media_search(media_search, rowid, title, episode_title)
        VALUES ('delete', old.id, old.title, old.episode_title);
    END;
    CREATE TRIGGER media_files_au AFTER UPDATE ON media_files BEGIN
        INSERT INTO media_search(media_search, rowid, title, episode_title)
        VALUES ('delete', old.id, old.title, old.episode_title);
        INSERT INTO media_search(rowid, title, episode_title)
        VALUES (new.id, new.title, new.episode_title);
    END;
",
    r"
    -- Online metadata per title: a movie is (title, year); a show is (title, year = 0).
    -- Rows exist for misses too, so lookups aren't retried on every launch.
    CREATE TABLE title_metadata (
        kind           TEXT NOT NULL CHECK (kind IN ('movie', 'show')),
        title          TEXT NOT NULL COLLATE NOCASE,
        year           INTEGER NOT NULL,
        status         TEXT NOT NULL CHECK (status IN ('matched', 'not_found')),
        tmdb_id        INTEGER,
        display_title  TEXT,
        overview       TEXT,
        poster_file    TEXT,
        backdrop_file  TEXT,
        accent_rgb     INTEGER,
        fetched_at     INTEGER NOT NULL,
        PRIMARY KEY (kind, title, year)
    );
",
    r"
    -- Per-episode artwork and details: TMDB stills/names/synopses, or a frame grabbed from the
    -- file. tmdb_checked records that TMDB was consulted, so frames can be upgraded once a
    -- key is configured without re-querying episodes TMDB does not have.
    CREATE TABLE episode_art (
        file_id       INTEGER PRIMARY KEY REFERENCES media_files(id) ON DELETE CASCADE,
        source        TEXT NOT NULL CHECK (source IN ('tmdb', 'frame', 'none')),
        still_file    TEXT,
        name          TEXT,
        overview      TEXT,
        runtime_min   INTEGER,
        tmdb_checked  INTEGER NOT NULL DEFAULT 0,
        fetched_at    INTEGER NOT NULL
    );
",
    r"
    -- User preferences as key/value text; typed access lives in settings.rs.
    CREATE TABLE settings (
        key    TEXT PRIMARY KEY,
        value  TEXT NOT NULL
    );
",
    r"
    -- Richer TMDB details; details_version lets a newer lookup refresh stored titles.
    ALTER TABLE title_metadata ADD COLUMN details_version INTEGER NOT NULL DEFAULT 0;
    ALTER TABLE title_metadata ADD COLUMN backdrops TEXT;
    ALTER TABLE title_metadata ADD COLUMN logo_file TEXT;
    ALTER TABLE title_metadata ADD COLUMN rating_tenths INTEGER;
    ALTER TABLE title_metadata ADD COLUMN certification TEXT;
    ALTER TABLE title_metadata ADD COLUMN cast_names TEXT;
    ALTER TABLE title_metadata ADD COLUMN genres TEXT;
    ALTER TABLE title_metadata ADD COLUMN runtime_min INTEGER;
    ALTER TABLE title_metadata ADD COLUMN release_year INTEGER;
",
    r"
    -- Titles the user matched by hand (Fix match): always looked up by this TMDB id, which
    -- may be of the other kind (a file parsed as a movie that is really a series).
    CREATE TABLE match_overrides (
        kind       TEXT NOT NULL CHECK (kind IN ('movie', 'show')),
        title      TEXT NOT NULL COLLATE NOCASE,
        year       INTEGER NOT NULL,
        tmdb_kind  TEXT NOT NULL CHECK (tmdb_kind IN ('movie', 'show')),
        tmdb_id    INTEGER NOT NULL,
        PRIMARY KEY (kind, title, year)
    );
",
    r"
    -- Audio and subtitle choices made while watching a show, reused for its other episodes.
    -- Languages when the tracks are tagged; per-file track ids as a fallback.
    CREATE TABLE show_track_prefs (
        show            TEXT PRIMARY KEY COLLATE NOCASE,
        audio_lang      TEXT,
        audio_track     INTEGER,
        subtitles_on    INTEGER,
        subtitle_lang   TEXT,
        subtitle_track  INTEGER
    );
",
    r"
    -- Full cast per title from TMDB, in billing order, for the cast pane and for finding an
    -- actor's other titles in the library.
    CREATE TABLE title_cast (
        kind          TEXT NOT NULL,
        title         TEXT NOT NULL COLLATE NOCASE,
        year          INTEGER NOT NULL,
        position      INTEGER NOT NULL,
        person_id     INTEGER NOT NULL,
        name          TEXT NOT NULL,
        character     TEXT,
        profile_path  TEXT,
        PRIMARY KEY (kind, title, year, position)
    );
    CREATE INDEX title_cast_person ON title_cast(person_id);
",
    r"
    -- Repair: episodes whose frames were grabbed before their show was matched were marked as
    -- checked against TMDB and never asked again. Ask once more for any without TMDB details.
    UPDATE episode_art SET tmdb_checked = 0 WHERE source != 'tmdb';
",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LibraryRoot {
    pub id: i64,
    pub path: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaFile {
    pub id: i64,
    pub path: String,
    pub media: ParsedMedia,
}

/// Size and modification time; a file is re-parsed only when either changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Fingerprint {
    pub size: i64,
    pub mtime: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MovieSummary {
    pub file_id: i64,
    pub title: String,
    pub year: Option<u16>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShowSummary {
    pub title: String,
    pub seasons: u32,
    pub episodes: u32,
}

pub struct Db {
    pub(crate) conn: Connection,
}

impl Db {
    pub fn open(path: &Path) -> Result<Self> {
        Self::init(Connection::open(path)?)
    }

    pub fn open_in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(mut conn: Connection) -> Result<Self> {
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "foreign_keys", true)?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        migrate(&mut conn)?;
        Ok(Self { conn })
    }

    /// Registers a library folder, returning the existing entry if already present.
    pub fn add_root(&self, path: &str) -> Result<LibraryRoot> {
        self.conn.execute(
            "INSERT INTO library_roots (path, added_at) VALUES (?1, ?2) ON CONFLICT(path) DO NOTHING",
            params![path, now()],
        )?;
        let id = self.conn.query_row(
            "SELECT id FROM library_roots WHERE path = ?1",
            [path],
            |r| r.get(0),
        )?;
        Ok(LibraryRoot {
            id,
            path: path.to_string(),
        })
    }

    pub fn roots(&self) -> Result<Vec<LibraryRoot>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, path FROM library_roots ORDER BY path")?;
        let rows = stmt.query_map([], |r| {
            Ok(LibraryRoot {
                id: r.get(0)?,
                path: r.get(1)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Starts a write batch; scans apply all changes for a root in one transaction.
    pub fn batch(&mut self) -> Result<Batch<'_>> {
        Ok(Batch {
            tx: self.conn.transaction()?,
        })
    }

    pub fn movies(&self) -> Result<Vec<MovieSummary>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, title, year FROM media_files WHERE kind = 'movie'
             ORDER BY title COLLATE NOCASE, year",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(MovieSummary {
                file_id: r.get(0)?,
                title: r.get(1)?,
                year: r.get(2)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn shows(&self) -> Result<Vec<ShowSummary>> {
        let mut stmt = self.conn.prepare(
            "SELECT title, COUNT(DISTINCT season), COUNT(*) FROM media_files WHERE kind = 'episode'
             GROUP BY title COLLATE NOCASE ORDER BY title COLLATE NOCASE",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(ShowSummary {
                title: r.get(0)?,
                seasons: r.get(1)?,
                episodes: r.get(2)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn episodes(&self, show: &str) -> Result<Vec<MediaFile>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {MEDIA_COLUMNS} FROM media_files
             WHERE kind = 'episode' AND title = ?1 COLLATE NOCASE ORDER BY season, episode"
        ))?;
        collect_media(stmt.query_map([show], media_row)?)
    }

    /// Prefix search over titles and episode titles, best matches first.
    pub fn search(&self, query: &str, limit: u32) -> Result<Vec<MediaFile>> {
        let Some(fts) = fts_query(query) else {
            return Ok(Vec::new());
        };
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {MEDIA_COLUMNS} FROM media_search s JOIN media_files ON media_files.id = s.rowid
             WHERE media_search MATCH ?1 ORDER BY s.rank LIMIT ?2"
        ))?;
        collect_media(stmt.query_map(params![fts, limit], media_row)?)
    }
}

pub struct Batch<'a> {
    tx: Transaction<'a>,
}

impl Batch<'_> {
    pub fn fingerprints(&self, root_id: i64) -> Result<HashMap<String, (i64, Fingerprint)>> {
        let mut stmt = self
            .tx
            .prepare("SELECT id, path, size, mtime FROM media_files WHERE root_id = ?1")?;
        let rows = stmt.query_map([root_id], |r| {
            Ok((
                r.get::<_, String>(1)?,
                (
                    r.get(0)?,
                    Fingerprint {
                        size: r.get(2)?,
                        mtime: r.get(3)?,
                    },
                ),
            ))
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Inserts or updates a file, keeping its id (and playback state) on update.
    pub fn upsert_file(
        &self,
        root_id: i64,
        path: &str,
        fingerprint: Fingerprint,
        media: &ParsedMedia,
    ) -> Result<()> {
        let c = MediaColumns::from(media);
        self.tx.execute(
            "INSERT INTO media_files
                (root_id, path, size, mtime, kind, title, year, season, episode, episode_end,
                 episode_title, added_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
             ON CONFLICT(path) DO UPDATE SET
                root_id = excluded.root_id, size = excluded.size, mtime = excluded.mtime,
                kind = excluded.kind, title = excluded.title, year = excluded.year,
                season = excluded.season, episode = excluded.episode,
                episode_end = excluded.episode_end, episode_title = excluded.episode_title",
            params![
                root_id,
                path,
                fingerprint.size,
                fingerprint.mtime,
                c.kind,
                c.title,
                c.year,
                c.season,
                c.episode,
                c.episode_end,
                c.episode_title,
                now()
            ],
        )?;
        Ok(())
    }

    pub fn delete_files(&self, ids: &[i64]) -> Result<()> {
        let mut stmt = self.tx.prepare("DELETE FROM media_files WHERE id = ?1")?;
        for id in ids {
            stmt.execute([id])?;
        }
        Ok(())
    }

    pub fn commit(self) -> Result<()> {
        Ok(self.tx.commit()?)
    }
}

fn migrate(conn: &mut Connection) -> Result<()> {
    let current: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
    let current = usize::try_from(current).unwrap_or(0);
    for (index, sql) in MIGRATIONS.iter().enumerate().skip(current) {
        let tx = conn.transaction()?;
        tx.execute_batch(sql)?;
        tx.pragma_update(None, "user_version", (index + 1) as i64)?;
        tx.commit()?;
        tracing::info!(version = index + 1, "applied database migration");
    }
    Ok(())
}

pub(crate) const MEDIA_COLUMNS: &str = "media_files.id, media_files.path, media_files.kind, media_files.title, \
    media_files.year, media_files.season, media_files.episode, media_files.episode_end, \
    media_files.episode_title";

/// Flat column view of [`ParsedMedia`] for writing.
struct MediaColumns<'a> {
    kind: &'static str,
    title: &'a str,
    year: Option<u16>,
    season: Option<u16>,
    episode: Option<u16>,
    episode_end: Option<u16>,
    episode_title: Option<&'a str>,
}

impl<'a> From<&'a ParsedMedia> for MediaColumns<'a> {
    fn from(media: &'a ParsedMedia) -> Self {
        match media {
            ParsedMedia::Movie { title, year } => Self {
                kind: "movie",
                title,
                year: *year,
                season: None,
                episode: None,
                episode_end: None,
                episode_title: None,
            },
            ParsedMedia::Episode {
                show,
                season,
                episode,
                episode_end,
                title,
            } => Self {
                kind: "episode",
                title: show,
                year: None,
                season: Some(*season),
                episode: Some(*episode),
                episode_end: *episode_end,
                episode_title: title.as_deref(),
            },
        }
    }
}

pub(crate) type RawMedia = (
    i64,
    String,
    String,
    String,
    Option<u16>,
    Option<u16>,
    Option<u16>,
    Option<u16>,
    Option<String>,
);

pub(crate) fn media_row(r: &Row<'_>) -> rusqlite::Result<RawMedia> {
    Ok((
        r.get(0)?,
        r.get(1)?,
        r.get(2)?,
        r.get(3)?,
        r.get(4)?,
        r.get(5)?,
        r.get(6)?,
        r.get(7)?,
        r.get(8)?,
    ))
}

pub(crate) fn collect_media(
    rows: impl Iterator<Item = rusqlite::Result<RawMedia>>,
) -> Result<Vec<MediaFile>> {
    rows.map(|row| {
        let (id, path, kind, title, year, season, episode, episode_end, episode_title) = row?;
        let media = match kind.as_str() {
            "movie" => ParsedMedia::Movie { title, year },
            "episode" => ParsedMedia::Episode {
                show: title,
                season: season.unwrap_or(0),
                episode: episode.unwrap_or(0),
                episode_end,
                title: episode_title,
            },
            _ => return Err(DbError::BadKind(kind)),
        };
        Ok(MediaFile { id, path, media })
    })
    .collect()
}

/// Turns free text into an FTS5 prefix query (`"dark"* "knight"*`), quoting each term so
/// user input can't inject FTS syntax.
pub(crate) fn fts_query(input: &str) -> Option<String> {
    let terms: Vec<String> = input
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(|t| format!("\"{t}\"*"))
        .collect();
    (!terms.is_empty()).then(|| terms.join(" "))
}

pub(crate) fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

/// Used by tests and tools to look up a single file.
impl Db {
    pub fn file_by_path(&self, path: &str) -> Result<Option<MediaFile>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {MEDIA_COLUMNS} FROM media_files WHERE path = ?1"
        ))?;
        let row = stmt.query_row([path], media_row).optional()?;
        Ok(collect_media(row.map(Ok).into_iter())?.pop())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FP: Fingerprint = Fingerprint {
        size: 100,
        mtime: 1,
    };

    fn ep(show: &str, season: u16, episode: u16, title: Option<&str>) -> ParsedMedia {
        ParsedMedia::Episode {
            show: show.into(),
            season,
            episode,
            episode_end: None,
            title: title.map(Into::into),
        }
    }

    fn seeded() -> Db {
        let mut db = Db::open_in_memory().unwrap();
        let root = db.add_root("/media").unwrap();
        let batch = db.batch().unwrap();
        let movie = |t: &str, y| ParsedMedia::Movie {
            title: t.into(),
            year: Some(y),
        };
        batch
            .upsert_file(
                root.id,
                "/media/dk.mkv",
                FP,
                &movie("The Dark Knight", 2008),
            )
            .unwrap();
        batch
            .upsert_file(root.id, "/media/am.mkv", FP, &movie("Amélie", 2001))
            .unwrap();
        batch
            .upsert_file(
                root.id,
                "/media/s1e2.mkv",
                FP,
                &ep("Dark", 1, 2, Some("Lies")),
            )
            .unwrap();
        batch
            .upsert_file(
                root.id,
                "/media/s1e1.mkv",
                FP,
                &ep("Dark", 1, 1, Some("Secrets")),
            )
            .unwrap();
        batch
            .upsert_file(root.id, "/media/s2e1.mkv", FP, &ep("Dark", 2, 1, None))
            .unwrap();
        batch.commit().unwrap();
        db
    }

    #[test]
    fn migrations_are_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("lib.db");
        Db::open(&path).unwrap().add_root("/a").unwrap();
        let db = Db::open(&path).unwrap();
        assert_eq!(db.roots().unwrap().len(), 1);
    }

    #[test]
    fn add_root_is_idempotent() {
        let db = Db::open_in_memory().unwrap();
        let a = db.add_root("/media").unwrap();
        let b = db.add_root("/media").unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn browse_movies_and_shows() {
        let db = seeded();
        let titles: Vec<_> = db.movies().unwrap().into_iter().map(|m| m.title).collect();
        assert_eq!(titles, ["Amélie", "The Dark Knight"]);
        assert_eq!(
            db.shows().unwrap(),
            [ShowSummary {
                title: "Dark".into(),
                seasons: 2,
                episodes: 3
            }]
        );
        let order: Vec<_> = db
            .episodes("dark")
            .unwrap()
            .into_iter()
            .map(|f| match f.media {
                ParsedMedia::Episode {
                    season, episode, ..
                } => (season, episode),
                ParsedMedia::Movie { .. } => panic!("expected episode"),
            })
            .collect();
        assert_eq!(order, [(1, 1), (1, 2), (2, 1)]);
    }

    #[test]
    fn search_uses_prefixes_diacritics_and_episode_titles() {
        let db = seeded();
        let paths = |q: &str| -> Vec<String> {
            let mut v: Vec<_> = db
                .search(q, 10)
                .unwrap()
                .into_iter()
                .map(|f| f.path)
                .collect();
            v.sort();
            v
        };
        assert_eq!(paths("knig"), ["/media/dk.mkv"]);
        assert_eq!(paths("amelie"), ["/media/am.mkv"]);
        assert_eq!(paths("secrets"), ["/media/s1e1.mkv"]);
        assert_eq!(paths("dark").len(), 4);
        assert!(paths("\"; DROP TABLE x --").is_empty());
        assert!(paths("   ").is_empty());
    }

    #[test]
    fn upsert_updates_in_place_and_search_follows() {
        let mut db = seeded();
        let before = db.file_by_path("/media/dk.mkv").unwrap().unwrap();
        let root = db.add_root("/media").unwrap();
        let batch = db.batch().unwrap();
        let renamed = ParsedMedia::Movie {
            title: "Batman Begins".into(),
            year: Some(2005),
        };
        batch
            .upsert_file(root.id, "/media/dk.mkv", FP, &renamed)
            .unwrap();
        batch.commit().unwrap();
        let after = db.file_by_path("/media/dk.mkv").unwrap().unwrap();
        assert_eq!(before.id, after.id);
        assert_eq!(after.media, renamed);
        assert!(db.search("knight", 10).unwrap().is_empty());
        assert_eq!(db.search("batman", 10).unwrap().len(), 1);
    }

    #[test]
    fn delete_removes_from_search() {
        let mut db = seeded();
        let id = db.file_by_path("/media/am.mkv").unwrap().unwrap().id;
        let batch = db.batch().unwrap();
        batch.delete_files(&[id]).unwrap();
        batch.commit().unwrap();
        assert!(db.search("amelie", 10).unwrap().is_empty());
    }
}
