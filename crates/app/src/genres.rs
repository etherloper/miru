//! Filters for the Movies and TV Shows grids: genre, watched state, rating and length.

use std::collections::HashMap;
use std::rc::Rc;

use miru_core::library::LibraryItem;

/// How long a movie runs, for the Length filter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Length {
    #[default]
    Any,
    Short,
    Medium,
    Long,
}

impl Length {
    pub const ALL: [Self; 4] = [Self::Any, Self::Short, Self::Medium, Self::Long];

    pub fn label(self) -> &'static str {
        match self {
            Self::Any => "Any length",
            Self::Short => "Under 90 min",
            Self::Medium => "90 min – 2 hours",
            Self::Long => "Over 2 hours",
        }
    }

    fn fits(self, minutes: Option<u32>) -> bool {
        match (self, minutes) {
            (Self::Any, _) => true,
            (_, None) => false,
            (Self::Short, Some(m)) => m < 90,
            (Self::Medium, Some(m)) => (90..=120).contains(&m),
            (Self::Long, Some(m)) => m > 120,
        }
    }
}

/// Which titles show by watched state.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WatchedFilter {
    #[default]
    Any,
    Unwatched,
    Watched,
}

impl WatchedFilter {
    pub const ALL: [Self; 3] = [Self::Any, Self::Unwatched, Self::Watched];

    pub fn label(self) -> &'static str {
        match self {
            Self::Any => "All titles",
            Self::Unwatched => "Unwatched",
            Self::Watched => "Watched",
        }
    }

    fn keeps(self, watched: bool) -> bool {
        match self {
            Self::Any => true,
            Self::Unwatched => !watched,
            Self::Watched => watched,
        }
    }
}

/// Minimum ratings offered, in tenths (70 = 7.0).
pub const MIN_RATINGS: [u16; 3] = [60, 70, 80];

/// Filter state for one grid: the chosen genre, which titles show by watched state, the
/// minimum rating and length, the genres on offer and the items shown.
#[derive(Default)]
pub struct GenreFilter {
    pub selected: Option<String>,
    pub watched: WatchedFilter,
    /// Minimum TMDB rating in tenths; unrated titles are hidden while it is set.
    pub min_rating: Option<u16>,
    /// Movies only; titles without a known runtime are hidden while it is set.
    pub length: Length,
    /// Genres present in the grid, most common first, with how many titles carry each.
    pub genres: Vec<(String, usize)>,
    /// The grid's items narrowed by the filters.
    pub items: Rc<Vec<LibraryItem>>,
}

impl GenreFilter {
    /// Whether any filter is hiding titles.
    pub fn is_narrowed(&self) -> bool {
        self.selected.is_some()
            || self.watched != WatchedFilter::Any
            || self.min_rating.is_some()
            || self.length != Length::Any
    }

    /// Recomputes the genres and filtered items from the full, sorted list. A selection whose
    /// genre has disappeared from the library is cleared.
    pub fn update(&mut self, all: &Rc<Vec<LibraryItem>>, watched: impl Fn(&LibraryItem) -> bool) {
        self.genres = genre_counts(all);
        if self
            .selected
            .as_ref()
            .is_some_and(|g| !self.genres.iter().any(|(name, _)| name == g))
        {
            self.selected = None;
        }
        self.items = if !self.is_narrowed() {
            all.clone()
        } else {
            Rc::new(
                all.iter()
                    .filter(|item| self.selected.as_deref().is_none_or(|g| has_genre(item, g)))
                    .filter(|item| self.watched.keeps(watched(item)))
                    .filter(|item| {
                        self.min_rating.is_none_or(|min| {
                            item.metadata
                                .as_ref()
                                .and_then(|m| m.rating_tenths)
                                .is_some_and(|r| r >= min)
                        })
                    })
                    .filter(|item| {
                        self.length
                            .fits(item.metadata.as_ref().and_then(|m| m.runtime_min))
                    })
                    .cloned()
                    .collect(),
            )
        };
    }
}

fn has_genre(item: &LibraryItem, genre: &str) -> bool {
    item.metadata
        .as_ref()
        .is_some_and(|m| m.genres.iter().any(|g| g == genre))
}

/// Genres across `items`, most common first, ties in alphabetical order.
pub fn genre_counts(items: &[LibraryItem]) -> Vec<(String, usize)> {
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for item in items {
        if let Some(meta) = &item.metadata {
            for genre in &meta.genres {
                *counts.entry(genre.as_str()).or_default() += 1;
            }
        }
    }
    let mut genres: Vec<(String, usize)> = counts
        .into_iter()
        .map(|(name, n)| (name.to_string(), n))
        .collect();
    genres.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    genres
}

#[cfg(test)]
mod tests {
    use super::*;
    use miru_core::library::{ItemKind, Metadata, TitleKey};

    fn item(title: &str, genres: &[&str]) -> LibraryItem {
        LibraryItem {
            key: TitleKey {
                kind: ItemKind::Movie,
                title: title.into(),
                year: None,
            },
            file_id: 0,
            file_count: 1,
            added_at: 0,
            metadata: Some(Metadata {
                genres: genres.iter().map(|g| g.to_string()).collect(),
                ..Default::default()
            }),
            metadata_version: 0,
        }
    }

    #[test]
    fn rating_and_length_narrow_the_grid() {
        let rated = |title: &str, rating: Option<u16>, minutes: Option<u32>| {
            let mut i = item(title, &[]);
            if let Some(m) = &mut i.metadata {
                m.rating_tenths = rating;
                m.runtime_min = minutes;
            }
            i
        };
        let all = Rc::new(vec![
            rated("Short and good", Some(82), Some(85)),
            rated("Long and fine", Some(68), Some(150)),
            rated("Unknown", None, None),
        ]);
        let titles = |f: &GenreFilter| -> Vec<String> {
            f.items.iter().map(|i| i.key.title.clone()).collect()
        };
        let mut filter = GenreFilter {
            min_rating: Some(70),
            ..Default::default()
        };
        filter.update(&all, |_| false);
        assert_eq!(titles(&filter), ["Short and good"]);

        filter.min_rating = None;
        filter.length = Length::Long;
        filter.update(&all, |_| false);
        assert_eq!(titles(&filter), ["Long and fine"]);
    }

    #[test]
    fn counts_are_ordered_by_frequency_then_name() {
        let items = [
            item("A", &["Drama", "Comedy"]),
            item("B", &["Drama"]),
            item("C", &["Action"]),
        ];
        assert_eq!(
            genre_counts(&items),
            vec![
                ("Drama".into(), 2),
                ("Action".into(), 1),
                ("Comedy".into(), 1)
            ]
        );
    }

    #[test]
    fn filter_keeps_order_and_clears_vanished_genres() {
        let all = Rc::new(vec![
            item("A", &["Drama"]),
            item("B", &["Comedy"]),
            item("C", &["Drama"]),
        ]);
        let mut filter = GenreFilter {
            selected: Some("Drama".into()),
            ..Default::default()
        };
        filter.update(&all, |_| false);
        let titles: Vec<_> = filter.items.iter().map(|i| i.key.title.as_str()).collect();
        assert_eq!(titles, ["A", "C"]);

        let fewer = Rc::new(vec![item("B", &["Comedy"])]);
        filter.update(&fewer, |_| false);
        assert_eq!(filter.selected, None);
        assert_eq!(filter.items.len(), 1);
    }

    #[test]
    fn watched_filter_picks_by_watched_state() {
        let all = Rc::new(vec![item("A", &["Drama"]), item("B", &["Drama"])]);
        let mut filter = GenreFilter {
            watched: WatchedFilter::Unwatched,
            ..Default::default()
        };
        filter.update(&all, |i| i.key.title == "A");
        let titles: Vec<_> = filter.items.iter().map(|i| i.key.title.as_str()).collect();
        assert_eq!(titles, ["B"]);

        filter.watched = WatchedFilter::Watched;
        filter.update(&all, |i| i.key.title == "A");
        let titles: Vec<_> = filter.items.iter().map(|i| i.key.title.as_str()).collect();
        assert_eq!(titles, ["A"]);
    }
}
