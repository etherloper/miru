//! Movies and TV Shows grids with their sort and genre controls.

use super::*;

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
            .child(self.unwatched_toggle(tab, cx))
            .child(self.genre_dropdown(tab, cx))
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
                            if self
                                .genre_filter(tab)
                                .is_some_and(|f| f.selected.is_some() || f.unwatched_only)
                            {
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

    fn toggle_unwatched(&mut self, tab: Tab, cx: &mut Context<Self>) {
        match tab {
            Tab::Movies => self.movie_filter.unwatched_only ^= true,
            Tab::Shows => self.show_filter.unwatched_only ^= true,
            Tab::Home | Tab::Search => return,
        }
        self.apply_filters();
        self.grid_scroll
            .scroll_to_item(0, gpui::ScrollStrategy::Top);
        cx.notify();
    }

    /// "Unwatched" pill beside the genre dropdown; white while it is on.
    fn unwatched_toggle(&self, tab: Tab, cx: &mut Context<Self>) -> AnyElement {
        let Some(filter) = self.genre_filter(tab) else {
            return theme::nothing();
        };
        let on = filter.unwatched_only;
        div()
            .id("unwatched-toggle")
            .tab_index(0)
            .focus_visible(theme::focus_ring)
            .h(px(40.))
            .px_4()
            .flex()
            .flex_shrink_0()
            .items_center()
            .gap_2()
            .rounded(px(20.))
            .border_1()
            .text_sm()
            .font_weight(FontWeight::MEDIUM)
            .cursor_pointer()
            .when(on, |d| {
                d.bg(theme::text())
                    .border_color(theme::text())
                    .text_color(theme::bg())
            })
            .when(!on, |d| {
                d.bg(theme::glass())
                    .border_color(theme::glass_border())
                    .text_color(theme::text())
                    .hover(|s| s.bg(theme::glass_hover()))
            })
            .child("Unwatched")
            .on_click(cx.listener(move |this, _, _, cx| this.toggle_unwatched(tab, cx)))
            .into_any_element()
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

    pub(super) fn close_genre_menu(&mut self) {
        if self.genre_menu.take().is_some() {
            self.genre_menu_closed = Instant::now();
        }
    }

    /// Genre picker beside the sort control: a pill naming the current genre that opens a
    /// floating list ("All genres" plus each genre with its title count, most common first).
    pub(super) fn genre_dropdown(&self, tab: Tab, cx: &mut Context<Self>) -> AnyElement {
        let Some(filter) = self.genre_filter(tab) else {
            return theme::nothing();
        };
        if filter.genres.is_empty() {
            return theme::nothing();
        }
        let open = self.genre_menu == Some(tab);
        let active = filter.selected.is_some();
        let label: SharedString = filter
            .selected
            .clone()
            .unwrap_or_else(|| "All genres".into())
            .into();

        let trigger = div()
            .id("genre-dropdown")
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
                if this.genre_menu == Some(tab) {
                    this.close_genre_menu();
                } else if this.genre_menu_closed.elapsed() > Duration::from_millis(200) {
                    this.genre_menu = Some(tab);
                }
                cx.notify();
            }));

        let row = |ix: usize, name: Option<String>, count: Option<usize>, selected: bool| {
            let label = name.clone().unwrap_or_else(|| "All genres".into());
            div()
                .id(("genre-option", ix))
                .h(px(36.))
                .px_3()
                .flex()
                .flex_shrink_0()
                .items_center()
                .gap_3()
                .rounded(px(8.))
                .text_sm()
                .cursor_pointer()
                .text_color(if selected {
                    theme::text()
                } else {
                    theme::text_secondary()
                })
                .hover(|s| s.bg(rgba(0xffffff14)).text_color(theme::text()))
                .child(
                    div()
                        .w(px(14.))
                        .flex()
                        .children(selected.then(|| icon("icons/check.svg", 14., theme::accent()))),
                )
                .child(div().flex_1().child(label))
                .children(count.map(|n| {
                    div()
                        .text_xs()
                        .text_color(theme::text_tertiary())
                        .child(n.to_string())
                }))
                .on_click(
                    cx.listener(move |this, _, _, cx| this.select_genre(tab, name.clone(), cx)),
                )
        };

        let menu = open.then(|| {
            let mut rows = vec![row(0, None, None, filter.selected.is_none())];
            rows.extend(filter.genres.iter().enumerate().map(|(ix, (name, n))| {
                let selected = filter.selected.as_deref() == Some(name.as_str());
                row(ix + 1, Some(name.clone()), Some(*n), selected)
            }));
            let list = div()
                .id("genre-menu")
                .occlude()
                .mt_2()
                .w(px(248.))
                .max_h(px(420.))
                .overflow_y_scroll()
                .p_1p5()
                .flex()
                .flex_col()
                .rounded(px(14.))
                .bg(theme::panel())
                .border_1()
                .border_color(theme::glass_border())
                .shadow(theme::frost_shadow())
                .children(rows)
                .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                    this.close_genre_menu();
                    cx.notify();
                }));
            div().absolute().top_full().right_0().child(
                gpui::deferred(
                    gpui::anchored()
                        .anchor(gpui::Anchor::TopRight)
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

    pub(super) fn sort_control(&self, tab: Tab, cx: &mut Context<Self>) -> AnyElement {
        let (id, current, options): (&'static str, LibrarySort, Vec<LibrarySort>) = match tab {
            Tab::Movies => (
                "sort-movies",
                self.settings.movie_sort,
                LibrarySort::ALL.to_vec(),
            ),
            // Shows are grouped by name only, so they have no year to sort by.
            Tab::Shows => (
                "sort-shows",
                self.settings.show_sort,
                vec![LibrarySort::Title, LibrarySort::RecentlyAdded],
            ),
            Tab::Home | Tab::Search => return theme::nothing(),
        };
        let labels = options
            .iter()
            .map(|o| (o.label().to_string(), *o == current))
            .collect();
        segmented(
            id,
            labels,
            cx.listener(move |this, ix: &usize, _, cx| {
                let sort = options[*ix];
                this.update_settings(
                    |s| match tab {
                        Tab::Movies => s.movie_sort = sort,
                        _ => s.show_sort = sort,
                    },
                    cx,
                );
                this.reload();
            }),
        )
        .into_any_element()
    }
}

/// Orders a library grid. Title order ignores leading articles ("The Matrix" under M).
pub(super) fn sort_items(items: &mut [LibraryItem], sort: LibrarySort) {
    match sort {
        LibrarySort::Title => items.sort_by_cached_key(|i| title_sort_key(i.display_title())),
        LibrarySort::RecentlyAdded => items.sort_by_key(|i| std::cmp::Reverse(i.added_at)),
        // Undated titles go last in both year orders.
        LibrarySort::Newest => items.sort_by_cached_key(|i| {
            (
                i.key.year.is_none(),
                std::cmp::Reverse(i.key.year),
                title_sort_key(i.display_title()),
            )
        }),
        LibrarySort::Oldest => items.sort_by_cached_key(|i| {
            (
                i.key.year.is_none(),
                i.key.year,
                title_sort_key(i.display_title()),
            )
        }),
    }
}
