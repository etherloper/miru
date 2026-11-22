//! Library search: titles (movies and shows) and episode names, as separate result groups.

use rusqlite::params;

use crate::db::{Db, MEDIA_COLUMNS, MediaFile, Result, collect_media, fts_query, media_row};
use crate::library::{ItemKind, TitleKey};

impl Db {
    /// Movies and shows whose title matches `query` (word prefixes, accent-insensitive),
    /// best matches first.
    pub fn search_titles(&self, query: &str, limit: usize) -> Result<Vec<TitleKey>> {
        let Some(terms) = fts_query(query) else {
            return Ok(Vec::new());
        };
        let mut stmt = self.conn.prepare(
            "SELECT f.kind, f.title, f.year FROM media_search s JOIN media_files f ON f.id = s.rowid
             WHERE media_search MATCH ?1 ORDER BY s.rank LIMIT 500",
        )?;
        let rows = stmt.query_map(params![format!("{{title}} : ({terms})")], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Option<u16>>(2)?,
            ))
        })?;
        let mut keys: Vec<TitleKey> = Vec::new();
        for row in rows {
            let (kind, title, year) = row?;
            let key = match kind.as_str() {
                "movie" => TitleKey {
                    kind: ItemKind::Movie,
                    title,
                    year,
                },
                _ => TitleKey {
                    kind: ItemKind::Show,
                    title,
                    year: None,
                },
            };
            // Several files can share a title (episodes, duplicate copies): keep the first.
            let duplicate = keys.iter().any(|k| {
                k.kind == key.kind && k.year == key.year && k.title.eq_ignore_ascii_case(&key.title)
            });
            if !duplicate {
                keys.push(key);
                if keys.len() >= limit {
                    break;
                }
            }
        }
        Ok(keys)
    }

    /// Episodes whose name matches `query`, best matches first.
    pub fn search_episodes(&self, query: &str, limit: u32) -> Result<Vec<MediaFile>> {
        let Some(terms) = fts_query(query) else {
            return Ok(Vec::new());
        };
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {MEDIA_COLUMNS} FROM media_search s JOIN media_files ON media_files.id = s.rowid
             WHERE media_search MATCH ?1 AND media_files.kind = 'episode' ORDER BY s.rank LIMIT ?2"
        ))?;
        collect_media(stmt.query_map(
            params![format!("{{episode_title}} : ({terms})"), limit],
            media_row,
        )?)
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
        let fp = Fingerprint { size: 1, mtime: 1 };
        let b = db.batch().unwrap();
        let movie = |t: &str, y| ParsedMedia::Movie {
            title: t.into(),
            year: Some(y),
        };
        let ep = |show: &str, e, title: &str| ParsedMedia::Episode {
            show: show.into(),
            season: 1,
            episode: e,
            episode_end: None,
            title: Some(title.into()),
        };
        b.upsert_file(root.id, "/m/dk.mkv", fp, &movie("The Dark Knight", 2008))
            .unwrap();
        b.upsert_file(root.id, "/m/dk-4k.mkv", fp, &movie("The Dark Knight", 2008))
            .unwrap();
        b.upsert_file(root.id, "/m/am.mkv", fp, &movie("Amélie", 2001))
            .unwrap();
        b.upsert_file(root.id, "/m/d1.mkv", fp, &ep("Dark", 1, "Secrets"))
            .unwrap();
        b.upsert_file(root.id, "/m/d2.mkv", fp, &ep("Dark", 2, "Lies"))
            .unwrap();
        b.upsert_file(
            root.id,
            "/m/s1.mkv",
            fp,
            &ep("Severance", 1, "Good News About Hell"),
        )
        .unwrap();
        b.upsert_file(
            root.id,
            "/m/s2.mkv",
            fp,
            &ep("Severance", 2, "Dark Secrets"),
        )
        .unwrap();
        b.commit().unwrap();
        db
    }

    #[test]
    fn titles_match_titles_only_and_dedupe() {
        let db = seeded();
        let titles: Vec<_> = db
            .search_titles("dark", 10)
            .unwrap()
            .into_iter()
            .map(|k| (k.kind, k.title))
            .collect();
        // "Dark Secrets" is an episode name, so Severance is not a title match.
        assert_eq!(titles.len(), 2);
        assert!(titles.contains(&(ItemKind::Movie, "The Dark Knight".into())));
        assert!(titles.contains(&(ItemKind::Show, "Dark".into())));
        assert_eq!(db.search_titles("ameli", 10).unwrap()[0].title, "Amélie");
        assert!(db.search_titles("  ", 10).unwrap().is_empty());
    }

    #[test]
    fn episodes_match_episode_names() {
        let db = seeded();
        let mut paths: Vec<_> = db
            .search_episodes("secrets", 10)
            .unwrap()
            .into_iter()
            .map(|f| f.path)
            .collect();
        paths.sort();
        assert_eq!(paths, ["/m/d1.mkv", "/m/s2.mkv"]);
        assert!(
            db.search_episodes("knight", 10).unwrap().is_empty(),
            "movie titles are not episodes"
        );
        assert_eq!(
            db.search_episodes("good hell", 10).unwrap().len(),
            1,
            "terms can be in any order"
        );
    }
}
