//! Ratings from other sites on detail pages, from MDBList (with the user's own key): which
//! sites exist and how each one's badge looks, fetching and caching, and the Settings rows
//! for choosing which sites to show.

use miru_core::ratings::{RATING_SITES, RATINGS_MAX_AGE_SECS, SourceRating, TitleRatings};
use miru_metadata::Mdblist;

use super::*;

impl LibraryView {
    /// Shows a title's cached ratings and, with an MDBList key, fetches them if they're missing
    /// or a week old.
    pub(super) fn load_ratings(&mut self, item: &LibraryItem, cx: &mut Context<Self>) {
        let key = item.key.clone();
        let cached = self.db.title_ratings(&key).ok().flatten();
        if let Some(ratings) = &cached {
            self.ratings.insert(key.clone(), ratings.clone());
        }
        let Some(api_key) = self.settings.mdblist_key.clone() else {
            return;
        };
        // MDBList is asked by TMDB id, so titles without a TMDB match have nothing to look up.
        let Some(tmdb_id) = item.metadata.as_ref().and_then(|m| m.tmdb_id) else {
            return;
        };
        let now = unix_secs();
        let fresh = cached
            .as_ref()
            .is_some_and(|r| now - r.fetched_at < RATINGS_MAX_AGE_SECS);
        if fresh || !self.ratings_pending.insert(key.clone()) {
            return;
        }
        let kind = key.kind;
        cx.spawn(async move |this, cx| {
            let found = cx
                .background_executor()
                .spawn(async move { Mdblist::new(&api_key).ratings_by_tmdb(kind, tmdb_id) })
                .await;
            this.update(cx, |view, cx| {
                view.ratings_pending.remove(&key);
                match found {
                    Ok(found) => {
                        let ratings = TitleRatings {
                            ratings: found.unwrap_or_default(),
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

    /// While a grid sorts by a site's rating, fetches ratings for its titles that never had
    /// them (most are only fetched when their page opens), one at a time, re-sorting as they
    /// arrive. Stops at the first error, such as MDBList's daily limit; the rest are tried on
    /// the next launch or library sync.
    pub(super) fn fill_sort_ratings(&mut self, cx: &mut Context<Self>) {
        if self.ratings_filling {
            return;
        }
        let Some(api_key) = self.settings.mdblist_key.clone() else {
            return;
        };
        let mut wanted: Vec<(TitleKey, i64)> = Vec::new();
        for (sort, items) in [
            (self.settings.movie_sort, &self.movies),
            (self.settings.show_sort, &self.shows),
        ] {
            if !matches!(sort, LibrarySort::Site(_)) {
                continue;
            }
            wanted.extend(
                items
                    .iter()
                    .filter(|i| {
                        !self.ratings.contains_key(&i.key) && !self.ratings_pending.contains(&i.key)
                    })
                    .filter_map(|i| Some((i.key.clone(), i.metadata.as_ref()?.tmdb_id?))),
            );
        }
        if wanted.is_empty() {
            return;
        }
        self.ratings_filling = true;
        let keys: Vec<TitleKey> = wanted.iter().map(|(key, _)| key.clone()).collect();
        self.ratings_pending.extend(keys.iter().cloned());
        let client = Mdblist::new(&api_key);
        cx.spawn(async move |this, cx| {
            let total = wanted.len();
            for (n, (key, tmdb_id)) in wanted.into_iter().enumerate() {
                let kind = key.kind;
                let mdblist = client.clone();
                let found = cx
                    .background_executor()
                    .spawn(async move { mdblist.ratings_by_tmdb(kind, tmdb_id) })
                    .await;
                let keep_going = this
                    .update(cx, |view, cx| {
                        view.ratings_pending.remove(&key);
                        let found = match found {
                            Ok(found) => found,
                            Err(e) => {
                                tracing::warn!("fetching ratings for sorting: {e}");
                                return false;
                            }
                        };
                        let ratings = TitleRatings {
                            ratings: found.unwrap_or_default(),
                            fetched_at: unix_secs(),
                        };
                        if let Err(e) = view.db.save_title_ratings(&key, &ratings) {
                            tracing::warn!("saving ratings: {e}");
                        }
                        view.ratings.insert(key, ratings);
                        // In batches, so the grid doesn't reshuffle with every title.
                        if (n + 1) % 10 == 0 || n + 1 == total {
                            view.resort(cx);
                        }
                        true
                    })
                    .unwrap_or(false);
                if !keep_going {
                    break;
                }
            }
            this.update(cx, |view, cx| {
                view.ratings_filling = false;
                for key in &keys {
                    view.ratings_pending.remove(key);
                }
                view.resort(cx);
            })
            .ok();
        })
        .detach();
    }

    /// Re-sorts both grids by their current order, e.g. as ratings arrive.
    fn resort(&mut self, cx: &mut Context<Self>) {
        let mut movies = (*self.movies).clone();
        sort_items(&mut movies, self.settings.movie_sort, &self.ratings);
        self.movies = Rc::new(movies);
        let mut shows = (*self.shows).clone();
        sort_items(&mut shows, self.settings.show_sort, &self.ratings);
        self.shows = Rc::new(shows);
        self.apply_filters();
        cx.notify();
    }

    /// After the MDBList key changes: fetch ratings afresh for the page that's open.
    pub(super) fn refresh_open_ratings(&mut self, cx: &mut Context<Self>) {
        let item = match &self.page {
            Page::Movie { item, .. } | Page::Show { item, .. } => item.clone(),
            _ => return,
        };
        self.ratings.remove(&item.key);
        if let Err(e) = self
            .db
            .save_title_ratings(&item.key, &TitleRatings::default())
        {
            tracing::warn!("resetting ratings: {e}");
        }
        self.load_ratings(&item, cx);
    }

    /// The rating badges for a detail page: the chosen sites that have a rating, in order.
    /// Empty without an MDBList key (the page then shows TMDB's score instead).
    pub(super) fn rating_badges(&self, key: &TitleKey) -> Vec<AnyElement> {
        if self.settings.mdblist_key.is_none() {
            return Vec::new();
        }
        let Some(ratings) = self.ratings.get(key) else {
            return Vec::new();
        };
        RATING_SITES
            .iter()
            .filter(|site| self.settings.rating_sources.iter().any(|s| s == site.id))
            .filter_map(|site| ratings.get(site.id).and_then(|r| rating_badge(site.id, r)))
            .collect()
    }

    fn toggle_rating_source(&mut self, id: &'static str, cx: &mut Context<Self>) {
        self.update_settings(
            |s| {
                if let Some(at) = s.rating_sources.iter().position(|x| x == id) {
                    s.rating_sources.remove(at);
                } else {
                    s.rating_sources.push(id.to_string());
                }
            },
            cx,
        );
    }

    /// Settings → Ratings: the MDBList key, then a switch for each site.
    pub(super) fn ratings_rows(&self, window: &Window, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let description = if self.settings.mdblist_key.is_some() {
            "Showing ratings from the sites below on movie and show pages."
        } else {
            "Add an MDBList key to show ratings from IMDb, Rotten Tomatoes, Metacritic, \
             Letterboxd and more on movie and show pages. Keys are free at mdblist.com."
        };
        let mut rows = vec![
            setting_row(
                "MDBList key",
                Some(description),
                Some(self.key_control(about::KeyKind::Mdblist, window, cx)),
            )
            .into_any_element(),
        ];
        rows.extend(RATING_SITES.iter().enumerate().map(|(ix, site)| {
            let on = self.settings.rating_sources.iter().any(|s| s == site.id);
            let id = site.id;
            setting_row(
                site.name,
                None,
                Some(
                    div()
                        .id(("rating-source", ix))
                        .child(toggle(
                            "rating-source-toggle",
                            on,
                            cx.listener(move |this, _, _, cx| this.toggle_rating_source(id, cx)),
                        ))
                        .into_any_element(),
                ),
            )
            .into_any_element()
        }));
        rows
    }
}

/// One site's badge: its mark (or name plate) and the rating on its own scale.
fn rating_badge(site: &str, r: &SourceRating) -> Option<AnyElement> {
    // The rating on the site's scale, falling back to its score out of 100.
    let score = r.score.map(f64::from);
    let out_of = |scale: f64| {
        r.value
            .filter(|v| *v <= scale)
            .or(score.map(|s| s * scale / 100.))
    };
    let percent = || {
        score
            .or(r.value.filter(|v| *v <= 100.))
            .map(|p| p.round() as u8)
    };
    let row = |mark: AnyElement, text: String| {
        div()
            .flex()
            .items_center()
            .gap_1p5()
            .h(px(26.))
            .text_sm()
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(theme::text())
            .child(mark)
            .child(text)
            .into_any_element()
    };
    let badge = match site {
        "imdb" => row(
            plate("IMDb", 0xf5c518, 0x000000),
            format!("{:.1}", out_of(10.)?),
        ),
        "tomatoes" => {
            let p = percent()?;
            let mark = if p >= 60 {
                "brand/rt-fresh.svg"
            } else {
                "brand/rt-rotten.svg"
            };
            row(brand_image(mark), format!("{p}%"))
        }
        "popcorn" => row(
            brand_image("brand/rt-popcorn.svg"),
            format!("{}%", percent()?),
        ),
        "metacritic" => {
            let s = percent()?;
            div()
                .flex()
                .items_center()
                .gap_1p5()
                .h(px(26.))
                .child(brand_image("brand/metacritic.svg"))
                .child(score_box(s.to_string(), metacritic_colour(f64::from(s))))
                .into_any_element()
        }
        "metacriticuser" => {
            let v = out_of(10.)?;
            div()
                .flex()
                .items_center()
                .gap_1p5()
                .h(px(26.))
                .child(brand_image("brand/metacritic.svg"))
                .child(score_box(format!("{v:.1}"), metacritic_colour(v * 10.)))
                .child(
                    div()
                        .text_xs()
                        .text_color(theme::text_tertiary())
                        .child("users"),
                )
                .into_any_element()
        }
        "letterboxd" => row(
            brand_image("brand/letterboxd.svg"),
            format!("{:.1}", out_of(5.)?),
        ),
        "trakt" => row(brand_image("brand/trakt.svg"), format!("{}%", percent()?)),
        "tmdb" => row(
            plate("TMDB", 0x0d253f, 0x01b4e4),
            format!("{}%", percent()?),
        ),
        "rogerebert" => row(
            div()
                .text_color(gpui::rgb(0xf5c518))
                .child("★")
                .into_any_element(),
            format!("{:.1} Ebert", out_of(4.)?),
        ),
        "myanimelist" => row(
            plate("MAL", 0x2e51a2, 0xffffff),
            format!("{:.2}", out_of(10.)?),
        ),
        _ => return None,
    };
    Some(badge)
}

/// A small coloured name plate (IMDb's yellow, TMDB's navy…).
fn plate(text: &'static str, bg: u32, ink: u32) -> AnyElement {
    div()
        .h(px(19.))
        .px(px(6.))
        .flex()
        .items_center()
        .rounded(px(3.))
        .bg(gpui::rgb(bg))
        .text_color(gpui::rgb(ink))
        .text_size(px(13.))
        .font_weight(FontWeight::BLACK)
        .child(text)
        .into_any_element()
}

/// Metacritic's coloured score box.
fn score_box(text: String, colour: u32) -> AnyElement {
    div()
        .h(px(19.))
        .min_w(px(24.))
        .px(px(3.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(3.))
        .bg(gpui::rgb(colour))
        .text_color(gpui::rgb(0x000000))
        .text_sm()
        .font_weight(FontWeight::BOLD)
        .child(text)
        .into_any_element()
}

/// Metacritic's colours for a score out of 100: green from 61, yellow 40–60, red below.
fn metacritic_colour(score: f64) -> u32 {
    if score >= 61. {
        0x66cc33
    } else if score >= 40. {
        0xffcc33
    } else {
        0xff0000
    }
}

/// A review site's mark, drawn in its own colours. Built-in images go through GPUI's own
/// image cache: the library's cache only loads artwork files from disk.
fn brand_image(path: &'static str) -> AnyElement {
    div()
        .flex_shrink_0()
        .image_cache(gpui::retain_all(path))
        .child(gpui::img(path).size(px(20.)))
        .into_any_element()
}

fn unix_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}
