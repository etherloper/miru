//! Browse-level view of the library: one item per movie or show, joined with metadata.

use rusqlite::{OptionalExtension, params};

use crate::db::{Db, Result, now};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ItemKind {
    Movie,
    Show,
}

impl ItemKind {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Movie => "movie",
            Self::Show => "show",
        }
    }
}

/// Identity of a browseable title. Shows are grouped by name only, so their year is `None`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TitleKey {
    pub kind: ItemKind,
    pub title: String,
    pub year: Option<u16>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Metadata {
    pub tmdb_id: Option<i64>,
    pub display_title: Option<String>,
    pub overview: Option<String>,
    /// Local cached artwork files.
    pub poster_file: Option<String>,
    pub backdrop_file: Option<String>,
    /// Average poster colour (0xRRGGBB), used for placeholders and accents.
    pub accent_rgb: Option<u32>,
    /// Extra cached backdrops (textless where available), for variety between launches.
    pub backdrops: Vec<String>,
    /// Cached title logo (transparent PNG), shown instead of plain text titles.
    pub logo_file: Option<String>,
    /// Audience rating out of 10, in tenths (78 = 7.8).
    pub rating_tenths: Option<u16>,
    /// Age rating, e.g. "15" or "PG-13".
    pub certification: Option<String>,
    pub cast: Vec<String>,
    /// Full cast with TMDB ids and photos. Filled by lookups and saved to its own table by
    /// `save_metadata`; empty when metadata is read back (use `Db::cast`).
    pub credits: Vec<CastMember>,
    /// Directors (movies) or creators (shows), with the job in `character`. Saved and read
    /// back like `credits` (use `Db::crew`).
    pub crew: Vec<CastMember>,
    pub genres: Vec<String>,
    pub runtime_min: Option<u32>,
    pub release_year: Option<u16>,
}

/// One cast member of a title.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CastMember {
    /// TMDB person id, shared across titles.
    pub person_id: i64,
    pub name: String,
    pub character: Option<String>,
    /// TMDB profile image path (e.g. `/abc.jpg`), downloaded on demand.
    pub profile_path: Option<String>,
}

/// A person found by name in the cast or crew of stored titles.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersonMatch {
    pub person_id: i64,
    pub name: String,
    pub profile_path: Option<String>,
    /// Plays a part in at least one title.
    pub acts: bool,
    /// Crew jobs held across titles, e.g. "Director".
    pub jobs: Vec<String>,
}

impl PersonMatch {
    pub fn as_member(&self) -> CastMember {
        CastMember {
            person_id: self.person_id,
            name: self.name.clone(),
            character: None,
            profile_path: self.profile_path.clone(),
        }
    }
}

/// Where crew rows start in a title's cast table, after the cast.
const CREW_POSITION: usize = 1000;

/// Bump when lookups gain new fields or better matching, so stored titles are fetched again.
/// 5: directors and creators.
pub const METADATA_VERSION: u32 = 5;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LibraryItem {
    pub key: TitleKey,
    /// Movie: the file to play (lowest id when duplicated). Show: unused, episodes are listed.
    pub file_id: i64,
    /// Number of files: copies of a movie, or episodes of a show.
    pub file_count: u32,
    pub added_at: i64,
    /// `None` until a lookup has been attempted.
    pub metadata: Option<Metadata>,
    /// Version of the lookup that produced `metadata` (0 when none).
    pub metadata_version: u32,
}

impl LibraryItem {
    pub fn display_title(&self) -> &str {
        self.metadata
            .as_ref()
            .and_then(|m| m.display_title.as_deref())
            .unwrap_or(&self.key.title)
    }
}

// Short-lived (one lookup, then saved), so the size difference between variants is harmless.
#[allow(clippy::large_enum_variant)]
pub enum MetadataResult {
    Matched(Metadata),
    NotFound,
}

/// Metadata columns in the order `metadata_row` reads them.
const METADATA_COLUMNS: &str = "m.tmdb_id, m.display_title, m.overview, m.poster_file, m.backdrop_file, \n    m.accent_rgb, m.backdrops, m.logo_file, m.rating_tenths, m.certification, m.cast_names, \n    m.genres, m.runtime_min, m.release_year";

/// Reads `METADATA_COLUMNS` starting at column `at`. List columns are newline-separated text.
fn metadata_row(r: &rusqlite::Row<'_>, at: usize) -> rusqlite::Result<Metadata> {
    let list = |i: usize| -> rusqlite::Result<Vec<String>> {
        Ok(r.get::<_, Option<String>>(at + i)?
            .map(|s| {
                s.lines()
                    .filter(|l| !l.is_empty())
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default())
    };
    Ok(Metadata {
        tmdb_id: r.get(at)?,
        display_title: r.get(at + 1)?,
        overview: r.get(at + 2)?,
        poster_file: r.get(at + 3)?,
        backdrop_file: r.get(at + 4)?,
        accent_rgb: r.get(at + 5)?,
        backdrops: list(6)?,
        logo_file: r.get(at + 7)?,
        rating_tenths: r.get(at + 8)?,
        certification: r.get(at + 9)?,
        cast: list(10)?,
        credits: Vec::new(),
        crew: Vec::new(),
        genres: list(11)?,
        runtime_min: r.get(at + 12)?,
        release_year: r.get(at + 13)?,
    })
}

const ITEM_SELECT: &str = "
    SELECT f.title, f.year, MIN(f.id), COUNT(*), MAX(f.added_at), m.status, m.details_version,
           m.tmdb_id, m.display_title, m.overview, m.poster_file, m.backdrop_file, m.accent_rgb,
           m.backdrops, m.logo_file, m.rating_tenths, m.certification, m.cast_names, m.genres,
           m.runtime_min, m.release_year
    FROM media_files f
    LEFT JOIN title_metadata m
      ON m.kind = ?1 AND m.title = f.title
     AND m.year = CASE WHEN ?1 = 'movie' THEN COALESCE(f.year, 0) ELSE 0 END";

impl Db {
    /// All movies or shows, alphabetically.
    pub fn library(&self, kind: ItemKind) -> Result<Vec<LibraryItem>> {
        self.items(kind, "ORDER BY f.title COLLATE NOCASE, f.year", u32::MAX)
    }

    pub fn recently_added(&self, kind: ItemKind, limit: u32) -> Result<Vec<LibraryItem>> {
        self.items(kind, "ORDER BY MAX(f.added_at) DESC, MIN(f.id) DESC", limit)
    }

    /// Titles never looked up, or looked up by an older version of the lookup.
    pub fn missing_metadata(&self, kind: ItemKind, limit: u32) -> Result<Vec<LibraryItem>> {
        let all = self.items(kind, "ORDER BY MAX(f.added_at) DESC", u32::MAX)?;
        Ok(all
            .into_iter()
            .filter(|i| i.metadata.is_none() || i.metadata_version < METADATA_VERSION)
            .take(limit as usize)
            .collect())
    }

    fn items(&self, kind: ItemKind, order: &str, limit: u32) -> Result<Vec<LibraryItem>> {
        let (file_kind, group) = match kind {
            ItemKind::Movie => ("movie", "f.title COLLATE NOCASE, f.year"),
            ItemKind::Show => ("episode", "f.title COLLATE NOCASE"),
        };
        let sql = format!("{ITEM_SELECT} WHERE f.kind = ?2 GROUP BY {group} {order} LIMIT ?3");
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(params![kind.as_str(), file_kind, limit], |r| {
            let status: Option<String> = r.get(5)?;
            let metadata_version: Option<u32> = r.get(6)?;
            let metadata = match status.as_deref() {
                Some("matched") => Some(metadata_row(r, 7)?),
                Some(_) => Some(Metadata::default()),
                None => None,
            };
            Ok(LibraryItem {
                key: TitleKey {
                    kind,
                    title: r.get(0)?,
                    year: if kind == ItemKind::Movie {
                        r.get(1)?
                    } else {
                        None
                    },
                },
                file_id: r.get(2)?,
                file_count: r.get(3)?,
                added_at: r.get(4)?,
                metadata,
                metadata_version: metadata_version.unwrap_or(0),
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn save_metadata(&self, key: &TitleKey, result: &MetadataResult) -> Result<()> {
        let empty = Metadata::default();
        let (status, m) = match result {
            MetadataResult::Matched(m) => ("matched", m),
            MetadataResult::NotFound => ("not_found", &empty),
        };
        let join = |items: &[String]| (!items.is_empty()).then(|| items.join("\n"));
        // A show matched to a different TMDB entry than before has episode art from the wrong
        // series; drop it so it is fetched again for the new match. A show matched for the first
        // time keeps its grabbed frames but asks TMDB again for names, synopses and stills (they
        // were marked checked while TMDB had nothing to offer for an unmatched show).
        if key.kind == ItemKind::Show {
            let previous: Option<i64> = self
                .conn
                .query_row(
                    "SELECT tmdb_id FROM title_metadata WHERE kind = 'show' AND title = ?1 AND year = 0",
                    [&key.title],
                    |r| r.get(0),
                )
                .optional()?
                .flatten();
            if previous.is_some() && previous != m.tmdb_id {
                self.conn.execute(
                    "DELETE FROM episode_art WHERE file_id IN
                        (SELECT id FROM media_files WHERE kind = 'episode' AND title = ?1 COLLATE NOCASE)",
                    [&key.title],
                )?;
            } else if previous.is_none() && m.tmdb_id.is_some() {
                self.conn.execute(
                    "UPDATE episode_art SET tmdb_checked = 0 WHERE source != 'tmdb' AND file_id IN
                        (SELECT id FROM media_files WHERE kind = 'episode' AND title = ?1 COLLATE NOCASE)",
                    [&key.title],
                )?;
            }
        }
        self.conn.execute(
            "INSERT OR REPLACE INTO title_metadata
                (kind, title, year, status, tmdb_id, display_title, overview, poster_file,
                 backdrop_file, accent_rgb, fetched_at, details_version, backdrops, logo_file,
                 rating_tenths, certification, cast_names, genres, runtime_min, release_year)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17,
                     ?18, ?19, ?20)",
            params![
                key.kind.as_str(),
                key.title,
                key.year.unwrap_or(0),
                status,
                m.tmdb_id,
                m.display_title,
                m.overview,
                m.poster_file,
                m.backdrop_file,
                m.accent_rgb,
                now(),
                METADATA_VERSION,
                join(&m.backdrops),
                m.logo_file,
                m.rating_tenths,
                m.certification,
                join(&m.cast),
                join(&m.genres),
                m.runtime_min,
                m.release_year,
            ],
        )?;
        self.conn.execute(
            "DELETE FROM title_cast WHERE kind = ?1 AND title = ?2 AND year = ?3",
            params![key.kind.as_str(), key.title, key.year.unwrap_or(0)],
        )?;
        // Plain INSERT: REPLACE would delete rows without firing the search index's trigger.
        let mut insert = self.conn.prepare(
            "INSERT INTO title_cast
                (kind, title, year, position, person_id, name, character, profile_path, job)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        )?;
        let cast = m
            .credits
            .iter()
            .enumerate()
            .map(|(i, member)| (i, member, false));
        // Crew follow the cast, numbered from CREW_POSITION so the two never collide.
        let crew = m
            .crew
            .iter()
            .enumerate()
            .map(|(i, member)| (CREW_POSITION + i, member, true));
        for (position, member, is_crew) in cast.chain(crew) {
            let (character, job) = if is_crew {
                (None, member.character.as_deref())
            } else {
                (member.character.as_deref(), None)
            };
            insert.execute(params![
                key.kind.as_str(),
                key.title,
                key.year.unwrap_or(0),
                position as i64,
                member.person_id,
                member.name,
                character,
                member.profile_path,
                job,
            ])?;
        }
        Ok(())
    }

    /// Rewrites stored artwork paths that start with `old` to start with `new` instead, after
    /// the app's data folder moved (titles' posters, backdrops and logos, and episode stills).
    pub fn replace_path_prefix(&self, old: &str, new: &str) -> Result<()> {
        // `LIKE` treats `_` and `%` as wildcards and paths may contain them, so match the
        // prefix with `substr` instead.
        let swap = |column: &str| {
            format!(
                "{column} = CASE WHEN substr({column}, 1, length(?1)) = ?1
                     THEN ?2 || substr({column}, length(?1) + 1) ELSE {column} END"
            )
        };
        self.conn.execute(
            &format!(
                "UPDATE title_metadata SET {}, {}, {}",
                swap("poster_file"),
                swap("backdrop_file"),
                swap("logo_file")
            ),
            params![old, new],
        )?;
        // Extra backdrops are a newline-separated list: replace the prefix on every line.
        self.conn.execute(
            "UPDATE title_metadata SET backdrops = replace(backdrops, ?1, ?2)
             WHERE backdrops IS NOT NULL",
            params![old, new],
        )?;
        self.conn.execute(
            &format!("UPDATE episode_art SET {}", swap("still_file")),
            params![old, new],
        )?;
        Ok(())
    }

    /// A title's full cast, in billing order.
    pub fn cast(&self, key: &TitleKey) -> Result<Vec<CastMember>> {
        self.title_people(key, "job IS NULL", "character")
    }

    /// A title's directors or creators, with the job in `character`.
    pub fn crew(&self, key: &TitleKey) -> Result<Vec<CastMember>> {
        self.title_people(key, "job IS NOT NULL", "job")
    }

    fn title_people(&self, key: &TitleKey, filter: &str, role: &str) -> Result<Vec<CastMember>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT person_id, name, {role}, profile_path FROM title_cast
             WHERE kind = ?1 AND title = ?2 AND year = ?3 AND {filter} ORDER BY position"
        ))?;
        let rows = stmt.query_map(
            params![key.kind.as_str(), key.title, key.year.unwrap_or(0)],
            |r| {
                Ok(CastMember {
                    person_id: r.get(0)?,
                    name: r.get(1)?,
                    character: r.get(2)?,
                    profile_path: r.get(3)?,
                })
            },
        )?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Library titles a person appears in or made, with their part in each: the character
    /// they play, their job, or both ("Director · Joe").
    pub fn titles_with_person(&self, person_id: i64) -> Result<Vec<(TitleKey, Option<String>)>> {
        let mut stmt = self.conn.prepare(
            "SELECT kind, title, year, character, job FROM title_cast WHERE person_id = ?1
             ORDER BY year DESC, title, job IS NULL",
        )?;
        let rows = stmt.query_map([person_id], |r| {
            let kind = match r.get::<_, String>(0)?.as_str() {
                "show" => ItemKind::Show,
                _ => ItemKind::Movie,
            };
            let year: u16 = r.get(2)?;
            let part: Option<String> = r.get::<_, Option<String>>(4)?.or(r.get(3)?);
            Ok((
                TitleKey {
                    kind,
                    title: r.get(1)?,
                    year: (year != 0).then_some(year),
                },
                part,
            ))
        })?;
        let mut titles: Vec<(TitleKey, Option<String>)> = Vec::new();
        for row in rows {
            let (key, part) = row?;
            // Someone who both directs and acts in a title has two rows: show it once.
            match titles.iter_mut().find(|(k, _)| *k == key) {
                Some((_, existing)) => {
                    *existing = match (existing.take(), part) {
                        (Some(a), Some(b)) => Some(format!("{a} · {b}")),
                        (a, b) => a.or(b),
                    }
                }
                None => titles.push((key, part)),
            }
        }
        Ok(titles)
    }

    /// People in the cast or crew of stored titles whose name matches `query` (word
    /// prefixes, accent-insensitive), those with the most titles first.
    pub fn search_people(&self, query: &str, limit: u32) -> Result<Vec<PersonMatch>> {
        let Some(terms) = crate::db::fts_query(query) else {
            return Ok(Vec::new());
        };
        let mut stmt = self.conn.prepare(
            "SELECT c.person_id, MAX(c.name), MAX(c.profile_path),
                    MAX(c.job IS NULL), group_concat(DISTINCT c.job)
             FROM people_search s JOIN title_cast c ON c.rowid = s.rowid
             WHERE people_search MATCH ?1 AND c.person_id != 0
             GROUP BY c.person_id
             ORDER BY COUNT(DISTINCT c.kind || '/' || c.title || '/' || c.year) DESC, MAX(c.name)
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![terms, limit], |r| {
            let jobs: Option<String> = r.get(4)?;
            Ok(PersonMatch {
                person_id: r.get(0)?,
                name: r.get(1)?,
                profile_path: r.get(2)?,
                acts: r.get(3)?,
                jobs: jobs
                    .map(|j| j.split(',').map(str::to_string).collect())
                    .unwrap_or_default(),
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Stored metadata for one title, if it has been looked up and matched.
    pub fn metadata(&self, key: &TitleKey) -> Result<Option<Metadata>> {
        Ok(self
            .conn
            .query_row(
                &format!(
                    "SELECT {METADATA_COLUMNS} FROM title_metadata m
                     WHERE kind = ?1 AND title = ?2 AND year = ?3 AND status = 'matched'"
                ),
                params![key.kind.as_str(), key.title, key.year.unwrap_or(0)],
                |r| metadata_row(r, 0),
            )
            .optional()?)
    }

    /// The TMDB entry the user picked for a title with Fix match, if any.
    pub fn match_override(&self, key: &TitleKey) -> Result<Option<(ItemKind, i64)>> {
        Ok(self
            .conn
            .query_row(
                "SELECT tmdb_kind, tmdb_id FROM match_overrides
                 WHERE kind = ?1 AND title = ?2 AND year = ?3",
                params![key.kind.as_str(), key.title, key.year.unwrap_or(0)],
                |r| {
                    let kind = match r.get::<_, String>(0)?.as_str() {
                        "show" => ItemKind::Show,
                        _ => ItemKind::Movie,
                    };
                    Ok((kind, r.get(1)?))
                },
            )
            .optional()?)
    }

    pub fn set_match_override(&self, key: &TitleKey, kind: ItemKind, tmdb_id: i64) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO match_overrides (kind, title, year, tmdb_kind, tmdb_id)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                key.kind.as_str(),
                key.title,
                key.year.unwrap_or(0),
                kind.as_str(),
                tmdb_id
            ],
        )?;
        Ok(())
    }

    /// Path of a media file, for playback.
    pub fn file_path(&self, file_id: i64) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row(
                "SELECT path FROM media_files WHERE id = ?1",
                [file_id],
                |r| r.get(0),
            )
            .optional()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Fingerprint;
    use crate::parse::ParsedMedia;

    fn seeded() -> Db {
        let mut db = Db::open_in_memory().unwrap();
        let root = db.add_root("/m").unwrap();
        let b = db.batch().unwrap();
        let fp = Fingerprint { size: 1, mtime: 1 };
        let movie = |t: &str, y| ParsedMedia::Movie {
            title: t.into(),
            year: Some(y),
        };
        let ep = |s, e| ParsedMedia::Episode {
            show: "Dark".into(),
            season: s,
            episode: e,
            episode_end: None,
            title: None,
        };
        b.upsert_file(root.id, "/m/heat.mkv", fp, &movie("Heat", 1995))
            .unwrap();
        b.upsert_file(root.id, "/m/heat-4k.mkv", fp, &movie("heat", 1995))
            .unwrap();
        b.upsert_file(root.id, "/m/arrival.mkv", fp, &movie("Arrival", 2016))
            .unwrap();
        b.upsert_file(root.id, "/m/d1.mkv", fp, &ep(1, 1)).unwrap();
        b.upsert_file(root.id, "/m/d2.mkv", fp, &ep(1, 2)).unwrap();
        b.commit().unwrap();
        db
    }

    #[test]
    fn groups_movies_and_shows() {
        let db = seeded();
        let movies = db.library(ItemKind::Movie).unwrap();
        let summary: Vec<_> = movies
            .iter()
            .map(|m| (m.key.title.as_str(), m.file_count))
            .collect();
        assert_eq!(summary, [("Arrival", 1), ("Heat", 2)]);
        let shows = db.library(ItemKind::Show).unwrap();
        assert_eq!(shows.len(), 1);
        assert_eq!(shows[0].file_count, 2);
        assert_eq!(shows[0].key.year, None);
    }

    #[test]
    fn metadata_is_joined_case_insensitively() {
        let db = seeded();
        assert_eq!(db.missing_metadata(ItemKind::Movie, 10).unwrap().len(), 2);

        let key = TitleKey {
            kind: ItemKind::Movie,
            title: "HEAT".into(),
            year: Some(1995),
        };
        let meta = Metadata {
            tmdb_id: Some(949),
            display_title: Some("Heat".into()),
            poster_file: Some("/cache/949.jpg".into()),
            accent_rgb: Some(0x203040),
            ..Metadata::default()
        };
        db.save_metadata(&key, &MetadataResult::Matched(meta.clone()))
            .unwrap();
        let show = TitleKey {
            kind: ItemKind::Show,
            title: "Dark".into(),
            year: None,
        };
        db.save_metadata(&show, &MetadataResult::NotFound).unwrap();

        let heat = db.library(ItemKind::Movie).unwrap().pop().unwrap();
        assert_eq!(heat.metadata, Some(meta.clone()));
        assert_eq!(db.metadata(&key).unwrap(), Some(meta));
        assert_eq!(db.metadata(&show).unwrap(), None, "misses are not returned");
        let missing = db.missing_metadata(ItemKind::Movie, 10).unwrap();
        assert_eq!(missing.len(), 1);
        assert_eq!(missing[0].key.title, "Arrival");
        assert_eq!(db.missing_metadata(ItemKind::Show, 10).unwrap().len(), 0);
        assert_eq!(
            db.library(ItemKind::Show).unwrap()[0].metadata,
            Some(Metadata::default())
        );
    }
}

#[cfg(test)]
mod rematch_tests {
    use super::*;
    use crate::db::Fingerprint;
    use crate::episodes::{ArtSource, EpisodeArt};
    use crate::parse::ParsedMedia;

    #[test]
    fn changing_a_shows_match_clears_its_episode_art() {
        let mut db = Db::open_in_memory().unwrap();
        let root = db.add_root("/m").unwrap();
        let b = db.batch().unwrap();
        let ep = ParsedMedia::Episode {
            show: "Shogun".into(),
            season: 1,
            episode: 1,
            episode_end: None,
            title: None,
        };
        b.upsert_file(
            root.id,
            "/m/s1e1.mkv",
            Fingerprint { size: 1, mtime: 1 },
            &ep,
        )
        .unwrap();
        b.commit().unwrap();
        let file = db.file_by_path("/m/s1e1.mkv").unwrap().unwrap().id;
        let key = TitleKey {
            kind: ItemKind::Show,
            title: "Shogun".into(),
            year: None,
        };
        let matched = |id| {
            MetadataResult::Matched(Metadata {
                tmdb_id: Some(id),
                ..Metadata::default()
            })
        };
        let art = EpisodeArt {
            source: ArtSource::Tmdb,
            still_file: None,
            name: Some("Wrong show".into()),
            overview: None,
            runtime_min: None,
            tmdb_checked: true,
        };

        db.save_metadata(&key, &matched(1)).unwrap();
        db.save_episode_art(file, &art).unwrap();
        db.save_metadata(&key, &matched(1)).unwrap();
        assert_eq!(
            db.episode_art("Shogun").unwrap().len(),
            1,
            "same match keeps art"
        );

        db.save_metadata(&key, &matched(2)).unwrap();
        assert!(
            db.episode_art("Shogun").unwrap().is_empty(),
            "new match drops art"
        );
    }

    #[test]
    fn match_overrides_round_trip_per_title() {
        let db = Db::open_in_memory().unwrap();
        let key = TitleKey {
            kind: ItemKind::Movie,
            title: "Our Blues".into(),
            year: None,
        };
        assert_eq!(db.match_override(&key).unwrap(), None);
        db.set_match_override(&key, ItemKind::Show, 136_283)
            .unwrap();
        assert_eq!(
            db.match_override(&key).unwrap(),
            Some((ItemKind::Show, 136_283))
        );
        let other = TitleKey {
            year: Some(2022),
            ..key.clone()
        };
        assert_eq!(db.match_override(&other).unwrap(), None);
    }

    #[test]
    fn a_first_match_rechecks_frame_only_episodes() {
        let mut db = Db::open_in_memory().unwrap();
        let root = db.add_root("/m").unwrap();
        let b = db.batch().unwrap();
        let ep = ParsedMedia::Episode {
            show: "Dead End Job".into(),
            season: 1,
            episode: 1,
            episode_end: None,
            title: None,
        };
        b.upsert_file(root.id, "/m/e1.mkv", Fingerprint { size: 1, mtime: 1 }, &ep)
            .unwrap();
        b.commit().unwrap();
        let file = db.file_by_path("/m/e1.mkv").unwrap().unwrap().id;
        let key = TitleKey {
            kind: ItemKind::Show,
            title: "Dead End Job".into(),
            year: None,
        };
        // Not found on TMDB at first: a frame is grabbed and marked as checked.
        db.save_metadata(&key, &MetadataResult::NotFound).unwrap();
        let frame = EpisodeArt {
            source: ArtSource::Frame,
            still_file: Some("/cache/frames/1.jpg".into()),
            name: None,
            overview: None,
            runtime_min: None,
            tmdb_checked: true,
        };
        db.save_episode_art(file, &frame).unwrap();

        // Matched later: the frame stays, but TMDB is asked again.
        let matched = MetadataResult::Matched(Metadata {
            tmdb_id: Some(289_517),
            ..Metadata::default()
        });
        db.save_metadata(&key, &matched).unwrap();
        let art = db
            .episode_art("Dead End Job")
            .unwrap()
            .remove(&file)
            .unwrap();
        assert_eq!(art.still_file.as_deref(), Some("/cache/frames/1.jpg"));
        assert!(!art.tmdb_checked);
    }
}

#[cfg(test)]
mod cast_tests {
    use super::*;

    fn member(id: i64, name: &str, character: &str) -> CastMember {
        CastMember {
            person_id: id,
            name: name.into(),
            character: Some(character.into()),
            profile_path: Some(format!("/{id}.jpg")),
        }
    }

    #[test]
    fn cast_is_saved_in_order_and_found_by_person() {
        let db = Db::open_in_memory().unwrap();
        let heat = TitleKey {
            kind: ItemKind::Movie,
            title: "Heat".into(),
            year: Some(1995),
        };
        let show = TitleKey {
            kind: ItemKind::Show,
            title: "True Detective".into(),
            year: None,
        };
        let save = |key: &TitleKey, credits: Vec<CastMember>| {
            let meta = Metadata {
                credits,
                ..Default::default()
            };
            db.save_metadata(key, &MetadataResult::Matched(meta))
                .unwrap();
        };
        save(
            &heat,
            vec![
                member(1, "Al Pacino", "Hanna"),
                member(2, "Val Kilmer", "Chris"),
            ],
        );
        save(
            &show,
            vec![
                member(3, "Matthew McConaughey", "Rust"),
                member(2, "Val Kilmer", "Guest"),
            ],
        );

        let cast = db.cast(&heat).unwrap();
        assert_eq!(
            cast.iter().map(|m| m.name.as_str()).collect::<Vec<_>>(),
            ["Al Pacino", "Val Kilmer"]
        );
        let titles = db.titles_with_person(2).unwrap();
        assert_eq!(titles.len(), 2);
        assert!(
            titles
                .iter()
                .any(|(k, c)| k == &show && c.as_deref() == Some("Guest"))
        );
        assert!(titles.iter().any(|(k, _)| k == &heat));

        // A new lookup replaces the old cast.
        save(&heat, vec![member(1, "Al Pacino", "Hanna")]);
        assert_eq!(db.cast(&heat).unwrap().len(), 1);
        assert_eq!(db.titles_with_person(2).unwrap().len(), 1);
    }

    #[test]
    fn crew_is_kept_apart_from_cast_and_people_are_searchable() {
        let db = Db::open_in_memory().unwrap();
        let heat = TitleKey {
            kind: ItemKind::Movie,
            title: "Heat".into(),
            year: Some(1995),
        };
        let meta = Metadata {
            credits: vec![
                member(1, "Al Pacino", "Hanna"),
                member(9, "Michael Mann", "Cameo"),
            ],
            crew: vec![member(9, "Michael Mann", "Director")],
            ..Default::default()
        };
        db.save_metadata(&heat, &MetadataResult::Matched(meta.clone()))
            .unwrap();
        assert_eq!(db.cast(&heat).unwrap().len(), 2);
        let crew = db.crew(&heat).unwrap();
        assert_eq!(crew.len(), 1);
        assert_eq!(crew[0].character.as_deref(), Some("Director"));

        // Directing and acting in one title is listed once, with both parts.
        let titles = db.titles_with_person(9).unwrap();
        assert_eq!(titles.len(), 1);
        assert_eq!(titles[0].1.as_deref(), Some("Director · Cameo"));

        let found = db.search_people("mann", 10).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].name, "Michael Mann");
        assert!(found[0].acts);
        assert_eq!(found[0].jobs, ["Director"]);
        assert_eq!(db.search_people("al pac", 10).unwrap()[0].person_id, 1);
        assert!(db.search_people("nobody", 10).unwrap().is_empty());

        // Saving again replaces the rows and keeps the search index in step.
        db.save_metadata(&heat, &MetadataResult::Matched(meta))
            .unwrap();
        assert_eq!(db.search_people("mann", 10).unwrap().len(), 1);
        db.save_metadata(&heat, &MetadataResult::Matched(Metadata::default()))
            .unwrap();
        assert!(db.search_people("mann", 10).unwrap().is_empty());
    }
}

#[cfg(test)]
mod path_tests {
    use super::*;

    #[test]
    fn artwork_paths_follow_a_moved_folder() {
        let db = Db::open_in_memory().unwrap();
        let key = TitleKey {
            kind: ItemKind::Movie,
            title: "Heat".into(),
            year: Some(1995),
        };
        let old = r"C:\Users\x\AppData\Local\Nocturne";
        let meta = Metadata {
            poster_file: Some(format!(r"{old}\cache\artwork\movie-949-poster.jpg")),
            backdrop_file: Some(format!(r"{old}\cache\artwork\movie-949-backdrop-0.jpg")),
            backdrops: vec![
                format!(r"{old}\cache\artwork\movie-949-backdrop-0.jpg"),
                format!(r"{old}\cache\artwork\movie-949-backdrop-1.jpg"),
            ],
            // Local artwork next to the media is left alone.
            logo_file: Some(r"D:\Movies\Heat\logo.png".into()),
            ..Default::default()
        };
        db.save_metadata(&key, &MetadataResult::Matched(meta))
            .unwrap();

        db.replace_path_prefix(old, r"C:\Users\x\AppData\Local\Miru")
            .unwrap();
        let moved = db.metadata(&key).unwrap().unwrap();
        assert_eq!(
            moved.poster_file.as_deref(),
            Some(r"C:\Users\x\AppData\Local\Miru\cache\artwork\movie-949-poster.jpg")
        );
        assert!(moved.backdrops.iter().all(|b| b.contains(r"\Miru\")));
        assert_eq!(moved.logo_file.as_deref(), Some(r"D:\Movies\Heat\logo.png"));
    }
}
