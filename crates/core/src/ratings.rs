//! Outside ratings for a title (IMDb, Rotten Tomatoes, Metacritic, Letterboxd…), fetched from
//! MDBList and cached so detail pages show them straight away.

use rusqlite::{OptionalExtension, params};

use crate::db::{Db, Result};
use crate::library::TitleKey;

/// Ratings are fetched again after this long.
pub const RATINGS_MAX_AGE_SECS: i64 = 7 * 24 * 60 * 60;

/// One site's rating of a title.
#[derive(Debug, Clone, PartialEq)]
pub struct SourceRating {
    /// MDBList's name for the site, e.g. "imdb", "tomatoes", "letterboxd".
    pub source: String,
    /// The rating on the site's own scale (8.4 out of 10, 92 percent, 4.1 out of 5…).
    pub value: Option<f64>,
    /// The same rating out of 100.
    pub score: Option<u8>,
    pub votes: Option<u32>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct TitleRatings {
    pub ratings: Vec<SourceRating>,
    /// Unix seconds.
    pub fetched_at: i64,
}

impl TitleRatings {
    pub fn get(&self, source: &str) -> Option<&SourceRating> {
        self.ratings.iter().find(|r| r.source == source)
    }
}

impl Db {
    /// A title's cached ratings, or `None` if they were never fetched.
    pub fn title_ratings(&self, key: &TitleKey) -> Result<Option<TitleRatings>> {
        let k = (key.kind.as_str(), &key.title, key.year.unwrap_or(0));
        let fetched_at: Option<i64> = self
            .conn
            .query_row(
                "SELECT fetched_at FROM rating_fetches WHERE kind = ?1 AND title = ?2 AND year = ?3",
                params![k.0, k.1, k.2],
                |r| r.get(0),
            )
            .optional()?;
        let Some(fetched_at) = fetched_at else {
            return Ok(None);
        };
        let mut stmt = self.conn.prepare(
            "SELECT source, value, score, votes FROM title_scores
             WHERE kind = ?1 AND title = ?2 AND year = ?3",
        )?;
        let ratings = stmt
            .query_map(params![k.0, k.1, k.2], |r| {
                Ok(SourceRating {
                    source: r.get(0)?,
                    value: r.get(1)?,
                    score: r.get(2)?,
                    votes: r.get(3)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        Ok(Some(TitleRatings {
            ratings,
            fetched_at,
        }))
    }

    pub fn save_title_ratings(&self, key: &TitleKey, ratings: &TitleRatings) -> Result<()> {
        let k = (key.kind.as_str(), &key.title, key.year.unwrap_or(0));
        self.conn.execute(
            "DELETE FROM title_scores WHERE kind = ?1 AND title = ?2 AND year = ?3",
            params![k.0, k.1, k.2],
        )?;
        for r in &ratings.ratings {
            self.conn.execute(
                "INSERT OR REPLACE INTO title_scores (kind, title, year, source, value, score, votes)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![k.0, k.1, k.2, r.source, r.value, r.score, r.votes],
            )?;
        }
        self.conn.execute(
            "INSERT INTO rating_fetches (kind, title, year, fetched_at) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(kind, title, year) DO UPDATE SET fetched_at = excluded.fetched_at",
            params![k.0, k.1, k.2, ratings.fetched_at],
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
            ratings: vec![
                SourceRating {
                    source: "imdb".into(),
                    value: Some(8.3),
                    score: Some(83),
                    votes: Some(750_000),
                },
                SourceRating {
                    source: "tomatoes".into(),
                    value: Some(88.),
                    score: Some(88),
                    votes: None,
                },
            ],
            fetched_at: 10,
        };
        db.save_title_ratings(&key, &ratings).unwrap();
        let back = db.title_ratings(&key).unwrap().unwrap();
        assert_eq!(back.fetched_at, 10);
        assert_eq!(back.get("imdb").and_then(|r| r.value), Some(8.3));
        assert_eq!(back.get("tomatoes").and_then(|r| r.score), Some(88));

        // Saving again replaces the old set (a site that dropped its rating disappears).
        db.save_title_ratings(
            &key,
            &TitleRatings {
                ratings: vec![],
                fetched_at: 20,
            },
        )
        .unwrap();
        let back = db.title_ratings(&key).unwrap().unwrap();
        assert!(back.ratings.is_empty());
        assert_eq!(back.fetched_at, 20);
    }
}
