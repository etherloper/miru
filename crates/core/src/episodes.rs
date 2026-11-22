//! Per-episode artwork and details (stills, names, synopses).

use std::collections::HashMap;

use rusqlite::params;

use crate::db::{Db, MEDIA_COLUMNS, MediaFile, Result, collect_media, media_row, now};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArtSource {
    Tmdb,
    /// A frame grabbed from the file.
    Frame,
    /// Nothing could be produced; recorded so it isn't retried on every visit.
    None,
}

impl ArtSource {
    fn as_str(self) -> &'static str {
        match self {
            Self::Tmdb => "tmdb",
            Self::Frame => "frame",
            Self::None => "none",
        }
    }

    fn parse(s: &str) -> Self {
        match s {
            "tmdb" => Self::Tmdb,
            "frame" => Self::Frame,
            _ => Self::None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EpisodeArt {
    pub source: ArtSource,
    pub still_file: Option<String>,
    pub name: Option<String>,
    pub overview: Option<String>,
    pub runtime_min: Option<u32>,
    pub tmdb_checked: bool,
}

impl Db {
    /// Artwork for every episode of a show, keyed by file id.
    pub fn episode_art(&self, show: &str) -> Result<HashMap<i64, EpisodeArt>> {
        let mut stmt = self.conn.prepare(
            "SELECT a.file_id, a.source, a.still_file, a.name, a.overview, a.runtime_min, a.tmdb_checked
             FROM episode_art a JOIN media_files f ON f.id = a.file_id
             WHERE f.kind = 'episode' AND f.title = ?1 COLLATE NOCASE",
        )?;
        let rows = stmt.query_map([show], |r| {
            Ok((
                r.get(0)?,
                EpisodeArt {
                    source: ArtSource::parse(&r.get::<_, String>(1)?),
                    still_file: r.get(2)?,
                    name: r.get(3)?,
                    overview: r.get(4)?,
                    runtime_min: r.get(5)?,
                    tmdb_checked: r.get(6)?,
                },
            ))
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Episodes that still need artwork. With `want_tmdb`, episodes that only have a grabbed
    /// frame and were never checked against TMDB are included too.
    pub fn episodes_needing_art(
        &self,
        show: &str,
        want_tmdb: bool,
        limit: u32,
    ) -> Result<Vec<MediaFile>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {MEDIA_COLUMNS} FROM media_files
             LEFT JOIN episode_art a ON a.file_id = media_files.id
             WHERE media_files.kind = 'episode' AND media_files.title = ?1 COLLATE NOCASE
               AND (a.file_id IS NULL OR (?2 AND a.tmdb_checked = 0))
             ORDER BY media_files.season, media_files.episode LIMIT ?3"
        ))?;
        collect_media(stmt.query_map(params![show, want_tmdb, limit], media_row)?)
    }

    pub fn save_episode_art(&self, file_id: i64, art: &EpisodeArt) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO episode_art
                (file_id, source, still_file, name, overview, runtime_min, tmdb_checked, fetched_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                file_id,
                art.source.as_str(),
                art.still_file,
                art.name,
                art.overview,
                art.runtime_min,
                art.tmdb_checked,
                now()
            ],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Fingerprint;
    use crate::parse::ParsedMedia;

    fn show() -> (Db, Vec<i64>) {
        let mut db = Db::open_in_memory().unwrap();
        let root = db.add_root("/m").unwrap();
        let b = db.batch().unwrap();
        for e in 1..=3 {
            let media = ParsedMedia::Episode {
                show: "Dark".into(),
                season: 1,
                episode: e,
                episode_end: None,
                title: None,
            };
            b.upsert_file(
                root.id,
                &format!("/m/d{e}.mkv"),
                Fingerprint { size: 1, mtime: 1 },
                &media,
            )
            .unwrap();
        }
        b.commit().unwrap();
        let ids = (1..=3)
            .map(|e| {
                db.file_by_path(&format!("/m/d{e}.mkv"))
                    .unwrap()
                    .unwrap()
                    .id
            })
            .collect();
        (db, ids)
    }

    fn art(source: ArtSource, tmdb_checked: bool) -> EpisodeArt {
        EpisodeArt {
            source,
            still_file: Some("/cache/x.jpg".into()),
            name: Some("Secrets".into()),
            overview: None,
            runtime_min: Some(51),
            tmdb_checked,
        }
    }

    #[test]
    fn tracks_which_episodes_need_art() {
        let (db, ids) = show();
        assert_eq!(db.episodes_needing_art("dark", false, 10).unwrap().len(), 3);

        db.save_episode_art(ids[0], &art(ArtSource::Tmdb, true))
            .unwrap();
        db.save_episode_art(ids[1], &art(ArtSource::Frame, false))
            .unwrap();
        db.save_episode_art(ids[2], &art(ArtSource::None, true))
            .unwrap();

        // Without TMDB everything is done; with TMDB the frame-only episode is revisited.
        assert!(
            db.episodes_needing_art("dark", false, 10)
                .unwrap()
                .is_empty()
        );
        let pending: Vec<_> = db
            .episodes_needing_art("dark", true, 10)
            .unwrap()
            .into_iter()
            .map(|f| f.id)
            .collect();
        assert_eq!(pending, [ids[1]]);

        let map = db.episode_art("Dark").unwrap();
        assert_eq!(map.len(), 3);
        assert_eq!(map[&ids[0]], art(ArtSource::Tmdb, true));
        assert_eq!(map[&ids[2]].source, ArtSource::None);
    }

    #[test]
    fn art_is_removed_with_the_file() {
        let (mut db, ids) = show();
        db.save_episode_art(ids[0], &art(ArtSource::Frame, false))
            .unwrap();
        let b = db.batch().unwrap();
        b.delete_files(&[ids[0]]).unwrap();
        b.commit().unwrap();
        assert!(db.episode_art("dark").unwrap().is_empty());
    }
}
