//! Online metadata for library titles: TMDB lookups and a local artwork cache.
//!
//! All calls are blocking; run them off the UI thread.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use miru_core::db::Db;
use miru_core::episodes::{ArtSource, EpisodeArt};
use miru_core::library::{CastMember, ItemKind, LibraryItem, Metadata, MetadataResult, TitleKey};
use miru_core::parse::ParsedMedia;
use serde::Deserialize;

mod omdb;
pub use omdb::{Omdb, OmdbRatings};

#[derive(Debug, thiserror::Error)]
pub enum MetadataError {
    #[error("http: {0}")]
    Http(#[from] ureq::Error),
    #[error("image: {0}")]
    Image(#[from] image::ImageError),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Db(#[from] miru_core::db::DbError),
}

pub type Result<T> = std::result::Result<T, MetadataError>;

const API: &str = "https://api.themoviedb.org/3";
const IMAGES: &str = "https://image.tmdb.org/t/p";

/// TMDB credential: a v4 read access token (a JWT, starts with `eyJ`) or a v3 API key.
#[derive(Clone)]
enum Credential {
    Bearer(String),
    ApiKey(String),
}

#[derive(Clone)]
pub struct Tmdb {
    agent: ureq::Agent,
    credential: Credential,
}

/// A search candidate.
#[derive(Debug, Clone, PartialEq)]
pub struct TmdbMatch {
    pub id: i64,
    pub title: String,
    pub original_title: Option<String>,
    pub year: Option<u16>,
    pub popularity: f32,
    pub overview: Option<String>,
    /// TMDB poster path, for thumbnails in the Fix match list (see `ArtworkCache::thumbnail`).
    pub poster_path: Option<String>,
}

#[derive(Deserialize)]
struct SearchResponse {
    #[serde(default)]
    results: Vec<SearchResult>,
}

#[derive(Deserialize)]
struct SearchResult {
    id: i64,
    /// Movies use `title`/`release_date`, TV uses `name`/`first_air_date`.
    title: Option<String>,
    name: Option<String>,
    original_title: Option<String>,
    original_name: Option<String>,
    release_date: Option<String>,
    first_air_date: Option<String>,
    popularity: Option<f32>,
    overview: Option<String>,
    poster_path: Option<String>,
}

impl From<SearchResult> for TmdbMatch {
    fn from(r: SearchResult) -> Self {
        Self {
            id: r.id,
            title: r.title.or(r.name).unwrap_or_default(),
            original_title: r.original_title.or(r.original_name),
            year: year_of(r.release_date.as_deref().or(r.first_air_date.as_deref())),
            popularity: r.popularity.unwrap_or(0.),
            overview: r.overview.filter(|o| !o.is_empty()),
            poster_path: r.poster_path,
        }
    }
}

/// Everything shown on a details page, from one `/movie/{id}` or `/tv/{id}` request.
#[derive(Debug, Default, Deserialize)]
struct DetailsResponse {
    title: Option<String>,
    name: Option<String>,
    overview: Option<String>,
    poster_path: Option<String>,
    backdrop_path: Option<String>,
    vote_average: Option<f32>,
    release_date: Option<String>,
    first_air_date: Option<String>,
    runtime: Option<u32>,
    #[serde(default)]
    episode_run_time: Vec<u32>,
    #[serde(default)]
    genres: Vec<Named>,
    images: Option<Images>,
    credits: Option<Credits>,
    aggregate_credits: Option<Credits>,
    release_dates: Option<Countries<CountryReleases>>,
    content_ratings: Option<Countries<CountryRating>>,
}

#[derive(Debug, Deserialize)]
struct Named {
    name: String,
}

#[derive(Debug, Default, Deserialize)]
struct Images {
    #[serde(default)]
    backdrops: Vec<Image>,
    #[serde(default)]
    logos: Vec<Image>,
}

#[derive(Debug, Clone, Deserialize)]
struct Image {
    file_path: String,
    iso_639_1: Option<String>,
    #[serde(default)]
    vote_average: f32,
}

#[derive(Debug, Default, Deserialize)]
struct Credits {
    #[serde(default)]
    cast: Vec<CastEntry>,
}

/// A cast entry from `credits` (movies: `character`) or `aggregate_credits` (shows: one
/// character per `roles` entry, across all seasons).
#[derive(Debug, Deserialize)]
struct CastEntry {
    #[serde(default)]
    id: i64,
    name: String,
    character: Option<String>,
    profile_path: Option<String>,
    #[serde(default)]
    roles: Vec<Role>,
}

#[derive(Debug, Deserialize)]
struct Role {
    character: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Countries<T> {
    #[serde(default)]
    results: Vec<T>,
}

#[derive(Debug, Default, Deserialize)]
struct CountryReleases {
    iso_3166_1: String,
    #[serde(default)]
    release_dates: Vec<Certified>,
}

#[derive(Debug, Deserialize)]
struct Certified {
    #[serde(default)]
    certification: String,
}

#[derive(Debug, Default, Deserialize)]
struct CountryRating {
    iso_3166_1: String,
    #[serde(default)]
    rating: String,
}

/// Regions whose age ratings are preferred, in order.
const RATING_REGIONS: [&str; 2] = ["GB", "US"];

impl DetailsResponse {
    /// The cast from whichever credits were requested.
    fn cast(&self) -> &[CastEntry] {
        self.aggregate_credits
            .as_ref()
            .or(self.credits.as_ref())
            .map_or(&[], |c| c.cast.as_slice())
    }

    fn certification(&self) -> Option<String> {
        RATING_REGIONS.iter().find_map(|region| {
            let from_movie = self.release_dates.as_ref().and_then(|c| {
                c.results
                    .iter()
                    .filter(|r| r.iso_3166_1 == *region)
                    .flat_map(|r| &r.release_dates)
                    .map(|d| d.certification.trim())
                    .find(|c| !c.is_empty())
                    .map(str::to_string)
            });
            from_movie.or_else(|| {
                self.content_ratings.as_ref().and_then(|c| {
                    c.results
                        .iter()
                        .find(|r| r.iso_3166_1 == *region && !r.rating.trim().is_empty())
                        .map(|r| r.rating.trim().to_string())
                })
            })
        })
    }

    /// Backdrops worth rotating through: the main one, then the best-rated textless ones.
    fn backdrop_paths(&self, limit: usize) -> Vec<String> {
        let mut extra: Vec<Image> = self
            .images
            .as_ref()
            .map(|i| {
                i.backdrops
                    .iter()
                    .filter(|b| b.iso_639_1.is_none())
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();
        extra.sort_by(|a, b| b.vote_average.total_cmp(&a.vote_average));
        let mut paths: Vec<String> = self.backdrop_path.iter().cloned().collect();
        for image in extra {
            if paths.len() >= limit {
                break;
            }
            if !paths.contains(&image.file_path) {
                paths.push(image.file_path);
            }
        }
        paths
    }

    /// Best English (or language-neutral) PNG logo.
    fn logo_path(&self) -> Option<String> {
        let logos = &self.images.as_ref()?.logos;
        let rank = |l: &Image| match l.iso_639_1.as_deref() {
            Some("en") => 0,
            None => 1,
            _ => 2,
        };
        logos
            .iter()
            .filter(|l| l.file_path.ends_with(".png") && rank(l) < 2)
            .min_by(|a, b| {
                rank(a)
                    .cmp(&rank(b))
                    .then(b.vote_average.total_cmp(&a.vote_average))
            })
            .map(|l| l.file_path.clone())
    }
}

fn year_of(date: Option<&str>) -> Option<u16> {
    date.and_then(|d| d.get(..4)).and_then(|y| y.parse().ok())
}

impl Tmdb {
    pub fn new(credential: &str) -> Self {
        let credential = credential.trim();
        let credential = if credential.starts_with("eyJ") {
            Credential::Bearer(credential.to_string())
        } else {
            Credential::ApiKey(credential.to_string())
        };
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(15)))
            .user_agent("Miru/0.1")
            .build()
            .into();
        Self { agent, credential }
    }

    /// Finds the best match for a title: candidates are scored on title similarity and year,
    /// and simpler forms of the title are tried when nothing fits ("Dune Part One" → "Dune").
    pub fn search(&self, key: &TitleKey) -> Result<Option<TmdbMatch>> {
        for query in query_variants(&key.title) {
            let candidates = self.search_query(key.kind, &query)?;
            if let Some(best) = best_match(&query, key.year, candidates) {
                return Ok(Some(best));
            }
        }
        Ok(None)
    }

    /// Raw search results in TMDB's relevance order, for the user to pick from.
    pub fn search_query(&self, kind: ItemKind, query: &str) -> Result<Vec<TmdbMatch>> {
        let endpoint = match kind {
            ItemKind::Movie => "movie",
            ItemKind::Show => "tv",
        };
        // No year filter: TMDB's filter can drop the right title (re-releases, festival
        // dates); the year is used for scoring instead.
        let request = self
            .agent
            .get(format!("{API}/search/{endpoint}"))
            .query("query", query)
            .query("include_adult", "false");
        let response: SearchResponse = self.authed(request).call()?.body_mut().read_json()?;
        Ok(response.results.into_iter().map(TmdbMatch::from).collect())
    }

    fn details(&self, kind: ItemKind, id: i64) -> Result<DetailsResponse> {
        // Shows use aggregate credits: the whole run's cast, not just the latest season.
        let (endpoint, credits, ratings) = match kind {
            ItemKind::Movie => ("movie", "credits", "release_dates"),
            ItemKind::Show => ("tv", "aggregate_credits", "content_ratings"),
        };
        let request = self
            .agent
            .get(format!("{API}/{endpoint}/{id}"))
            .query("append_to_response", format!("images,{credits},{ratings}"))
            .query("include_image_language", "en,null");
        Ok(self.authed(request).call()?.body_mut().read_json()?)
    }

    /// A title's IMDb id (e.g. "tt0113277"), for looking up its outside ratings.
    pub fn imdb_id(&self, kind: ItemKind, id: i64) -> Result<Option<String>> {
        #[derive(Deserialize)]
        struct ExternalIds {
            imdb_id: Option<String>,
        }
        let endpoint = match kind {
            ItemKind::Movie => "movie",
            ItemKind::Show => "tv",
        };
        let request = self
            .agent
            .get(format!("{API}/{endpoint}/{id}/external_ids"));
        let ids: ExternalIds = self.authed(request).call()?.body_mut().read_json()?;
        Ok(ids.imdb_id.filter(|i| i.starts_with("tt")))
    }

    /// Episodes of one season of a TV show. A season TMDB doesn't know returns an empty list.
    pub fn tv_season(&self, tv_id: i64, season: u16) -> Result<Vec<TmdbEpisode>> {
        let request = self.agent.get(format!("{API}/tv/{tv_id}/season/{season}"));
        match self.authed(request).call() {
            Ok(mut response) => Ok(response.body_mut().read_json::<SeasonResponse>()?.episodes),
            Err(ureq::Error::StatusCode(404)) => Ok(Vec::new()),
            Err(e) => Err(e.into()),
        }
    }

    fn authed<B>(&self, request: ureq::RequestBuilder<B>) -> ureq::RequestBuilder<B> {
        match &self.credential {
            Credential::Bearer(token) => request.header("Authorization", format!("Bearer {token}")),
            Credential::ApiKey(api_key) => request.query("api_key", api_key),
        }
    }

    fn download(&self, url: &str) -> Result<Vec<u8>> {
        Ok(self
            .agent
            .get(url)
            .call()?
            .body_mut()
            .with_config()
            .limit(20 << 20)
            .read_to_vec()?)
    }
}

/// Lowercase words without punctuation or a leading "the", for comparing titles.
fn normalise(title: &str) -> Vec<String> {
    let mut words: Vec<String> = fold_accents(&title.to_lowercase())
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_string)
        .collect();
    if words.len() > 1 && words[0] == "the" {
        words.remove(0);
    }
    words
}

/// Strips accents from common Latin letters ("shōgun" → "shogun", "amélie" → "amelie") so
/// titles typed without them still match. Expects lowercase input.
fn fold_accents(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        let folded = match c {
            'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' | 'ā' | 'ă' | 'ą' => "a",
            'æ' => "ae",
            'ç' | 'ć' | 'ĉ' | 'ċ' | 'č' => "c",
            'ď' | 'đ' => "d",
            'è' | 'é' | 'ê' | 'ë' | 'ē' | 'ĕ' | 'ė' | 'ę' | 'ě' => "e",
            'ĝ' | 'ğ' | 'ġ' | 'ģ' => "g",
            'ì' | 'í' | 'î' | 'ï' | 'ĩ' | 'ī' | 'ĭ' | 'į' | 'ı' => "i",
            'ł' => "l",
            'ñ' | 'ń' | 'ņ' | 'ň' => "n",
            'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' | 'ō' | 'ŏ' | 'ő' => "o",
            'œ' => "oe",
            'ř' => "r",
            'ś' | 'ŝ' | 'ş' | 'š' => "s",
            'ß' => "ss",
            'ť' | 'ţ' => "t",
            'ù' | 'ú' | 'û' | 'ü' | 'ũ' | 'ū' | 'ŭ' | 'ů' | 'ű' | 'ų' => "u",
            'ý' | 'ÿ' | 'ŷ' => "y",
            'ź' | 'ż' | 'ž' => "z",
            other => {
                out.push(other);
                continue;
            }
        };
        out.push_str(folded);
    }
    out
}

/// 1.0 for the same words, 0.8 when one title's words are all in the other, else overlap.
fn title_similarity(a: &str, b: &str) -> f32 {
    let (a, b) = (normalise(a), normalise(b));
    if a.is_empty() || b.is_empty() {
        return 0.;
    }
    if a == b {
        return 1.;
    }
    let contains = |x: &[String], y: &[String]| x.iter().all(|w| y.contains(w));
    if contains(&a, &b) || contains(&b, &a) {
        return 0.8;
    }
    let shared = a.iter().filter(|w| b.contains(w)).count() as f32;
    let union = (a.len() + b.len()) as f32 - shared;
    shared / union
}

/// Picks the candidate that best fits the query and year, or `None` if nothing fits well.
fn best_match(query: &str, year: Option<u16>, candidates: Vec<TmdbMatch>) -> Option<TmdbMatch> {
    let score = |c: &TmdbMatch| {
        let similarity = title_similarity(query, &c.title).max(
            c.original_title
                .as_deref()
                .map_or(0., |o| title_similarity(query, o)),
        );
        let year_score = match (year, c.year) {
            (Some(want), Some(got)) => match want.abs_diff(got) {
                0 => 0.5,
                1 => 0.25,
                _ => -0.6,
            },
            _ => 0.,
        };
        // Popularity only breaks ties between otherwise equal candidates.
        (
            similarity,
            similarity + year_score + c.popularity.min(100.) / 2000.,
        )
    };
    candidates
        .into_iter()
        .map(|c| {
            let (similarity, total) = score(&c);
            (c, similarity, total)
        })
        .filter(|(_, similarity, total)| *similarity >= 0.5 && *total >= 0.6)
        .max_by(|a, b| a.2.total_cmp(&b.2))
        .map(|(c, ..)| c)
}

/// The title itself, then simpler forms: before a colon or dash, and without a trailing
/// "Part One"/"Part 2"/"Chapter 3" style suffix.
fn query_variants(title: &str) -> Vec<String> {
    let mut variants = vec![title.trim().to_string()];
    let mut push = |v: &str| {
        let v = v.trim();
        if !v.is_empty() && !variants.iter().any(|x| x.eq_ignore_ascii_case(v)) {
            variants.push(v.to_string());
        }
    };
    for separator in [":", " - "] {
        if let Some((head, _)) = title.split_once(separator) {
            push(head);
        }
    }
    let words: Vec<&str> = title.split_whitespace().collect();
    if let Some(pos) = words.iter().rposition(|w| {
        matches!(
            w.to_lowercase().as_str(),
            "part" | "chapter" | "vol" | "vol." | "volume"
        )
    }) && pos > 0
    {
        push(&words[..pos].join(" "));
    }
    variants
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct TmdbEpisode {
    pub episode_number: u16,
    pub name: Option<String>,
    pub overview: Option<String>,
    pub still_path: Option<String>,
    pub runtime: Option<u32>,
}

#[derive(Deserialize)]
struct SeasonResponse {
    #[serde(default)]
    episodes: Vec<TmdbEpisode>,
}

/// Stores posters, backdrops and episode stills on disk, named by TMDB id (or file id for
/// grabbed frames).
pub struct ArtworkCache {
    dir: PathBuf,
}

impl ArtworkCache {
    pub fn new(dir: impl Into<PathBuf>) -> std::io::Result<Self> {
        let dir = dir.into();
        std::fs::create_dir_all(&dir)?;
        Ok(Self { dir })
    }

    /// Where a frame grabbed from library file `file_id` is stored.
    pub fn frame_path(&self, file_id: i64) -> std::io::Result<PathBuf> {
        let dir = self.dir.join("frames");
        std::fs::create_dir_all(&dir)?;
        Ok(dir.join(format!("{file_id}.jpg")))
    }

    /// A cast member's photo, cached per person (shared by every title they appear in).
    pub fn profile(&self, tmdb: &Tmdb, person_id: i64, remote: &str) -> Result<PathBuf> {
        self.fetch(tmdb, "w185", remote, &format!("person-{person_id}.jpg"))
    }

    /// Small poster for a search candidate, cached like other artwork.
    pub fn thumbnail(&self, tmdb: &Tmdb, remote: &str) -> Result<PathBuf> {
        let name: String = remote
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '.' {
                    c
                } else {
                    '-'
                }
            })
            .collect();
        self.fetch(tmdb, "w154", remote, &format!("thumb{name}"))
    }

    fn fetch(&self, tmdb: &Tmdb, size: &str, remote: &str, file: &str) -> Result<PathBuf> {
        let path = self.dir.join(file);
        if !path.exists() {
            let bytes = tmdb.download(&format!("{IMAGES}/{size}{remote}"))?;
            // Write via a temp file so a crash never leaves a truncated image behind.
            let tmp = path.with_extension("part");
            std::fs::write(&tmp, &bytes)?;
            std::fs::rename(&tmp, &path)?;
        }
        Ok(path)
    }
}

/// Average colour of an image, darkened so white text stays readable on it.
pub fn accent_color(path: &Path) -> Result<u32> {
    let thumb = image::open(path)?.thumbnail(16, 16).to_rgb8();
    let count = u64::from(thumb.width() * thumb.height()).max(1);
    let [mut r, mut g, mut b] = [0u64; 3];
    for p in thumb.pixels() {
        r += u64::from(p[0]);
        g += u64::from(p[1]);
        b += u64::from(p[2]);
    }
    let scale = |c: u64| ((c / count) * 3 / 5) as u32;
    Ok((scale(r) << 16) | (scale(g) << 8) | scale(b))
}

/// Backdrops cached per title, rotated between launches.
const BACKDROPS_PER_TITLE: usize = 3;
const CAST_SHOWN: usize = 6;
/// Cast members kept per title for the cast pane.
const CAST_STORED: usize = 40;

/// Looks up one title and downloads its artwork. Network failures are returned as errors
/// (so the title is retried later); a definitive miss is returned as `NotFound`.
pub fn lookup(tmdb: &Tmdb, cache: &ArtworkCache, key: &TitleKey) -> Result<MetadataResult> {
    let Some(found) = tmdb.search(key)? else {
        return Ok(MetadataResult::NotFound);
    };
    lookup_id(tmdb, cache, key, key.kind, found.id)
}

/// Fetches details and artwork for a known TMDB entry of `kind` (which may differ from the
/// kind of `key` after a manual match).
pub fn lookup_id(
    tmdb: &Tmdb,
    cache: &ArtworkCache,
    key: &TitleKey,
    kind: ItemKind,
    id: i64,
) -> Result<MetadataResult> {
    let details = tmdb.details(kind, id)?;
    let prefix = match kind {
        ItemKind::Movie => "movie",
        ItemKind::Show => "tv",
    };
    let to_string = |p: PathBuf| p.to_str().map(str::to_string);

    let poster = details
        .poster_path
        .as_deref()
        .map(|p| cache.fetch(tmdb, "w342", p, &format!("{prefix}-{id}-poster.jpg")))
        .transpose()?;
    let mut backdrops = Vec::new();
    for (n, remote) in details
        .backdrop_paths(BACKDROPS_PER_TITLE)
        .iter()
        .enumerate()
    {
        match cache.fetch(
            tmdb,
            "w1280",
            remote,
            &format!("{prefix}-{id}-backdrop-{n}.jpg"),
        ) {
            Ok(path) => backdrops.extend(to_string(path)),
            Err(e) => tracing::warn!(title = %key.title, "backdrop download failed: {e}"),
        }
    }
    let logo = details.logo_path().and_then(|remote| {
        cache
            .fetch(tmdb, "w500", &remote, &format!("{prefix}-{id}-logo.png"))
            .inspect_err(|e| tracing::warn!(title = %key.title, "logo download failed: {e}"))
            .ok()
    });
    let accent_rgb = poster.as_deref().and_then(|p| accent_color(p).ok());
    let runtime = details
        .runtime
        .or_else(|| details.episode_run_time.first().copied())
        .filter(|r| *r > 0);

    Ok(MetadataResult::Matched(Metadata {
        tmdb_id: Some(id),
        display_title: details
            .title
            .clone()
            .or(details.name.clone())
            .filter(|t| !t.is_empty()),
        overview: details.overview.clone().filter(|o| !o.is_empty()),
        poster_file: poster.and_then(to_string),
        backdrop_file: backdrops.first().cloned(),
        accent_rgb,
        backdrops,
        logo_file: logo.and_then(to_string),
        rating_tenths: details
            .vote_average
            .filter(|v| *v > 0.)
            .map(|v| (v * 10.).round() as u16),
        certification: details.certification(),
        cast: details
            .cast()
            .iter()
            .take(CAST_SHOWN)
            .map(|p| p.name.clone())
            .collect(),
        credits: details
            .cast()
            .iter()
            .filter(|p| p.id != 0)
            .take(CAST_STORED)
            .map(|p| CastMember {
                person_id: p.id,
                name: p.name.clone(),
                character: p
                    .character
                    .clone()
                    .or_else(|| p.roles.first().and_then(|r| r.character.clone()))
                    .filter(|c| !c.trim().is_empty()),
                profile_path: p.profile_path.clone(),
            })
            .collect(),
        genres: details.genres.iter().map(|g| g.name.clone()).collect(),
        runtime_min: runtime,
        release_year: year_of(
            details
                .release_date
                .as_deref()
                .or(details.first_air_date.as_deref()),
        ),
    }))
}

const POSTER_NAMES: &[&str] = &["poster", "folder", "cover"];
const BACKDROP_NAMES: &[&str] = &["fanart", "backdrop", "background"];
const ART_EXTENSIONS: &[&str] = &["jpg", "jpeg", "png", "webp"];

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct LocalArtwork {
    pub poster: Option<PathBuf>,
    pub backdrop: Option<PathBuf>,
}

/// Finds artwork stored next to the media (`poster.jpg`, `fanart.jpg`, ...), following the
/// Jellyfin/Kodi convention. Movies look in the file's folder; shows look in the show folder
/// above any `Season N` folder.
pub fn local_artwork(media_file: &Path, kind: ItemKind) -> LocalArtwork {
    let Some(mut dir) = media_file.parent() else {
        return LocalArtwork::default();
    };
    if kind == ItemKind::Show {
        let is_season = dir.file_name().and_then(|n| n.to_str()).is_some_and(|n| {
            n.to_lowercase().starts_with("season") || n.to_lowercase().starts_with("series")
        });
        if is_season && let Some(parent) = dir.parent() {
            dir = parent;
        }
    }
    let find = |names: &[&str]| {
        names.iter().find_map(|name| {
            ART_EXTENSIONS
                .iter()
                .map(|ext| dir.join(format!("{name}.{ext}")))
                .find(|p| p.is_file())
        })
    };
    LocalArtwork {
        poster: find(POSTER_NAMES),
        backdrop: find(BACKDROP_NAMES),
    }
}

/// Fills in metadata for titles that have none yet, combining local artwork with TMDB (when a
/// credential is configured). Returns how many titles were saved.
///
/// Without TMDB every pending title is checked in one pass (filesystem only); with TMDB at most
/// `limit` titles are looked up so the caller can refresh the UI between batches.
pub fn enrich_missing(
    db: &Db,
    tmdb: Option<&Tmdb>,
    cache: &ArtworkCache,
    limit: u32,
) -> Result<usize> {
    let limit = if tmdb.is_some() { limit } else { u32::MAX };
    let mut pending: Vec<LibraryItem> = db.missing_metadata(ItemKind::Movie, limit)?;
    let remaining = limit.saturating_sub(pending.len() as u32);
    pending.extend(db.missing_metadata(ItemKind::Show, remaining)?);

    let mut saved = 0;
    for item in pending {
        let path = db.file_path(item.file_id)?;
        let local = match &path {
            Some(path) => local_artwork(Path::new(path), item.key.kind),
            None => LocalArtwork::default(),
        };
        // Shows are stored without a year; use the show folder's year (if any) to tell
        // remakes apart when searching. The result is still saved under the item's own key.
        let search_key = match (item.key.kind, item.key.year, path.as_deref()) {
            (ItemKind::Show, None, Some(path)) => TitleKey {
                year: miru_core::parse::show_year_hint(Path::new(path)),
                ..item.key.clone()
            },
            _ => item.key.clone(),
        };
        let manual = db.match_override(&item.key)?;
        let online = match tmdb
            .map(|t| match manual {
                Some((kind, id)) => lookup_id(t, cache, &item.key, kind, id),
                None => lookup(t, cache, &search_key),
            })
            .transpose()
        {
            Ok(online) => online,
            Err(e) => {
                // Leave the title pending so it is retried on the next sync.
                tracing::warn!(title = %item.key.title, "metadata lookup failed: {e}");
                continue;
            }
        };
        if let Some(result) = merge(local, online) {
            db.save_metadata(&item.key, &result)?;
            saved += 1;
        }
    }
    Ok(saved)
}

/// Matches a title to the TMDB entry the user picked: remembers the choice (so later refreshes
/// keep it) and fetches the new details and artwork right away.
pub fn apply_match(
    db: &Db,
    tmdb: &Tmdb,
    cache: &ArtworkCache,
    item: &LibraryItem,
    kind: ItemKind,
    id: i64,
) -> Result<()> {
    let online = lookup_id(tmdb, cache, &item.key, kind, id)?;
    let local = match db.file_path(item.file_id)? {
        Some(path) => local_artwork(Path::new(&path), item.key.kind),
        None => LocalArtwork::default(),
    };
    db.set_match_override(&item.key, kind, id)?;
    if let Some(result) = merge(local, Some(online)) {
        db.save_metadata(&item.key, &result)?;
    }
    Ok(())
}

/// Grabs a still frame from a video file: `(video, output_jpeg)`.
pub type FrameGrabber<'a> = &'a dyn Fn(&Path, &Path) -> std::result::Result<(), String>;

/// What TMDB said about one episode.
enum EpisodeLookup {
    /// No credential, or the show isn't matched yet: nothing to ask.
    Unavailable,
    /// The request failed; try again later.
    Failed,
    /// TMDB has no such episode.
    Missing,
    Found(TmdbEpisode),
}

/// Fills in artwork for up to `limit` episodes of `show`: TMDB stills, names and synopses where
/// available, otherwise a frame grabbed from the file. Returns how many episodes were saved.
pub fn enrich_episodes(
    db: &Db,
    tmdb: Option<&Tmdb>,
    cache: &ArtworkCache,
    show: &str,
    limit: u32,
    grab: FrameGrabber<'_>,
) -> Result<usize> {
    let pending = db.episodes_needing_art(show, tmdb.is_some(), limit)?;
    if pending.is_empty() {
        return Ok(0);
    }
    let key = TitleKey {
        kind: ItemKind::Show,
        title: show.to_string(),
        year: None,
    };
    let tv_id = match tmdb {
        Some(_) => db.metadata(&key)?.and_then(|m| m.tmdb_id),
        None => None,
    };
    // A show TMDB definitively doesn't know counts as checked; one not looked up yet doesn't.
    let show_unknown_to_tmdb = tmdb.is_some()
        && tv_id.is_none()
        && !db
            .missing_metadata(ItemKind::Show, u32::MAX)?
            .iter()
            .any(|i| i.key.title.eq_ignore_ascii_case(show));
    let existing = db.episode_art(show)?;
    let mut seasons: HashMap<u16, Option<Vec<TmdbEpisode>>> = HashMap::new();

    let mut saved = 0;
    for file in pending {
        let ParsedMedia::Episode {
            season, episode, ..
        } = file.media
        else {
            continue;
        };
        let lookup = match (tmdb, tv_id) {
            (Some(t), Some(id)) => {
                let episodes =
                    seasons
                        .entry(season)
                        .or_insert_with(|| match t.tv_season(id, season) {
                            Ok(eps) => Some(eps),
                            Err(e) => {
                                tracing::warn!(show, season, "TMDB season lookup failed: {e}");
                                None
                            }
                        });
                match episodes {
                    None => EpisodeLookup::Failed,
                    Some(eps) => eps
                        .iter()
                        .find(|e| e.episode_number == episode)
                        .cloned()
                        .map_or(EpisodeLookup::Missing, EpisodeLookup::Found),
                }
            }
            _ => EpisodeLookup::Unavailable,
        };

        let tmdb_still = match (&lookup, tmdb, tv_id) {
            (EpisodeLookup::Found(ep), Some(t), Some(id)) => {
                ep.still_path.as_deref().and_then(|remote| {
                    let name = format!("tv-{id}-s{season}e{episode}-still.jpg");
                    cache
                        .fetch(t, "original", remote, &name)
                        .inspect_err(|e| {
                            tracing::warn!(show, season, episode, "still download failed: {e}")
                        })
                        .ok()
                })
            }
            _ => None,
        };
        let previous_frame = existing
            .get(&file.id)
            .filter(|a| a.source == ArtSource::Frame)
            .and_then(|a| a.still_file.clone())
            .filter(|p| Path::new(p).is_file());
        let (source, still_file) = if let Some(still) = tmdb_still {
            (ArtSource::Tmdb, still.to_str().map(str::to_string))
        } else if let Some(frame) = previous_frame {
            (ArtSource::Frame, Some(frame))
        } else {
            let out = cache.frame_path(file.id)?;
            match grab(Path::new(&file.path), &out) {
                Ok(()) => (ArtSource::Frame, out.to_str().map(str::to_string)),
                Err(e) => {
                    tracing::warn!(path = %file.path, "frame grab failed: {e}");
                    (ArtSource::None, None)
                }
            }
        };
        let details = match &lookup {
            EpisodeLookup::Found(ep) => Some(ep),
            _ => None,
        };
        let art = EpisodeArt {
            source,
            still_file,
            name: details
                .and_then(|e| e.name.clone())
                .filter(|n| !n.is_empty()),
            overview: details
                .and_then(|e| e.overview.clone())
                .filter(|o| !o.is_empty()),
            runtime_min: details.and_then(|e| e.runtime),
            tmdb_checked: matches!(lookup, EpisodeLookup::Found(_) | EpisodeLookup::Missing)
                || show_unknown_to_tmdb,
        };
        db.save_episode_art(file.id, &art)?;
        saved += 1;
    }
    Ok(saved)
}

/// Local artwork wins over downloaded artwork; TMDB supplies titles and overviews.
/// Returns `None` when there is nothing to record yet (no local art and no TMDB configured).
fn merge(local: LocalArtwork, online: Option<MetadataResult>) -> Option<MetadataResult> {
    let has_local = local.poster.is_some() || local.backdrop.is_some();
    let mut meta = match online {
        Some(MetadataResult::Matched(meta)) => meta,
        Some(MetadataResult::NotFound) if !has_local => return Some(MetadataResult::NotFound),
        None if !has_local => return None,
        Some(MetadataResult::NotFound) | None => Metadata::default(),
    };
    let to_string = |p: PathBuf| p.to_str().map(str::to_string);
    if let Some(poster) = local.poster.and_then(to_string) {
        meta.accent_rgb = accent_color(Path::new(&poster)).ok().or(meta.accent_rgb);
        meta.poster_file = Some(poster);
    }
    if let Some(backdrop) = local.backdrop.and_then(to_string) {
        meta.backdrop_file = Some(backdrop);
    }
    Some(MetadataResult::Matched(meta))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(id: i64, title: &str, year: u16, popularity: f32) -> TmdbMatch {
        TmdbMatch {
            id,
            title: title.into(),
            original_title: None,
            year: Some(year),
            popularity,
            overview: None,
            poster_path: None,
        }
    }

    #[test]
    fn parses_search_results() {
        let response: SearchResponse = serde_json::from_str(
            r#"{"results":[
                {"id":27205,"title":"Inception","release_date":"2010-07-15","popularity":80.5},
                {"id":95396,"name":"Severance","first_air_date":"2022-02-17"},
                {"id":1,"title":"Undated","release_date":""}]}"#,
        )
        .unwrap();
        let all: Vec<TmdbMatch> = response.results.into_iter().map(TmdbMatch::from).collect();
        assert_eq!(
            (all[0].title.as_str(), all[0].year),
            ("Inception", Some(2010))
        );
        assert_eq!(
            (all[1].title.as_str(), all[1].year),
            ("Severance", Some(2022))
        );
        assert_eq!(all[2].year, None);
    }

    #[test]
    fn year_beats_popularity() {
        // A bare "Blade Runner" must not become the more popular sequel.
        let candidates = vec![
            candidate(335984, "Blade Runner 2049", 2017, 90.),
            candidate(78, "Blade Runner", 1982, 40.),
        ];
        assert_eq!(
            best_match("Blade Runner", Some(1982), candidates.clone())
                .unwrap()
                .id,
            78
        );
        assert_eq!(
            best_match("Blade Runner 2049", Some(2017), candidates)
                .unwrap()
                .id,
            335984
        );
    }

    #[test]
    fn unrelated_titles_are_rejected() {
        let candidates = vec![candidate(
            656908,
            "Ramayana: The Legend of Prince Rama",
            1993,
            30.,
        )];
        assert!(best_match("Dune Part One", Some(2021), candidates).is_none());
        // The simplified query then finds the film.
        let dune = vec![
            candidate(438631, "Dune", 2021, 95.),
            candidate(841, "Dune", 1984, 20.),
        ];
        assert_eq!(best_match("Dune", Some(2021), dune).unwrap().id, 438631);
    }

    #[test]
    fn accents_are_ignored_and_years_pick_the_remake() {
        assert_eq!(title_similarity("Shogun", "Shōgun"), 1.0);
        assert_eq!(title_similarity("Amelie", "Amélie"), 1.0);
        let shows = vec![
            candidate(1, "Abarenbo Shogun", 1978, 15.),
            candidate(2, "Shōgun", 1980, 20.),
            candidate(3, "Shōgun", 2024, 200.),
        ];
        // An exact (accent-folded) title beats one that merely contains the word.
        assert_eq!(best_match("Shogun", None, shows.clone()).unwrap().id, 3);
        assert_eq!(
            best_match("Shogun", Some(1980), shows.clone()).unwrap().id,
            2
        );
        assert_eq!(best_match("Shogun", Some(2024), shows).unwrap().id, 3);
    }

    #[test]
    fn query_variants_simplify_titles() {
        assert_eq!(query_variants("Dune Part One"), ["Dune Part One", "Dune"]);
        assert_eq!(
            query_variants("Mission: Impossible - Fallout"),
            [
                "Mission: Impossible - Fallout",
                "Mission",
                "Mission: Impossible"
            ]
        );
        assert_eq!(query_variants("Heat"), ["Heat"]);
    }

    #[test]
    fn details_pick_ratings_backdrops_and_logo() {
        let details: DetailsResponse = serde_json::from_str(
            r#"{"title":"Dune","backdrop_path":"/main.jpg","vote_average":7.8,
                "release_date":"2021-09-15","runtime":155,"genres":[{"name":"Science Fiction"}],
                "images":{"backdrops":[
                    {"file_path":"/text.jpg","iso_639_1":"en","vote_average":9.0},
                    {"file_path":"/a.jpg","iso_639_1":null,"vote_average":5.0},
                    {"file_path":"/b.jpg","iso_639_1":null,"vote_average":6.0}],
                  "logos":[
                    {"file_path":"/fr.png","iso_639_1":"fr","vote_average":9.0},
                    {"file_path":"/en.svg","iso_639_1":"en","vote_average":9.0},
                    {"file_path":"/en.png","iso_639_1":"en","vote_average":5.0}]},
                "credits":{"cast":[{"name":"Timothée Chalamet"},{"name":"Rebecca Ferguson"}]},
                "release_dates":{"results":[
                    {"iso_3166_1":"US","release_dates":[{"certification":"PG-13"}]},
                    {"iso_3166_1":"GB","release_dates":[{"certification":""},{"certification":"12A"}]}]}}"#,
        )
        .unwrap();
        assert_eq!(details.certification().as_deref(), Some("12A"));
        assert_eq!(details.backdrop_paths(3), ["/main.jpg", "/b.jpg", "/a.jpg"]);
        assert_eq!(details.logo_path().as_deref(), Some("/en.png"));

        let tv: DetailsResponse = serde_json::from_str(
            r#"{"name":"Severance","content_ratings":{"results":[{"iso_3166_1":"US","rating":"TV-MA"}]}}"#,
        )
        .unwrap();
        assert_eq!(tv.certification().as_deref(), Some("TV-MA"));
    }

    #[test]
    fn show_cast_comes_from_aggregate_credits() {
        let details: DetailsResponse = serde_json::from_str(
            r#"{"name":"Dark","aggregate_credits":{"cast":[
                {"id":7,"name":"Louis Hofmann","profile_path":"/l.jpg",
                 "roles":[{"character":"Jonas Kahnwald"}]},
                {"id":8,"name":"Lisa Vicari","roles":[]}]}}"#,
        )
        .unwrap();
        let cast = details.cast();
        assert_eq!(cast.len(), 2);
        assert_eq!(cast[0].id, 7);
        assert_eq!(
            cast[0].roles.first().and_then(|r| r.character.as_deref()),
            Some("Jonas Kahnwald")
        );
        assert!(cast[1].profile_path.is_none());
    }

    #[test]
    fn credential_kind_is_detected() {
        assert!(matches!(
            Tmdb::new("eyJhbGciOi.x.y").credential,
            Credential::Bearer(_)
        ));
        assert!(
            matches!(Tmdb::new(" 0123abcd ").credential, Credential::ApiKey(ref k) if k == "0123abcd")
        );
    }

    #[test]
    fn finds_local_artwork_for_movies_and_shows() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let write = |rel: &str| {
            let p = root.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, b"x").unwrap();
        };
        write("Arrival (2016)/Arrival.mkv");
        write("Arrival (2016)/folder.png");
        write("Arrival (2016)/fanart.jpg");
        write("Dark/poster.jpg");
        write("Dark/Season 1/Dark.S01E01.mkv");
        write("Loose.Movie.2001.mkv");

        let movie = local_artwork(&root.join("Arrival (2016)/Arrival.mkv"), ItemKind::Movie);
        assert_eq!(movie.poster, Some(root.join("Arrival (2016)/folder.png")));
        assert_eq!(movie.backdrop, Some(root.join("Arrival (2016)/fanart.jpg")));

        let show = local_artwork(&root.join("Dark/Season 1/Dark.S01E01.mkv"), ItemKind::Show);
        assert_eq!(show.poster, Some(root.join("Dark/poster.jpg")));
        assert_eq!(show.backdrop, None);

        assert_eq!(
            local_artwork(&root.join("Loose.Movie.2001.mkv"), ItemKind::Movie),
            LocalArtwork::default()
        );
    }

    #[test]
    fn merge_prefers_local_art_and_defers_when_nothing_known() {
        assert!(merge(LocalArtwork::default(), None).is_none());
        assert!(matches!(
            merge(LocalArtwork::default(), Some(MetadataResult::NotFound)),
            Some(MetadataResult::NotFound)
        ));
        let local = LocalArtwork {
            poster: None,
            backdrop: Some(PathBuf::from("/a/fanart.jpg")),
        };
        let online = MetadataResult::Matched(Metadata {
            overview: Some("o".into()),
            backdrop_file: Some("/cache/b.jpg".into()),
            ..Metadata::default()
        });
        let Some(MetadataResult::Matched(m)) = merge(local, Some(online)) else {
            panic!()
        };
        assert_eq!(m.overview.as_deref(), Some("o"));
        assert_eq!(m.backdrop_file.as_deref(), Some("/a/fanart.jpg"));
    }

    #[test]
    fn parses_season_episodes() {
        let season: SeasonResponse = serde_json::from_str(
            r#"{"season_number":1,"episodes":[
                {"episode_number":1,"name":"Good News About Hell","overview":"Mark leads a team.",
                 "still_path":"/s1e1.jpg","runtime":57},
                {"episode_number":2,"name":"Half Loop","overview":"","still_path":null,"runtime":null}]}"#,
        )
        .unwrap();
        assert_eq!(season.episodes.len(), 2);
        assert_eq!(season.episodes[0].runtime, Some(57));
        assert_eq!(season.episodes[1].still_path, None);
        let empty: SeasonResponse = serde_json::from_str("{}").unwrap();
        assert!(empty.episodes.is_empty());
    }

    #[test]
    fn enrich_episodes_grabs_frames_without_tmdb() {
        use miru_core::db::Fingerprint;

        let dir = tempfile::tempdir().unwrap();
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
        let cache = ArtworkCache::new(dir.path()).unwrap();
        // Fake grabber: succeeds for episodes 1-2, fails for 3.
        let grab = |video: &Path, out: &Path| {
            if video.to_string_lossy().contains("d3") {
                return Err("corrupt file".to_string());
            }
            std::fs::write(out, b"jpeg").map_err(|e| e.to_string())
        };

        assert_eq!(
            enrich_episodes(&db, None, &cache, "Dark", 2, &grab).unwrap(),
            2
        );
        assert_eq!(
            enrich_episodes(&db, None, &cache, "Dark", 10, &grab).unwrap(),
            1
        );
        assert_eq!(
            enrich_episodes(&db, None, &cache, "Dark", 10, &grab).unwrap(),
            0,
            "nothing left to do"
        );

        let art = db.episode_art("Dark").unwrap();
        let mut sources: Vec<_> = art
            .values()
            .map(|a| (a.source, a.still_file.is_some(), a.tmdb_checked))
            .collect();
        sources.sort_by_key(|(s, ..)| format!("{s:?}"));
        assert_eq!(
            sources,
            [
                (ArtSource::Frame, true, false),
                (ArtSource::Frame, true, false),
                (ArtSource::None, false, false)
            ]
        );
    }

    #[test]
    fn accent_is_darkened_average() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("red.png");
        image::RgbImage::from_pixel(4, 4, image::Rgb([200, 100, 0]))
            .save(&path)
            .unwrap();
        assert_eq!(accent_color(&path).unwrap(), (120 << 16) | (60 << 8));
    }
}

#[cfg(test)]
mod live {
    use super::*;

    /// Hits the real TMDB API using TMDB_API_KEY from the workspace `.env`. Prints only titles.
    /// Run with `cargo test -p miru-metadata live -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn live_matching() {
        let env = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../.env"))
            .expect(".env");
        let key = env
            .lines()
            .find_map(|l| {
                l.trim()
                    .strip_prefix("TMDB_API_KEY=")
                    .map(|v| v.trim().trim_matches('"').to_string())
            })
            .filter(|k| !k.is_empty())
            .expect("TMDB_API_KEY in .env");
        let tmdb = Tmdb::new(&key);
        for (title, year) in [
            ("Shogun", Some(2024)),
            ("Shogun", None),
            ("Severance", Some(2022)),
            ("Dark", None),
            ("Yes, Prime Minister", Some(1986)),
            ("Yes, Prime Minister", None),
            ("Dead End Job", Some(2026)),
            ("Dead End Job", None),
            ("Our Blues", Some(2022)),
            ("Reply 1988", Some(2015)),
            ("Reply 1988", None),
        ] {
            let key = TitleKey {
                kind: ItemKind::Show,
                title: title.into(),
                year,
            };
            let found = tmdb.search(&key).unwrap();
            println!(
                "TV {title} {year:?} -> {:?}",
                found.map(|m| format!("{} ({:?}) id={}", m.title, m.year, m.id))
            );
        }
        for (title, year) in [
            ("Dune Part One", Some(2021)),
            ("Dune Part Two", Some(2024)),
            ("Blade Runner", Some(1982)),
            ("Blade Runner 2049", Some(2017)),
            ("Heat", Some(1995)),
        ] {
            let key = TitleKey {
                kind: ItemKind::Movie,
                title: title.into(),
                year,
            };
            let found = tmdb.search(&key).unwrap();
            let summary = found
                .as_ref()
                .map(|m| format!("{} ({:?}) id={}", m.title, m.year, m.id));
            println!("{title} {year:?} -> {summary:?}");
            if let Some(m) = found {
                let d = tmdb.details(ItemKind::Movie, m.id).unwrap();
                println!(
                    "   cert={:?} rating={:?} backdrops={} logo={:?} cast={:?}",
                    d.certification(),
                    d.vote_average,
                    d.backdrop_paths(3).len(),
                    d.logo_path().is_some(),
                    d.cast()
                        .iter()
                        .take(3)
                        .map(|p| p.name.as_str())
                        .collect::<Vec<_>>(),
                );
            }
        }
    }
}
