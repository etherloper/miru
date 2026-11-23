//! Resume points, watched state and the "Continue watching" list.

use std::collections::{HashMap, HashSet};

use rusqlite::{OptionalExtension, params};

use crate::db::{Db, MEDIA_COLUMNS, MediaFile, Result, collect_media, media_row};
use crate::parse::ParsedMedia;

/// Positions shorter than this aren't worth resuming (skipped intros, accidental opens).
pub const MIN_RESUME_SECS: f64 = 30.0;
/// Past this fraction a title counts as watched (credits are usually left unwatched).
pub const WATCHED_FRACTION: f64 = 0.92;

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Progress {
    pub position: f64,
    pub duration: f64,
    pub watched: bool,
}

impl Progress {
    /// Where playback should start, or `None` to start from the beginning.
    pub fn resume_at(&self) -> Option<f64> {
        (!self.watched && self.position >= MIN_RESUME_SECS).then_some(self.position)
    }

    pub fn fraction(&self) -> f32 {
        if self.duration > 0.0 {
            (self.position / self.duration).clamp(0.0, 1.0) as f32
        } else {
            0.0
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ContinueReason {
    /// Partially watched.
    Resume,
    /// The previous episode was finished; this is the next one.
    NextEpisode,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ContinueItem {
    pub file: MediaFile,
    pub progress: Progress,
    pub reason: ContinueReason,
}

/// Millisecond timestamp for ordering recent activity.
fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

impl Db {
    /// Records a playback position. Reaching [`WATCHED_FRACTION`] marks the file watched and
    /// clears its resume point.
    pub fn save_progress(&self, file_id: i64, position: f64, duration: f64) -> Result<Progress> {
        let watched = duration > 0.0 && position >= duration * WATCHED_FRACTION;
        let progress = Progress {
            position: if watched { 0.0 } else { position.max(0.0) },
            duration,
            watched,
        };
        self.conn.execute(
            "INSERT INTO playback_state (file_id, position, duration, watched, updated_at)
             -- Strictly increasing, so saves in the same millisecond still order correctly.
             VALUES (?1, ?2, ?3, ?4, MAX(?5, (SELECT COALESCE(MAX(updated_at), 0) + 1 FROM playback_state)))
             ON CONFLICT(file_id) DO UPDATE SET
                position = excluded.position, duration = excluded.duration,
                -- Rewatching a finished title from the start must not clear 'watched'.
                watched = MAX(playback_state.watched, excluded.watched),
                updated_at = excluded.updated_at",
            params![file_id, progress.position, duration, watched, now_millis()],
        )?;
        Ok(self.progress(file_id)?.unwrap_or(progress))
    }

    pub fn progress(&self, file_id: i64) -> Result<Option<Progress>> {
        Ok(self
            .conn
            .query_row(
                "SELECT position, duration, watched FROM playback_state WHERE file_id = ?1",
                [file_id],
                |r| {
                    Ok(Progress {
                        position: r.get(0)?,
                        duration: r.get(1)?,
                        watched: r.get(2)?,
                    })
                },
            )
            .optional()?)
    }

    /// Everything with saved progress, most recent first, with when each was last played
    /// (unix milliseconds), for managing watch history.
    pub fn watch_history(&self, limit: u32) -> Result<Vec<(MediaFile, Progress, i64)>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {MEDIA_COLUMNS}, p.position, p.duration, p.watched, p.updated_at
             FROM playback_state p JOIN media_files ON media_files.id = p.file_id
             ORDER BY p.updated_at DESC LIMIT ?1"
        ))?;
        let rows: Vec<(crate::db::RawMedia, Progress, i64)> = stmt
            .query_map([limit], |r| {
                Ok((
                    media_row(r)?,
                    Progress {
                        position: r.get(9)?,
                        duration: r.get(10)?,
                        watched: r.get(11)?,
                    },
                    r.get(12)?,
                ))
            })?
            .collect::<rusqlite::Result<_>>()?;
        let mut out = Vec::with_capacity(rows.len());
        for (raw, progress, at) in rows {
            if let Some(file) = collect_media(std::iter::once(Ok(raw)))?.pop() {
                out.push((file, progress, at));
            }
        }
        Ok(out)
    }

    /// Forgets progress and watched state for one file.
    pub fn remove_from_history(&self, file_id: i64) -> Result<()> {
        self.conn
            .execute("DELETE FROM playback_state WHERE file_id = ?1", [file_id])?;
        Ok(())
    }

    /// Marks files watched (clearing any resume point) or unwatched (forgetting their
    /// progress), e.g. from a detail page's menu.
    pub fn set_watched(&self, file_ids: &[i64], watched: bool) -> Result<()> {
        for &id in file_ids {
            if watched {
                self.conn.execute(
                    "INSERT INTO playback_state (file_id, position, duration, watched, updated_at)
                     VALUES (?1, 0, 0, 1, MAX(?2, (SELECT COALESCE(MAX(updated_at), 0) + 1 FROM playback_state)))
                     ON CONFLICT(file_id) DO UPDATE SET
                        position = 0, watched = 1, updated_at = excluded.updated_at",
                    params![id, now_millis()],
                )?;
            } else {
                self.remove_from_history(id)?;
            }
        }
        Ok(())
    }

    /// Movie files marked watched, and the shows (lowercased titles) whose every episode is
    /// watched; for watched badges and the Unwatched filter.
    pub fn watched_titles(&self) -> Result<(HashSet<i64>, HashSet<String>)> {
        let mut stmt = self.conn.prepare(
            "SELECT f.id FROM media_files f JOIN playback_state p ON p.file_id = f.id
             WHERE f.kind = 'movie' AND p.watched = 1",
        )?;
        let movies = stmt
            .query_map([], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        let mut stmt = self.conn.prepare(
            "SELECT f.title FROM media_files f LEFT JOIN playback_state p ON p.file_id = f.id
             WHERE f.kind = 'episode'
             GROUP BY f.title COLLATE NOCASE
             HAVING SUM(COALESCE(p.watched, 0)) = COUNT(*)",
        )?;
        let shows = stmt
            .query_map([], |r| r.get::<_, String>(0).map(|t| t.to_lowercase()))?
            .collect::<rusqlite::Result<_>>()?;
        Ok((movies, shows))
    }

    /// Forgets all progress and watched state.
    pub fn clear_history(&self) -> Result<()> {
        self.conn.execute("DELETE FROM playback_state", [])?;
        Ok(())
    }

    /// Progress for every episode of a show, keyed by file id.
    pub fn show_progress(&self, show: &str) -> Result<HashMap<i64, Progress>> {
        let mut stmt = self.conn.prepare(
            "SELECT p.file_id, p.position, p.duration, p.watched FROM playback_state p
             JOIN media_files f ON f.id = p.file_id
             WHERE f.kind = 'episode' AND f.title = ?1 COLLATE NOCASE",
        )?;
        let rows = stmt.query_map([show], |r| {
            Ok((
                r.get(0)?,
                Progress {
                    position: r.get(1)?,
                    duration: r.get(2)?,
                    watched: r.get(3)?,
                },
            ))
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// One entry per title, most recently played first: partially watched files, plus the
    /// next unwatched episode of shows whose last-played episode was finished.
    pub fn continue_watching(&self, limit: usize) -> Result<Vec<ContinueItem>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {MEDIA_COLUMNS}, p.position, p.duration, p.watched
             FROM playback_state p JOIN media_files ON media_files.id = p.file_id
             ORDER BY p.updated_at DESC LIMIT 200"
        ))?;
        let rows: Vec<(crate::db::RawMedia, Progress)> = stmt
            .query_map([], |r| {
                Ok((
                    media_row(r)?,
                    Progress {
                        position: r.get(9)?,
                        duration: r.get(10)?,
                        watched: r.get(11)?,
                    },
                ))
            })?
            .collect::<rusqlite::Result<_>>()?;

        let mut seen_titles = HashSet::new();
        let mut items = Vec::new();
        for (raw, progress) in rows {
            let Some(file) = collect_media(std::iter::once(Ok(raw)))?.pop() else {
                continue;
            };
            let title_key = match &file.media {
                ParsedMedia::Movie { title, year } => {
                    format!("m|{}|{year:?}", title.to_lowercase())
                }
                ParsedMedia::Episode { show, .. } => format!("s|{}", show.to_lowercase()),
            };
            // Only the most recent activity per title decides what to show.
            if !seen_titles.insert(title_key) {
                continue;
            }
            if progress.resume_at().is_some() {
                items.push(ContinueItem {
                    file,
                    progress,
                    reason: ContinueReason::Resume,
                });
            } else if progress.watched
                && let ParsedMedia::Episode { show, .. } = &file.media
                && let Some(next) = self.next_unwatched_episode(show, &file)?
            {
                let progress = self.progress(next.id)?.unwrap_or_default();
                items.push(ContinueItem {
                    file: next,
                    progress,
                    reason: ContinueReason::NextEpisode,
                });
            } else if !progress.watched && matches!(file.media, ParsedMedia::Episode { .. }) {
                // An episode only just started (under the resume threshold): usually the next
                // episode autoplay or Next episode began before the player was closed. Offer it
                // from the start rather than dropping the show. (A barely opened movie is
                // still skipped: that is usually an accidental click.)
                items.push(ContinueItem {
                    file,
                    progress,
                    reason: ContinueReason::NextEpisode,
                });
            }
            if items.len() >= limit {
                break;
            }
        }
        Ok(items)
    }

    /// The episode straight after `file` (by season and episode), watched or not.
    pub fn next_episode(&self, file_id: i64) -> Result<Option<MediaFile>> {
        let Some(current) = self.file_by_id(file_id)? else {
            return Ok(None);
        };
        let ParsedMedia::Episode {
            show,
            season,
            episode,
            episode_end,
            ..
        } = &current.media
        else {
            return Ok(None);
        };
        let last = episode_end.unwrap_or(*episode);
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {MEDIA_COLUMNS} FROM media_files
             WHERE media_files.kind = 'episode' AND media_files.title = ?1 COLLATE NOCASE
               AND (media_files.season > ?2 OR (media_files.season = ?2 AND media_files.episode > ?3))
             ORDER BY media_files.season, media_files.episode LIMIT 1"
        ))?;
        Ok(collect_media(stmt.query_map(params![show, season, last], media_row)?)?.pop())
    }

    /// The episode a show's page should offer, following the one played most recently: that
    /// episode again if it is part-way through (or barely started), otherwise the next unwatched
    /// one after it. `None` when nothing has been played, or the last episode was finished and
    /// nothing unwatched follows it.
    pub fn show_up_next(&self, show: &str) -> Result<Option<i64>> {
        let last: Option<(i64, Progress)> = self
            .conn
            .query_row(
                "SELECT p.file_id, p.position, p.duration, p.watched FROM playback_state p
                 JOIN media_files f ON f.id = p.file_id
                 WHERE f.kind = 'episode' AND f.title = ?1 COLLATE NOCASE
                 ORDER BY p.updated_at DESC LIMIT 1",
                [show],
                |r| {
                    Ok((
                        r.get(0)?,
                        Progress {
                            position: r.get(1)?,
                            duration: r.get(2)?,
                            watched: r.get(3)?,
                        },
                    ))
                },
            )
            .optional()?;
        let Some((file_id, progress)) = last else {
            return Ok(None);
        };
        // Rewatching a watched episode part-way still resumes it.
        if progress.resume_at().is_some() || !progress.watched {
            return Ok(Some(file_id));
        }
        let Some(file) = self.file_by_id(file_id)? else {
            return Ok(None);
        };
        Ok(self.next_unwatched_episode(show, &file)?.map(|f| f.id))
    }

    fn file_by_id(&self, file_id: i64) -> Result<Option<MediaFile>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {MEDIA_COLUMNS} FROM media_files WHERE id = ?1"
        ))?;
        Ok(collect_media(stmt.query_map([file_id], media_row)?)?.pop())
    }

    /// The first unwatched episode after `after` in season/episode order.
    pub fn next_unwatched_episode(
        &self,
        show: &str,
        after: &MediaFile,
    ) -> Result<Option<MediaFile>> {
        let ParsedMedia::Episode {
            season,
            episode,
            episode_end,
            ..
        } = after.media
        else {
            return Ok(None);
        };
        let last = episode_end.unwrap_or(episode);
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {MEDIA_COLUMNS} FROM media_files
             LEFT JOIN playback_state p ON p.file_id = media_files.id
             WHERE media_files.kind = 'episode' AND media_files.title = ?1 COLLATE NOCASE
               AND (media_files.season > ?2 OR (media_files.season = ?2 AND media_files.episode > ?3))
               AND COALESCE(p.watched, 0) = 0
             ORDER BY media_files.season, media_files.episode LIMIT 1"
        ))?;
        Ok(collect_media(stmt.query_map(params![show, season, last], media_row)?)?.pop())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Fingerprint;

    pub(super) fn library() -> (Db, HashMap<&'static str, i64>) {
        let mut db = Db::open_in_memory().unwrap();
        let root = db.add_root("/m").unwrap();
        let fp = Fingerprint { size: 1, mtime: 1 };
        let ep = |e| ParsedMedia::Episode {
            show: "Dark".into(),
            season: 1,
            episode: e,
            episode_end: None,
            title: None,
        };
        let files = [
            (
                "heat",
                ParsedMedia::Movie {
                    title: "Heat".into(),
                    year: Some(1995),
                },
            ),
            (
                "arrival",
                ParsedMedia::Movie {
                    title: "Arrival".into(),
                    year: Some(2016),
                },
            ),
            ("d1", ep(1)),
            ("d2", ep(2)),
            ("d3", ep(3)),
        ];
        let b = db.batch().unwrap();
        for (name, media) in &files {
            b.upsert_file(root.id, &format!("/m/{name}.mkv"), fp, media)
                .unwrap();
        }
        b.commit().unwrap();
        let ids = files
            .iter()
            .map(|(name, _)| {
                (
                    *name,
                    db.file_by_path(&format!("/m/{name}.mkv"))
                        .unwrap()
                        .unwrap()
                        .id,
                )
            })
            .collect();
        (db, ids)
    }

    #[test]
    fn continue_watching_keeps_a_show_whose_next_episode_just_started() {
        let (db, ids) = library();
        // Episode 1 finished, then Next episode started episode 2 and the player was closed
        // after a few seconds (below the resume threshold).
        db.save_progress(ids["d1"], 3000., 3000.).unwrap();
        db.save_progress(ids["d2"], 5., 3000.).unwrap();
        let items = db.continue_watching(10).unwrap();
        assert_eq!(items.len(), 1, "the show stays in Continue Watching");
        assert_eq!(items[0].file.id, ids["d2"]);
        assert_eq!(items[0].reason, ContinueReason::NextEpisode);

        // A movie opened for a moment is still left out.
        db.save_progress(ids["heat"], 3., 6000.).unwrap();
        let items = db.continue_watching(10).unwrap();
        assert!(items.iter().all(|i| i.file.id != ids["heat"]));
    }

    #[test]
    fn show_up_next_follows_the_last_played_episode() {
        let (db, ids) = library();
        assert_eq!(db.show_up_next("Dark").unwrap(), None, "nothing played yet");

        // Skipped ahead to episode 2 and stopped part-way: offer episode 2, not episode 1.
        db.save_progress(ids["d2"], 1200.0, 3000.0).unwrap();
        assert_eq!(db.show_up_next("Dark").unwrap(), Some(ids["d2"]));

        // Finished it: offer the next unwatched episode after it.
        db.save_progress(ids["d2"], 2950.0, 3000.0).unwrap();
        assert_eq!(db.show_up_next("Dark").unwrap(), Some(ids["d3"]));

        // Finished the last episode: nothing follows.
        db.save_progress(ids["d3"], 2950.0, 3000.0).unwrap();
        assert_eq!(db.show_up_next("Dark").unwrap(), None);
    }
    #[test]
    fn resume_and_watched_rules() {
        let (db, ids) = library();
        let heat = ids["heat"];
        assert_eq!(db.progress(heat).unwrap(), None);

        let p = db.save_progress(heat, 10.0, 6000.0).unwrap();
        assert_eq!(p.resume_at(), None, "too early to resume");
        let p = db.save_progress(heat, 1200.0, 6000.0).unwrap();
        assert_eq!(p.resume_at(), Some(1200.0));
        assert!((p.fraction() - 0.2).abs() < 1e-6);

        let p = db.save_progress(heat, 5700.0, 6000.0).unwrap();
        assert!(p.watched);
        assert_eq!(p.resume_at(), None);

        // Rewatching from the start keeps it marked watched.
        let p = db.save_progress(heat, 600.0, 6000.0).unwrap();
        assert!(p.watched);
    }

    #[test]
    fn continue_watching_resumes_and_suggests_next_episode() {
        let (db, ids) = library();
        db.save_progress(ids["arrival"], 900.0, 6000.0).unwrap();
        db.save_progress(ids["d1"], 2900.0, 3000.0).unwrap(); // finished -> next is d2
        db.save_progress(ids["heat"], 5.0, 6000.0).unwrap(); // barely started -> hidden

        let items = db.continue_watching(10).unwrap();
        let summary: Vec<_> = items
            .iter()
            .map(|i| (i.file.id, i.reason.clone()))
            .collect();
        assert_eq!(
            summary,
            [
                (ids["d2"], ContinueReason::NextEpisode),
                (ids["arrival"], ContinueReason::Resume)
            ]
        );

        // Watching d2 partway replaces the suggestion with a resume entry for d2.
        db.save_progress(ids["d2"], 400.0, 3000.0).unwrap();
        let first = &db.continue_watching(10).unwrap()[0];
        assert_eq!(
            (first.file.id, &first.reason),
            (ids["d2"], &ContinueReason::Resume)
        );

        // Finishing the last episode leaves nothing to suggest for the show.
        db.save_progress(ids["d2"], 2950.0, 3000.0).unwrap();
        db.save_progress(ids["d3"], 2950.0, 3000.0).unwrap();
        let items = db.continue_watching(10).unwrap();
        assert!(
            items
                .iter()
                .all(|i| !matches!(i.file.media, ParsedMedia::Episode { .. }))
        );
    }

    #[test]
    fn next_episode_follows_season_order() {
        let (db, ids) = library();
        let next = |id| db.next_episode(id).unwrap().map(|f| f.id);
        assert_eq!(next(ids["d1"]), Some(ids["d2"]));
        assert_eq!(next(ids["d2"]), Some(ids["d3"]));
        assert_eq!(next(ids["d3"]), None);
        assert_eq!(next(ids["heat"]), None);
    }

    #[test]
    fn show_progress_is_keyed_by_file() {
        let (db, ids) = library();
        db.save_progress(ids["d3"], 100.0, 3000.0).unwrap();
        let map = db.show_progress("dark").unwrap();
        assert_eq!(map.len(), 1);
        assert_eq!(map[&ids["d3"]].position, 100.0);
    }
}

#[cfg(test)]
mod history_tests {
    use super::*;
    use crate::db::Fingerprint;

    #[test]
    fn history_lists_removes_and_clears() {
        let mut db = Db::open_in_memory().unwrap();
        let root = db.add_root("/m").unwrap();
        let fp = Fingerprint { size: 1, mtime: 1 };
        let b = db.batch().unwrap();
        for name in ["a", "b", "c"] {
            let media = ParsedMedia::Movie {
                title: name.into(),
                year: None,
            };
            b.upsert_file(root.id, &format!("/m/{name}.mkv"), fp, &media)
                .unwrap();
        }
        b.commit().unwrap();
        let id = |n: &str| db.file_by_path(&format!("/m/{n}.mkv")).unwrap().unwrap().id;
        let (a, b, c) = (id("a"), id("b"), id("c"));
        db.save_progress(a, 600., 6000.).unwrap();
        db.save_progress(b, 5900., 6000.).unwrap();
        db.save_progress(c, 100., 6000.).unwrap();

        let order: Vec<_> = db
            .watch_history(10)
            .unwrap()
            .into_iter()
            .map(|(f, _, _)| f.id)
            .collect();
        assert_eq!(order, [c, b, a], "most recent first");
        assert!(db.watch_history(10).unwrap()[1].1.watched);

        db.remove_from_history(b).unwrap();
        assert_eq!(db.progress(b).unwrap(), None);
        assert_eq!(db.watch_history(10).unwrap().len(), 2);

        db.clear_history().unwrap();
        assert!(db.watch_history(10).unwrap().is_empty());
        assert_eq!(
            db.next_episode(a).unwrap(),
            None,
            "movies have no next episode"
        );
        assert!(db.continue_watching(10).unwrap().is_empty());
    }

    #[test]
    fn marking_watched_and_unwatched() {
        let (db, ids) = super::tests::library();
        db.save_progress(ids["heat"], 600., 6000.).unwrap();
        db.set_watched(&[ids["heat"]], true).unwrap();
        let heat = db.progress(ids["heat"]).unwrap().unwrap();
        assert!(heat.watched);
        assert_eq!(heat.resume_at(), None);

        db.set_watched(&[ids["d1"], ids["d2"]], true).unwrap();
        let (movies, shows) = db.watched_titles().unwrap();
        assert!(movies.contains(&ids["heat"]) && !movies.contains(&ids["arrival"]));
        assert!(shows.is_empty(), "one episode of Dark is still unwatched");

        db.set_watched(&[ids["d3"]], true).unwrap();
        assert!(db.watched_titles().unwrap().1.contains("dark"));

        db.set_watched(&[ids["d3"], ids["heat"]], false).unwrap();
        assert_eq!(db.progress(ids["heat"]).unwrap(), None);
        let (movies, shows) = db.watched_titles().unwrap();
        assert!(movies.is_empty() && shows.is_empty());
    }
}
