//! Backup and restore: settings, library folders, collections, watch history and track
//! choices, as one serialisable value (the app writes it as JSON).
//!
//! Files are matched by what they are (a movie's title and year, an episode's show, season and
//! number), not by database ids or paths, so a backup restores onto a fresh install or another
//! PC once the same videos have been scanned. The TMDB key is left out: it's a secret.

use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};

use crate::db::{Db, Result};
use crate::library::{ItemKind, TitleKey};

pub const BACKUP_FORMAT: u32 = 1;

/// Settings that don't belong in a backup: secrets and per-install bookkeeping.
const SKIPPED_SETTINGS: [&str; 4] = [
    "metadata.tmdb_key",
    "metadata.mdblist_key",
    "scan.parser_version",
    "updates.checked_at",
];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Backup {
    pub app: String,
    pub format: u32,
    /// Unix seconds.
    pub exported_at: i64,
    pub settings: Vec<(String, String)>,
    pub folders: Vec<String>,
    pub collections: Vec<BackupCollection>,
    pub history: Vec<BackupProgress>,
    pub show_tracks: Vec<BackupShowTracks>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BackupCollection {
    pub name: String,
    pub items: Vec<BackupTitle>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BackupTitle {
    /// "movie" or "show".
    pub kind: String,
    pub title: String,
    pub year: Option<u16>,
}

/// Progress on one file, identified by what it is.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BackupProgress {
    /// "movie" or "episode".
    pub kind: String,
    /// Movie title, or the show for an episode.
    pub title: String,
    pub year: Option<u16>,
    pub season: Option<u16>,
    pub episode: Option<u16>,
    pub position: f64,
    pub duration: f64,
    pub watched: bool,
    /// Unix milliseconds.
    pub updated_at: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BackupShowTracks {
    pub show: String,
    pub audio_lang: Option<String>,
    pub audio_track: Option<i64>,
    pub audio_title: Option<String>,
    pub subtitles_on: Option<bool>,
    pub subtitle_lang: Option<String>,
    pub subtitle_track: Option<i64>,
    pub subtitle_title: Option<String>,
}

/// What a restore did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RestoreReport {
    pub folders_added: usize,
    pub collections: usize,
    pub history_restored: usize,
    /// History for files that aren't in this library (yet).
    pub history_unmatched: usize,
}

impl Db {
    pub fn export_backup(&self, now_secs: i64) -> Result<Backup> {
        let mut stmt = self
            .conn
            .prepare("SELECT key, value FROM settings ORDER BY key")?;
        let settings = stmt
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?
            .into_iter()
            .filter(|(k, _)| !SKIPPED_SETTINGS.contains(&k.as_str()))
            .collect();

        let folders = self.roots()?.into_iter().map(|r| r.path).collect();

        let mut collections = Vec::new();
        for c in self.collections()? {
            let items = self
                .collection_keys(c.id)?
                .into_iter()
                .map(|k| BackupTitle {
                    kind: kind_name(k.kind).into(),
                    title: k.title,
                    year: k.year,
                })
                .collect();
            collections.push(BackupCollection {
                name: c.name,
                items,
            });
        }

        let mut stmt = self.conn.prepare(
            "SELECT f.kind, f.title, f.year, f.season, f.episode,
                    p.position, p.duration, p.watched, p.updated_at
             FROM playback_state p JOIN media_files f ON f.id = p.file_id
             ORDER BY p.updated_at",
        )?;
        let history = stmt
            .query_map([], |r| {
                Ok(BackupProgress {
                    kind: r.get(0)?,
                    title: r.get(1)?,
                    year: r.get(2)?,
                    season: r.get(3)?,
                    episode: r.get(4)?,
                    position: r.get(5)?,
                    duration: r.get(6)?,
                    watched: r.get(7)?,
                    updated_at: r.get(8)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;

        let mut stmt = self.conn.prepare(
            "SELECT show, audio_lang, audio_track, audio_title, subtitles_on, subtitle_lang,
                    subtitle_track, subtitle_title
             FROM show_track_prefs ORDER BY show",
        )?;
        let show_tracks = stmt
            .query_map([], |r| {
                Ok(BackupShowTracks {
                    show: r.get(0)?,
                    audio_lang: r.get(1)?,
                    audio_track: r.get(2)?,
                    audio_title: r.get(3)?,
                    subtitles_on: r.get(4)?,
                    subtitle_lang: r.get(5)?,
                    subtitle_track: r.get(6)?,
                    subtitle_title: r.get(7)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;

        Ok(Backup {
            app: "miru".into(),
            format: BACKUP_FORMAT,
            exported_at: now_secs,
            settings,
            folders,
            collections,
            history,
            show_tracks,
        })
    }

    /// Merges a backup into this library. Settings are replaced; folders that exist on this PC
    /// are added; collections merge by name; history is restored for files that are already in
    /// the library, keeping whichever progress is more recent.
    pub fn restore_backup(&self, backup: &Backup) -> Result<RestoreReport> {
        let mut report = RestoreReport::default();
        for (key, value) in &backup.settings {
            if SKIPPED_SETTINGS.contains(&key.as_str()) {
                continue;
            }
            self.conn.execute(
                "INSERT INTO settings (key, value) VALUES (?1, ?2)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![key, value],
            )?;
        }

        let existing: Vec<String> = self.roots()?.into_iter().map(|r| r.path).collect();
        for folder in &backup.folders {
            if !existing.contains(folder) && std::path::Path::new(folder).is_dir() {
                self.add_root(folder)?;
                report.folders_added += 1;
            }
        }

        for collection in &backup.collections {
            let id = match self
                .collections()?
                .into_iter()
                .find(|c| c.name.eq_ignore_ascii_case(&collection.name))
            {
                Some(c) => c.id,
                None => self.create_collection(&collection.name)?,
            };
            for item in &collection.items {
                let key = TitleKey {
                    kind: if item.kind == "show" {
                        ItemKind::Show
                    } else {
                        ItemKind::Movie
                    },
                    title: item.title.clone(),
                    year: item.year,
                };
                self.add_to_collection(id, &key)?;
            }
            report.collections += 1;
        }

        for entry in &backup.history {
            let file_id: Option<i64> = if entry.kind == "episode" {
                self.conn
                    .query_row(
                        "SELECT id FROM media_files
                         WHERE kind = 'episode' AND title = ?1 COLLATE NOCASE
                           AND season = ?2 AND episode = ?3
                         ORDER BY id LIMIT 1",
                        params![entry.title, entry.season, entry.episode],
                        |r| r.get(0),
                    )
                    .optional()?
            } else {
                self.conn
                    .query_row(
                        "SELECT id FROM media_files
                         WHERE kind = 'movie' AND title = ?1 COLLATE NOCASE AND year IS ?2
                         ORDER BY id LIMIT 1",
                        params![entry.title, entry.year],
                        |r| r.get(0),
                    )
                    .optional()?
            };
            let Some(file_id) = file_id else {
                report.history_unmatched += 1;
                continue;
            };
            // Keep whichever progress is newer.
            let changed = self.conn.execute(
                "INSERT INTO playback_state (file_id, position, duration, watched, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(file_id) DO UPDATE SET
                    position = excluded.position, duration = excluded.duration,
                    watched = excluded.watched, updated_at = excluded.updated_at
                 WHERE excluded.updated_at > playback_state.updated_at",
                params![
                    file_id,
                    entry.position,
                    entry.duration,
                    entry.watched,
                    entry.updated_at
                ],
            )?;
            report.history_restored += changed;
        }

        for t in &backup.show_tracks {
            self.conn.execute(
                "INSERT INTO show_track_prefs (show, audio_lang, audio_track, audio_title,
                     subtitles_on, subtitle_lang, subtitle_track, subtitle_title)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                 ON CONFLICT(show) DO UPDATE SET
                    audio_lang = excluded.audio_lang, audio_track = excluded.audio_track,
                    audio_title = excluded.audio_title, subtitles_on = excluded.subtitles_on,
                    subtitle_lang = excluded.subtitle_lang,
                    subtitle_track = excluded.subtitle_track,
                    subtitle_title = excluded.subtitle_title",
                params![
                    t.show,
                    t.audio_lang,
                    t.audio_track,
                    t.audio_title,
                    t.subtitles_on,
                    t.subtitle_lang,
                    t.subtitle_track,
                    t.subtitle_title
                ],
            )?;
        }
        Ok(report)
    }
}

fn kind_name(kind: ItemKind) -> &'static str {
    match kind {
        ItemKind::Movie => "movie",
        ItemKind::Show => "show",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Fingerprint;
    use crate::parse::ParsedMedia;

    fn library() -> (Db, i64, i64) {
        let mut db = Db::open_in_memory().unwrap();
        let root = db.add_root("/m").unwrap();
        let fp = Fingerprint { size: 1, mtime: 1 };
        let b = db.batch().unwrap();
        b.upsert_file(
            root.id,
            "/m/heat.mkv",
            fp,
            &ParsedMedia::Movie {
                title: "Heat".into(),
                year: Some(1995),
            },
        )
        .unwrap();
        b.upsert_file(
            root.id,
            "/m/dark-s01e02.mkv",
            fp,
            &ParsedMedia::Episode {
                show: "Dark".into(),
                season: 1,
                episode: 2,
                episode_end: None,
                title: None,
            },
        )
        .unwrap();
        b.commit().unwrap();
        let heat = db.file_by_path("/m/heat.mkv").unwrap().unwrap().id;
        let dark = db.file_by_path("/m/dark-s01e02.mkv").unwrap().unwrap().id;
        (db, heat, dark)
    }

    #[test]
    fn round_trips_onto_a_fresh_library() {
        let (db, heat, dark) = library();
        db.save_progress(heat, 600., 6000.).unwrap();
        db.set_watched(&[dark], true).unwrap();
        db.conn
            .execute(
                "INSERT INTO settings (key, value) VALUES ('metadata.tmdb_key', 'secret'),
                 ('playback.seek_step', '30')",
                [],
            )
            .unwrap();
        let faves = db.create_collection("Favourites").unwrap();
        let heat_key = TitleKey {
            kind: ItemKind::Movie,
            title: "Heat".into(),
            year: Some(1995),
        };
        db.add_to_collection(faves, &heat_key).unwrap();
        db.set_show_subtitles("Dark", Some("en"), Some("SDH"), Some(3))
            .unwrap();

        let backup = db.export_backup(1).unwrap();
        assert!(
            backup
                .settings
                .iter()
                .all(|(k, _)| k != "metadata.tmdb_key"),
            "the TMDB key is never exported"
        );
        assert_eq!(backup.history.len(), 2);

        // Another install with the same files (different ids) and no history.
        let (fresh, fresh_heat, fresh_dark) = library();
        let report = fresh.restore_backup(&backup).unwrap();
        assert_eq!(report.history_restored, 2);
        assert_eq!(report.history_unmatched, 0);
        assert_eq!(report.collections, 1);
        assert_eq!(
            fresh.progress(fresh_heat).unwrap().map(|p| p.position),
            Some(600.)
        );
        assert!(
            fresh
                .progress(fresh_dark)
                .unwrap()
                .is_some_and(|p| p.watched)
        );
        assert_eq!(fresh.settings().unwrap().seek_step, 30);
        assert_eq!(fresh.collections().unwrap()[0].item_count, 1);
        assert_eq!(
            fresh
                .show_track_prefs("dark")
                .unwrap()
                .subtitle_title
                .as_deref(),
            Some("SDH")
        );

        // Restoring again changes nothing: the progress isn't newer.
        assert_eq!(fresh.restore_backup(&backup).unwrap().history_restored, 0);
        assert_eq!(fresh.collections().unwrap().len(), 1, "merged by name");
    }
}
