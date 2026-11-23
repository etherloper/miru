//! IMDb, Rotten Tomatoes and Metacritic ratings on detail pages, from OMDb (with the user's
//! own key). Cached in the database and refreshed weekly; fetched when a page opens.

use miru_core::ratings::{RATINGS_MAX_AGE_SECS, TitleRatings};
use miru_metadata::Omdb;

use super::*;

impl LibraryView {
    /// Shows a title's cached ratings and, with an OMDb key, fetches them if they're missing
    /// or a week old.
    pub(super) fn load_ratings(&mut self, item: &LibraryItem, cx: &mut Context<Self>) {
        let key = item.key.clone();
        let cached = self.db.title_ratings(&key).ok().flatten();
        if let Some(ratings) = &cached {
            self.ratings.insert(key.clone(), ratings.clone());
        }
        let Some(omdb_key) = self.settings.omdb_key.clone() else {
            return;
        };
        let now = unix_secs();
        let fresh = cached
            .as_ref()
            .is_some_and(|r| now - r.fetched_at < RATINGS_MAX_AGE_SECS);
        if fresh || !self.ratings_pending.insert(key.clone()) {
            return;
        }
        let tmdb = self.tmdb.clone();
        let tmdb_id = item.metadata.as_ref().and_then(|m| m.tmdb_id);
        let title = item.display_title().to_string();
        let year = key
            .year
            .or_else(|| item.metadata.as_ref().and_then(|m| m.release_year));
        let kind = key.kind;
        cx.spawn(async move |this, cx| {
            let found = cx
                .background_executor()
                .spawn(async move {
                    let omdb = Omdb::new(&omdb_key);
                    // By IMDb id when TMDB knows it (exact); else by title and year.
                    let imdb_id = match (tmdb, tmdb_id) {
                        (Some(tmdb), Some(id)) => tmdb.imdb_id(kind, id).ok().flatten(),
                        _ => None,
                    };
                    match imdb_id {
                        Some(id) => omdb.by_imdb_id(&id),
                        None => omdb.by_title(&title, year, kind),
                    }
                })
                .await;
            this.update(cx, |view, cx| {
                view.ratings_pending.remove(&key);
                match found {
                    Ok(found) => {
                        let found = found.unwrap_or_default();
                        let ratings = TitleRatings {
                            imdb_id: found.imdb_id,
                            imdb_tenths: found.imdb_tenths,
                            imdb_votes: found.imdb_votes,
                            rotten_tomatoes: found.rotten_tomatoes,
                            metacritic: found.metacritic,
                            fetched_at: unix_secs(),
                        };
                        if let Err(e) = view.db.save_title_ratings(&key, &ratings) {
                            tracing::warn!("saving ratings: {e}");
                        }
                        view.ratings.insert(key, ratings);
                        cx.notify();
                    }
                    Err(e) => tracing::warn!("fetching ratings for {}: {e}", key.title),
                }
            })
            .ok();
        })
        .detach();
    }

    /// After the OMDb key changes: fetch ratings for the page that's open.
    pub(super) fn refresh_open_ratings(&mut self, cx: &mut Context<Self>) {
        let item = match &self.page {
            Page::Movie { item, .. } | Page::Show { item, .. } => item.clone(),
            _ => return,
        };
        self.ratings.remove(&item.key);
        if let Err(e) = self.db.save_title_ratings(
            &item.key,
            &TitleRatings {
                fetched_at: 0,
                ..Default::default()
            },
        ) {
            tracing::warn!("resetting ratings: {e}");
        }
        self.load_ratings(&item, cx);
    }
}

fn unix_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}
