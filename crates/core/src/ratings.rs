//! Outside ratings for a title (IMDb, Rotten Tomatoes, Metacritic), fetched from OMDb and
//! cached so detail pages show them straight away.

use rusqlite::{OptionalExtension, params};

use crate::db::{Db, Result};
use crate::library::TitleKey;

/// Ratings are fetched again after this long.
pub const RATINGS_MAX_AGE_SECS: i64 = 7 * 24 * 60 * 60;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TitleRatings {
    pub imdb_id: Option<String>,
    /// IMDb rating out of 10, in tenths (78 = 7.8).
    pub imdb_tenths: Option<u16>,
    pub imdb_votes: Option<u32>,
    /// Rotten Tomatoes Tomatometer, percent.
    pub rotten_tomatoes: Option<u8>,
    /// Metascore out of 100.
    pub metacritic: Option<u8>,
    /// Unix seconds.
    pub fetched_at: i64,
}

impl TitleRatings {
    pub fn is_empty(&self) -> bool {
        self.imdb_tenths.is_none() && self.rotten_tomatoes.is_none() && self.metacritic.is_none()
    }
}

impl Db {
    pub fn title_ratings(&self, key: &TitleKey) -> Result<Option<TitleRatings>> {
        Ok(self
            .conn
            .query_row(
                "SELECT imdb_id, imdb_tenths, imdb_votes, rotten_tomatoes, metacritic, fetched_at
                 FROM title_ratings WHERE kind = ?1 AND title = ?2 AND year = ?3",
                params![key.kind.as_str(), key.title, key.year.unwrap_or(0)],
                |r| {
                    Ok(TitleRatings {
                        imdb_id: r.get(0)?,
                        imdb_tenths: r.get(1)?,
                        imdb_votes: r.get(2)?,
                        rotten_tomatoes: r.get(3)?,
                        metacritic: r.get(4)?,
                        fetched_at: r.get(5)?,
                    })
                },
            )
            .optional()?)
    }

    pub fn save_title_ratings(&self, key: &TitleKey, ratings: &TitleRatings) -> Result<()> {
        self.conn.execute(
            "INSERT INTO title_ratings
                (kind, title, year, imdb_id, imdb_tenths, imdb_votes, rotten_tomatoes, metacritic,
                 fetched_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
             ON CONFLICT(kind, title, year) DO UPDATE SET
                imdb_id = excluded.imdb_id, imdb_tenths = excluded.imdb_tenths,
                imdb_votes = excluded.imdb_votes, rotten_tomatoes = excluded.rotten_tomatoes,
                metacritic = excluded.metacritic, fetched_at = excluded.fetched_at",
            params![
                key.kind.as_str(),
                key.title,
                key.year.unwrap_or(0),
                ratings.imdb_id,
                ratings.imdb_tenths,
                ratings.imdb_votes,
                ratings.rotten_tomatoes,
                ratings.metacritic,
                ratings.fetched_at
            ],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::ItemKind;

    #[test]
    fn ratings_round_trip() {
        let db = Db::open_in_memory().unwrap();
        let key = TitleKey {
            kind: ItemKind::Movie,
            title: "Heat".into(),
            year: Some(1995),
        };
        assert_eq!(db.title_ratings(&key).unwrap(), None);
        let ratings = TitleRatings {
            imdb_id: Some("tt0113277".into()),
            imdb_tenths: Some(83),
            imdb_votes: Some(750_000),
            rotten_tomatoes: Some(88),
            metacritic: Some(76),
            fetched_at: 10,
        };
        db.save_title_ratings(&key, &ratings).unwrap();
        assert_eq!(db.title_ratings(&key).unwrap(), Some(ratings));
    }
}
