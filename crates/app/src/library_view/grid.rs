//! Movies and TV Shows grids with their sort and genre controls.

use super::*;
use crate::genres::{Length, MIN_RATINGS, WatchedFilter};
use miru_core::ratings::{RATING_SITES, RatingSite};

impl LibraryView {
    // ---- grids -----------------------------------------------------------------------------

    pub(super) fn render_grid(
        &self,
        tab: Tab,
        heading: &'static str,
        items: Rc<Vec<LibraryItem>>,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let count = items.len();
        let title_row = div()
            .flex()
            .items_center()
            .gap_3()
            .child(
                div()
                    .font_family(theme::FONT_DISPLAY)
                    .text_size(px(36.))
                    .font_weight(FontWeight::BOLD)
                    .child(heading),
            )
            .child(
                div()
                    .text_color(theme::text_tertiary())
                    .child(format!("{count} titles")),
            )
            .child(div().flex_1())
            .child(self.watched_dropdown(tab, cx))
            .child(self.genre_dropdown(tab, cx))
            .child(self.rating_dropdown(tab, cx))
            .child(self.length_dropdown(tab, cx))
            .child(self.sort_control(tab, cx));
        let header = div()
            .flex_shrink_0()
            .pt(px(NAV_HEIGHT + 24.))
            .pb_2()
            .px(px(PAGE_PADDING))
            .child(title_row);

        if items.is_empty() {
            return div()
                .size_full()
                .flex()
                .flex_col()
                .child(header)
                .child(
                    div()
                        .flex_1()
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_color(theme::text_secondary())
                        .child(
                            if self.genre_filter(tab).is_some_and(GenreFilter::is_narrowed) {
                                "Nothing matches these filters."
                            } else {
                                "Nothing here yet."
                            },
                        ),
                )
                .into_any_element();
        }
        let width =
            f32::from(window.viewport_size().width) - self.sidebar_width() - PAGE_PADDING * 2.;
        let cols = (((width + SHELF_GAP) / (MIN_GRID_TILE + SHELF_GAP)).floor() as usize).max(2);
        self.grid_cols.set(cols);
        let tile = (width - SHELF_GAP * (cols as f32 - 1.)) / cols as f32;
        let rows = count.div_ceil(cols);

        let list = uniform_list(
            "grid",
            rows,
            cx.processor(move |this, range: std::ops::Range<usize>, _window, cx| {
                range
                    .map(|row| {
                        let start = row * cols;
                        let end = (start + cols).min(items.len());
                        div()
                            .flex()
                            .gap(px(SHELF_GAP))
                            .px(px(PAGE_PADDING))
                            .pt_3()
                            .pb_5()
                            .children(
                                items[start..end]
                                    .iter()
                                    .map(|item| this.poster_card("grid", item, tile, cx)),
                            )
                    })
                    .collect()
            }),
        )
        .track_scroll(&self.grid_scroll)
        .flex_1();

        div()
            .size_full()
            .flex()
            .flex_col()
            .child(header)
            .child(list)
            .into_any_element()
    }

    pub(super) fn genre_filter(&self, tab: Tab) -> Option<&GenreFilter> {
        match tab {
            Tab::Movies => Some(&self.movie_filter),
            Tab::Shows => Some(&self.show_filter),
            Tab::Home | Tab::Search => None,
        }
    }

    /// A movie whose file is watched, or a show with every episode watched.
    pub(super) fn is_watched(&self, item: &LibraryItem) -> bool {
        match item.key.kind {
            ItemKind::Movie => self.watched_movies.contains(&item.file_id),
            ItemKind::Show => self.watched_shows.contains(&item.key.title.to_lowercase()),
        }
    }

    /// Recomputes both grids' visible items from the full lists.
    pub(super) fn apply_filters(&mut self) {
        let (movies, shows) = (&self.watched_movies, &self.watched_shows);
        self.movie_filter
            .update(&self.movies, |i| movies.contains(&i.file_id));
        self.show_filter
            .update(&self.shows, |i| shows.contains(&i.key.title.to_lowercase()));
    }

    fn set_watched_filter(&mut self, tab: Tab, value: WatchedFilter, cx: &mut Context<Self>) {
        match tab {
            Tab::Movies => self.movie_filter.watched = value,
            Tab::Shows => self.show_filter.watched = value,
            Tab::Home | Tab::Search => return,
        }
        self.apply_filters();
        self.close_genre_menu();
        self.grid_scroll
            .scroll_to_item(0, gpui::ScrollStrategy::Top);
        cx.notify();
    }

    pub(super) fn select_genre(&mut self, tab: Tab, genre: Option<String>, cx: &mut Context<Self>) {
        match tab {
            Tab::Movies => self.movie_filter.selected = genre,
            Tab::Shows => self.show_filter.selected = genre,
            Tab::Home | Tab::Search => return,
        }
        self.apply_filters();
        self.close_genre_menu();
        self.grid_scroll
            .scroll_to_item(0, gpui::ScrollStrategy::Top);
        cx.notify();
    }

    fn set_sort(&mut self, tab: Tab, sort: LibrarySort, cx: &mut Context<Self>) {
        self.close_genre_menu();
        self.update_settings(
            |s| match tab {
                Tab::Movies => s.movie_sort = sort,
                _ => s.show_sort = sort,
            },
            cx,
        );
        self.reload();
        self.fill_sort_ratings(cx);
    }

    pub(super) fn close_genre_menu(&mut self) {
        if self.genre_menu.take().is_some() {
            self.genre_menu_closed = Instant::now();
        }
    }

    /// "All titles" / "Unwatched" / "Watched": a dropdown, white while it filters.
    fn watched_dropdown(&self, tab: Tab, cx: &mut Context<Self>) -> AnyElement {
        let Some(filter) = self.genre_filter(tab) else {
            return theme::nothing();
        };
        let current = filter.watched;
        let rows = WatchedFilter::ALL
            .into_iter()
            .map(|value| GridOption {
                label: value.label().into(),
                count: None,
                heading: None,
                selected: value == current,
                action: Rc::new(move |this: &mut Self, cx: &mut Context<Self>| {
                    this.set_watched_filter(tab, value, cx)
                }),
            })
            .collect();
        let active = current != WatchedFilter::Any;
        let label = current.label().into();
        self.grid_dropdown(tab, GridMenu::Watched, label, active, rows, 180., cx)
    }

    /// Genre picker: a dropdown naming the current genre, listing "All genres" and each genre
    /// with its title count, most common first.
    pub(super) fn genre_dropdown(&self, tab: Tab, cx: &mut Context<Self>) -> AnyElement {
        let Some(filter) = self.genre_filter(tab) else {
            return theme::nothing();
        };
        if filter.genres.is_empty() {
            return theme::nothing();
        }
        let mut rows = vec![GridOption {
            label: "All genres".into(),
            count: None,
            heading: None,
            selected: filter.selected.is_none(),
            action: Rc::new(move |this: &mut Self, cx: &mut Context<Self>| {
                this.select_genre(tab, None, cx)
            }),
        }];
        rows.extend(filter.genres.iter().map(|(name, n)| {
            let genre = name.clone();
            GridOption {
                label: name.clone().into(),
                count: Some(n.to_string().into()),
                heading: None,
                selected: filter.selected.as_deref() == Some(name.as_str()),
                action: Rc::new(move |this: &mut Self, cx: &mut Context<Self>| {
                    this.select_genre(tab, Some(genre.clone()), cx)
                }),
            }
        }));
        let label: SharedString = filter
            .selected
            .clone()
            .unwrap_or_else(|| "All genres".into())
            .into();
        let active = filter.selected.is_some();
        self.grid_dropdown(tab, GridMenu::Genre, label, active, rows, 248., cx)
    }

    fn filter_mut(&mut self, tab: Tab) -> Option<&mut GenreFilter> {
        match tab {
            Tab::Movies => Some(&mut self.movie_filter),
            Tab::Shows => Some(&mut self.show_filter),
            Tab::Home | Tab::Search => None,
        }
    }

    /// Applies a change to a grid's filters and shows the grid from the top.
    fn change_filter(
        &mut self,
        tab: Tab,
        change: impl FnOnce(&mut GenreFilter),
        cx: &mut Context<Self>,
    ) {
        let Some(filter) = self.filter_mut(tab) else {
            return;
        };
        change(filter);
        self.apply_filters();
        self.close_genre_menu();
        self.grid_scroll
            .scroll_to_item(0, gpui::ScrollStrategy::Top);
        cx.notify();
    }

    /// Minimum TMDB rating (Any, 6+, 7+ or 8+), then, with an MDBList key, a sort by each
    /// rating site chosen in Settings.
    fn rating_dropdown(&self, tab: Tab, cx: &mut Context<Self>) -> AnyElement {
        let Some(filter) = self.genre_filter(tab) else {
            return theme::nothing();
        };
        let current = filter.min_rating;
        let label = |min: Option<u16>| match min {
            Some(min) => format!("Rated {}+", min / 10),
            None => "Any rating".to_string(),
        };
        let sort = match tab {
            Tab::Movies => self.settings.movie_sort,
            _ => self.settings.show_sort,
        };
        let sites: Vec<&'static RatingSite> = if self.settings.mdblist_key.is_some() {
            RATING_SITES
                .iter()
                .filter(|site| self.settings.rating_sources.iter().any(|s| s == site.id))
                .collect()
        } else {
            Vec::new()
        };
        let mut rows: Vec<GridOption> = std::iter::once(None)
            .chain(MIN_RATINGS.into_iter().map(Some))
            .enumerate()
            .map(|(ix, min)| GridOption {
                label: label(min).into(),
                count: None,
                heading: (ix == 0 && !sites.is_empty()).then(|| "Minimum rating".into()),
                selected: min == current,
                action: Rc::new(move |this: &mut Self, cx: &mut Context<Self>| {
                    this.change_filter(tab, |f| f.min_rating = min, cx)
                }),
            })
            .collect();
        rows.extend(sites.iter().enumerate().map(|(ix, site)| {
            let site_sort = LibrarySort::Site(site.id);
            GridOption {
                label: site.name.into(),
                count: None,
                heading: (ix == 0).then(|| "Sort by rating".into()),
                selected: sort == site_sort,
                action: Rc::new(move |this: &mut Self, cx: &mut Context<Self>| {
                    this.set_sort(tab, site_sort, cx)
                }),
            }
        }));
        let width = if sites.is_empty() { 180. } else { 240. };
        self.grid_dropdown(
            tab,
            GridMenu::Rating,
            label(current).into(),
            current.is_some(),
            rows,
            width,
            cx,
        )
    }

    /// Runtime ranges, for movies (a show's length varies by episode).
    fn length_dropdown(&self, tab: Tab, cx: &mut Context<Self>) -> AnyElement {
        if tab != Tab::Movies {
            return theme::nothing();
        }
        let Some(filter) = self.genre_filter(tab) else {
            return theme::nothing();
        };
        let current = filter.length;
        let rows = Length::ALL
            .into_iter()
            .map(|length| GridOption {
                label: length.label().into(),
                count: None,
                heading: None,
                selected: length == current,
                action: Rc::new(move |this: &mut Self, cx: &mut Context<Self>| {
                    this.change_filter(tab, |f| f.length = length, cx)
                }),
            })
            .collect();
        self.grid_dropdown(
            tab,
            GridMenu::Length,
            current.label().into(),
            current != Length::Any,
            rows,
            200.,
            cx,
        )
    }

    /// The season picker on a show page: "Season 3 ▾", listing each season with its episode
    /// count and how many are left to watch. `seasons` is (season, episodes, unwatched).
    pub(super) fn season_dropdown(
        &self,
        seasons: &[(u16, usize, usize)],
        current: u16,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let rows = seasons
            .iter()
            .map(|&(season, episodes, left)| {
                let count = format!("{episodes} episode{}", if episodes == 1 { "" } else { "s" });
                let detail = match left {
                    0 => format!("{count} · Watched"),
                    left if left == episodes => count,
                    left => format!("{count} · {left} left"),
                };
                GridOption {
                    label: season_name(season).into(),
                    count: Some(detail.into()),
                    heading: None,
                    selected: season == current,
                    action: Rc::new(move |this: &mut Self, cx: &mut Context<Self>| {
                        if let Page::Show { season: shown, .. } = &mut this.page {
                            *shown = Some(season);
                        }
                        this.close_genre_menu();
                        cx.notify();
                    }),
                }
            })
            .collect();
        self.grid_dropdown(
            Tab::Shows,
            GridMenu::Season,
            season_name(current).into(),
            false,
            rows,
            280.,
            cx,
        )
    }

    /// Sort order: a dropdown reading "Sort: Title".
    pub(super) fn sort_control(&self, tab: Tab, cx: &mut Context<Self>) -> AnyElement {
        let (current, options): (LibrarySort, Vec<LibrarySort>) = match tab {
            Tab::Movies => (self.settings.movie_sort, LibrarySort::ALL.to_vec()),
            // Shows sort by year too, using the first-aired year from TMDB.
            Tab::Shows => (self.settings.show_sort, LibrarySort::ALL.to_vec()),
            Tab::Home | Tab::Search => return theme::nothing(),
        };
        let rows = options
            .into_iter()
            .map(|sort| GridOption {
                label: sort.label().into(),
                count: None,
                heading: None,
                selected: sort == current,
                action: Rc::new(move |this: &mut Self, cx: &mut Context<Self>| {
                    this.set_sort(tab, sort, cx)
                }),
            })
            .collect();
        let label = format!("Sort: {}", current.label()).into();
        self.grid_dropdown(tab, GridMenu::Sort, label, false, rows, 200., cx)
    }

    /// A pill that opens a floating list of options, with a check beside the chosen one. White
    /// while `active` (a filter is narrowing the grid).
    #[allow(clippy::too_many_arguments)]
    fn grid_dropdown(
        &self,
        tab: Tab,
        kind: GridMenu,
        label: SharedString,
        active: bool,
        options: Vec<GridOption>,
        width: f32,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let open = self.genre_menu == Some((tab, kind));
        // The chosen option's position among the list's children (headings are children too).
        let selected_child = options.iter().position(|o| o.selected).map(|ix| {
            ix + options[..ix].iter().filter(|o| o.heading.is_some()).count()
                + usize::from(options[ix].heading.is_some())
        });
        // The season picker sits at the left of the page, so its menu opens rightwards.
        let open_right = kind == GridMenu::Season;
        let trigger = div()
            .id(("grid-dropdown", kind as usize))
            .tab_index(0)
            .focus_visible(theme::focus_ring)
            .h(px(40.))
            .pl_4()
            .pr_3()
            .flex()
            .items_center()
            .gap_2()
            .rounded(px(20.))
            .border_1()
            .text_sm()
            .font_weight(FontWeight::MEDIUM)
            .whitespace_nowrap()
            .cursor_pointer()
            .when(active, |d| {
                d.bg(theme::text())
                    .border_color(theme::text())
                    .text_color(theme::bg())
            })
            .when(!active, |d| {
                d.bg(if open {
                    theme::glass_hover()
                } else {
                    theme::glass()
                })
                .border_color(theme::glass_border())
                .text_color(theme::text())
                .hover(|s| s.bg(theme::glass_hover()))
            })
            .child(label)
            .child(icon(
                "icons/chevron-down.svg",
                14.,
                if active {
                    theme::bg()
                } else {
                    theme::text_secondary()
                },
            ))
            .on_click(cx.listener(move |this, _, _, cx| {
                if this.genre_menu == Some((tab, kind)) {
                    this.close_genre_menu();
                } else if this.genre_menu_closed.elapsed() > Duration::from_millis(200) {
                    this.genre_menu = Some((tab, kind));
                    // Open at the top, or on the chosen option in a long list.
                    this.grid_menu_scroll.reset();
                    if let Some(ix) = selected_child {
                        this.grid_menu_scroll.handle.scroll_to_item(ix);
                    }
                }
                cx.notify();
            }));

        let menu = open.then(|| {
            let rows = options.into_iter().enumerate().flat_map(|(ix, option)| {
                let action = option.action.clone();
                let row = div()
                    .id(("grid-option", ix))
                    .h(px(theme::MENU_ROW_H))
                    .px_3()
                    .flex()
                    .flex_shrink_0()
                    .items_center()
                    .gap_3()
                    .rounded(px(8.))
                    .text_sm()
                    .cursor_pointer()
                    .text_color(if option.selected {
                        theme::text()
                    } else {
                        theme::text_secondary()
                    })
                    .hover(|s| s.bg(theme::menu_hover()).text_color(theme::text()))
                    .child(
                        div().w(px(16.)).flex().children(
                            option
                                .selected
                                .then(|| icon("icons/check.svg", 15., theme::text())),
                        ),
                    )
                    .child(div().flex_1().child(option.label))
                    .children(option.count.map(|text| {
                        div()
                            .flex_shrink_0()
                            .text_xs()
                            .text_color(theme::text_tertiary())
                            .child(text)
                    }))
                    .on_click(cx.listener(move |this, _, _, cx| action(this, cx)));
                let heading = option.heading.map(|heading| {
                    div()
                        .px_3()
                        .pt(px(if ix == 0 { 6. } else { 14. }))
                        .pb_1()
                        .flex_shrink_0()
                        .text_xs()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(theme::text_tertiary())
                        .child(heading)
                });
                heading
                    .into_iter()
                    .map(IntoElement::into_any_element)
                    .chain(std::iter::once(row.into_any_element()))
            });
            let list = theme::menu_surface(div().id("grid-menu"))
                .occlude()
                .mt_2()
                .w(px(width))
                .max_h(px(420.))
                .overflow_y_scroll()
                .track_scroll(&self.grid_menu_scroll.handle)
                .flex()
                .flex_col()
                .children(rows)
                .child(self.grid_menu_scroll.scrollbar(cx.entity_id()))
                .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                    this.close_genre_menu();
                    cx.notify();
                }))
                .with_animation("grid-menu-open", theme::menu_open_animation(), |el, t| {
                    el.opacity(t).mt(px(8. - 6. * (1. - t)))
                });
            let (side, anchor) = if open_right {
                (div().absolute().top_full().left_0(), gpui::Anchor::TopLeft)
            } else {
                (
                    div().absolute().top_full().right_0(),
                    gpui::Anchor::TopRight,
                )
            };
            side.child(
                gpui::deferred(
                    gpui::anchored()
                        .anchor(anchor)
                        .snap_to_window_with_margin(px(12.))
                        .child(list),
                )
                .with_priority(1),
            )
        });

        div()
            .relative()
            .flex_shrink_0()
            .child(trigger)
            .children(menu)
            .into_any_element()
    }
}

/// "Season 3", or "Specials" for season 0.
pub(super) fn season_name(season: u16) -> String {
    if season == 0 {
        "Specials".into()
    } else {
        format!("Season {season}")
    }
}

/// Which grid dropdown is open.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum GridMenu {
    Watched,
    Genre,
    Rating,
    Length,
    Sort,
    /// The season picker on a show page.
    Season,
}

type GridAction = Rc<dyn Fn(&mut LibraryView, &mut Context<LibraryView>)>;

struct GridOption {
    label: SharedString,
    /// Quiet text at the right: a count, or a short detail.
    count: Option<SharedString>,
    /// A section title drawn above this option.
    heading: Option<SharedString>,
    selected: bool,
    action: GridAction,
}

impl LibraryView {}

/// Orders a library grid. Title order ignores leading articles ("The Matrix" under M).
pub(super) fn sort_items(
    items: &mut [LibraryItem],
    sort: LibrarySort,
    ratings: &HashMap<TitleKey, miru_core::ratings::TitleRatings>,
) {
    match sort {
        LibrarySort::Title => items.sort_by_cached_key(|i| title_sort_key(i.display_title())),
        LibrarySort::RecentlyAdded => items.sort_by_key(|i| std::cmp::Reverse(i.added_at)),
        // Undated titles go last in both year orders.
        LibrarySort::Newest => items.sort_by_cached_key(|i| {
            let year = item_year(i);
            (
                year.is_none(),
                std::cmp::Reverse(year),
                title_sort_key(i.display_title()),
            )
        }),
        LibrarySort::Oldest => items.sort_by_cached_key(|i| {
            let year = item_year(i);
            (year.is_none(), year, title_sort_key(i.display_title()))
        }),
        // Unrated titles go last.
        LibrarySort::TopRated => items.sort_by_cached_key(|i| {
            let rating = i.metadata.as_ref().and_then(|m| m.rating_tenths);
            (
                rating.is_none(),
                std::cmp::Reverse(rating),
                title_sort_key(i.display_title()),
            )
        }),
        // Titles the site hasn't rated (or whose ratings aren't fetched yet) go last.
        LibrarySort::Site(site) => items.sort_by_cached_key(|i| {
            let score = ratings
                .get(&i.key)
                .and_then(|r| r.get(site))
                .and_then(|r| r.score);
            (
                score.is_none(),
                std::cmp::Reverse(score),
                title_sort_key(i.display_title()),
            )
        }),
    }
}

/// A title's year: a movie's from its file name, else the release (or first-aired) year from
/// TMDB. Shows are grouped by name only, so theirs always comes from TMDB.
fn item_year(item: &LibraryItem) -> Option<u16> {
    item.key
        .year
        .or_else(|| item.metadata.as_ref().and_then(|m| m.release_year))
}
