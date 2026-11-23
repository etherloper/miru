//! OMDb: IMDb, Rotten Tomatoes and Metacritic ratings for a title, by its IMDb id (from TMDB)
//! or, failing that, by title and year. Needs the user's own OMDb key.

use std::time::Duration;

use miru_core::library::ItemKind;
use serde::Deserialize;

use crate::Result;

const API: &str = "https://www.omdbapi.com/";

#[derive(Clone)]
pub struct Omdb {
    agent: ureq::Agent,
    key: String,
}

/// Ratings OMDb reported. Any may be missing.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OmdbRatings {
    pub imdb_id: Option<String>,
    /// Out of 10, in tenths.
    pub imdb_tenths: Option<u16>,
    pub imdb_votes: Option<u32>,
    /// Percent.
    pub rotten_tomatoes: Option<u8>,
    /// Out of 100.
    pub metacritic: Option<u8>,
}

#[derive(Deserialize)]
struct Response {
    #[serde(rename = "Response")]
    response: String,
    #[serde(rename = "imdbID")]
    imdb_id: Option<String>,
    #[serde(rename = "imdbRating")]
    imdb_rating: Option<String>,
    #[serde(rename = "imdbVotes")]
    imdb_votes: Option<String>,
    #[serde(rename = "Metascore")]
    metascore: Option<String>,
    #[serde(rename = "Ratings", default)]
    ratings: Vec<Rating>,
}

#[derive(Deserialize)]
struct Rating {
    #[serde(rename = "Source")]
    source: String,
    #[serde(rename = "Value")]
    value: String,
}

impl Omdb {
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

    pub fn by_imdb_id(&self, imdb_id: &str) -> Result<Option<OmdbRatings>> {
        self.fetch(self.agent.get(API).query("i", imdb_id))
    }

    pub fn by_title(
        &self,
        title: &str,
        year: Option<u16>,
        kind: ItemKind,
    ) -> Result<Option<OmdbRatings>> {
        let mut request = self.agent.get(API).query("t", title).query(
            "type",
            match kind {
                ItemKind::Movie => "movie",
                ItemKind::Show => "series",
            },
        );
        if let Some(year) = year {
            request = request.query("y", year.to_string());
        }
        self.fetch(request)
    }

    fn fetch(
        &self,
        request: ureq::RequestBuilder<ureq::typestate::WithoutBody>,
    ) -> Result<Option<OmdbRatings>> {
        let response: Response = request
            .query("apikey", &self.key)
            .call()?
            .body_mut()
            .read_json()?;
        Ok((response.response == "True").then(|| parse(response)))
    }
}

fn parse(response: Response) -> OmdbRatings {
    // OMDb writes "N/A" for anything it doesn't have.
    let known = |v: Option<String>| v.filter(|v| !v.is_empty() && v != "N/A");
    let rotten_tomatoes = response
        .ratings
        .iter()
        .find(|r| r.source == "Rotten Tomatoes")
        .and_then(|r| r.value.trim_end_matches('%').parse().ok());
    OmdbRatings {
        imdb_id: known(response.imdb_id),
        imdb_tenths: known(response.imdb_rating)
            .and_then(|r| r.parse::<f32>().ok())
            .map(|r| (r * 10.).round() as u16),
        imdb_votes: known(response.imdb_votes).and_then(|v| v.replace(',', "").parse().ok()),
        rotten_tomatoes,
        metacritic: known(response.metascore).and_then(|m| m.parse().ok()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_omdb_replies() {
        let reply: Response = serde_json::from_str(
            r#"{"Title":"Heat","imdbID":"tt0113277","imdbRating":"8.3","imdbVotes":"750,123",
                "Metascore":"76","Ratings":[{"Source":"Internet Movie Database","Value":"8.3/10"},
                {"Source":"Rotten Tomatoes","Value":"88%"},{"Source":"Metacritic","Value":"76/100"}],
                "Response":"True"}"#,
        )
        .unwrap();
        assert_eq!(
            parse(reply),
            OmdbRatings {
                imdb_id: Some("tt0113277".into()),
                imdb_tenths: Some(83),
                imdb_votes: Some(750_123),
                rotten_tomatoes: Some(88),
                metacritic: Some(76),
            }
        );
        let sparse: Response = serde_json::from_str(
            r#"{"imdbID":"tt1","imdbRating":"N/A","imdbVotes":"N/A","Metascore":"N/A",
                "Response":"True"}"#,
        )
        .unwrap();
        let parsed = parse(sparse);
        assert_eq!(parsed.imdb_tenths, None);
        assert_eq!(parsed.rotten_tomatoes, None);
    }
}
