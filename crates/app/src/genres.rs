//! Genre filters for the Movies and TV Shows grids.

use std::collections::HashMap;
use std::rc::Rc;

use miru_core::library::LibraryItem;

/// Filter state for one grid: the chosen genre, whether only unwatched titles show, the
/// genres on offer and the items shown.
#[derive(Default)]
pub struct GenreFilter {
    pub selected: Option<String>,
    pub unwatched_only: bool,
    /// Genres present in the grid, most common first, with how many titles carry each.
    pub genres: Vec<(String, usize)>,
    /// The grid's items narrowed by the genre and the unwatched switch.
    pub items: Rc<Vec<LibraryItem>>,
}

impl GenreFilter {
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
        self.items = if self.selected.is_none() && !self.unwatched_only {
            all.clone()
        } else {
            Rc::new(
                all.iter()
                    .filter(|item| self.selected.as_deref().is_none_or(|g| has_genre(item, g)))
                    .filter(|item| !self.unwatched_only || !watched(item))
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
    fn unwatched_switch_hides_watched_titles() {
        let all = Rc::new(vec![item("A", &["Drama"]), item("B", &["Drama"])]);
        let mut filter = GenreFilter {
            unwatched_only: true,
            ..Default::default()
        };
        filter.update(&all, |i| i.key.title == "A");
        let titles: Vec<_> = filter.items.iter().map(|i| i.key.title.as_str()).collect();
        assert_eq!(titles, ["B"]);
    }
}
