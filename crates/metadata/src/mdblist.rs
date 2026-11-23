//! MDBList: one title's ratings from many sites (IMDb, Rotten Tomatoes critics and audience,
//! Metacritic, Letterboxd, Trakt, TMDB, Roger Ebert, MyAnimeList), looked up by TMDB id.
//! Needs the user's own MDBList key.

use std::time::Duration;

use miru_core::library::ItemKind;
use miru_core::ratings::SourceRating;
use serde_json::Value;

use crate::Result;

const API: &str = "https://api.mdblist.com";

#[derive(Clone)]
pub struct Mdblist {
    agent: ureq::Agent,
    key: String,
}

impl Mdblist {
    pub fn new(key: &str) -> Self {
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(15)))
            .user_agent("Miru/0.1")
            .build()
            .into();
        Self {
            agent,
            key: key.trim().to_string(),
        }
    }

    /// A title's ratings by its TMDB id. `None` when MDBList doesn't know the title.
    pub fn ratings_by_tmdb(
        &self,
        kind: ItemKind,
        tmdb_id: i64,
    ) -> Result<Option<Vec<SourceRating>>> {
        let media = match kind {
            ItemKind::Movie => "movie",
            ItemKind::Show => "show",
        };
        let request = self
            .agent
            .get(format!("{API}/tmdb/{media}/{tmdb_id}/"))
            .query("apikey", &self.key);
        match request.call() {
            Ok(mut response) => {
                let body: Value = response.body_mut().read_json()?;
                Ok(Some(parse_ratings(&body)))
            }
            Err(ureq::Error::StatusCode(404)) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }
}

/// The `ratings` array of a title. Read loosely: sites without data are skipped, and numbers
/// may arrive as numbers or strings.
fn parse_ratings(body: &Value) -> Vec<SourceRating> {
    let number = |v: Option<&Value>| match v {
        Some(Value::Number(n)) => n.as_f64(),
        Some(Value::String(s)) => s.trim().trim_end_matches('%').parse().ok(),
        _ => None,
    };
    body.get("ratings")
        .and_then(Value::as_array)
        .map(|ratings| {
            ratings
                .iter()
                .filter_map(|r| {
                    let source = r.get("source")?.as_str()?.to_string();
                    let value = number(r.get("value"));
                    let score = number(r.get("score"))
                        .filter(|s| (0. ..=100.).contains(s))
                        .map(|s| s.round() as u8);
                    // A site listed without any rating has nothing to show.
                    if value.is_none() && score.is_none() {
                        return None;
                    }
                    let votes = number(r.get("votes")).map(|v| v as u32);
                    Some(SourceRating {
                        source,
                        value,
                        score,
                        votes,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_ratings_loosely() {
        let body: Value = serde_json::from_str(
            r#"{"title":"Alien","ratings":[
                {"source":"imdb","value":8.4,"score":84,"votes":950000},
                {"source":"tomatoes","value":93,"score":93,"votes":null},
                {"source":"letterboxd","value":"4.3","score":86},
                {"source":"rogerebert","value":null,"score":null}]}"#,
        )
        .unwrap();
        let ratings = parse_ratings(&body);
        assert_eq!(ratings.len(), 3, "a site without a rating is skipped");
        assert_eq!(ratings[0].source, "imdb");
        assert_eq!(ratings[0].value, Some(8.4));
        assert_eq!(ratings[0].votes, Some(950_000));
        assert_eq!(ratings[1].score, Some(93));
        assert_eq!(ratings[2].value, Some(4.3));
        assert!(parse_ratings(&serde_json::json!({})).is_empty());
    }
}
