//! Ratings from other sites on detail pages, from MDBList (with the user's own key): which
//! sites exist and how each one's badge looks, fetching and caching, and the Settings rows
//! for choosing which sites to show.

use miru_core::ratings::{RATINGS_MAX_AGE_SECS, SourceRating, TitleRatings};
use miru_metadata::Mdblist;

use super::*;

/// A site MDBList reports ratings from, in the order the badges appear.
pub(super) struct RatingSite {
    /// MDBList's name for it.
    pub id: &'static str,
    pub name: &'static str,
}

pub(super) const RATING_SITES: [RatingSite; 10] = [
    RatingSite {
        id: "imdb",
        name: "IMDb",
    },
    RatingSite {
        id: "tomatoes",
        name: "Rotten Tomatoes",
    },
    RatingSite {
        id: "popcorn",
        name: "Rotten Tomatoes audience",
    },
    RatingSite {
        id: "metacritic",
        name: "Metacritic",
    },
    RatingSite {
        id: "metacriticuser",
        name: "Metacritic users",
    },
    RatingSite {
        id: "letterboxd",
        name: "Letterboxd",
    },
    RatingSite {
        id: "trakt",
        name: "Trakt",
    },
    RatingSite {
        id: "tmdb",
        name: "TMDB",
    },
    RatingSite {
        id: "rogerebert",
        name: "Roger Ebert",
    },
    RatingSite {
        id: "myanimelist",
        name: "MyAnimeList",
    },
];

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
            .h(px(22.))
            .text_xs()
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
                .h(px(22.))
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
                .h(px(22.))
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
        .h(px(16.))
        .px(px(5.))
        .flex()
        .items_center()
        .rounded(px(3.))
        .bg(gpui::rgb(bg))
        .text_color(gpui::rgb(ink))
        .text_size(px(11.))
        .font_weight(FontWeight::BLACK)
        .child(text)
        .into_any_element()
}

/// Metacritic's coloured score box.
fn score_box(text: String, colour: u32) -> AnyElement {
    div()
        .h(px(16.))
        .min_w(px(20.))
        .px(px(3.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(3.))
        .bg(gpui::rgb(colour))
        .text_color(gpui::rgb(0x000000))
        .text_xs()
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
        .child(gpui::img(path).size(px(16.)))
        .into_any_element()
}

fn unix_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}
