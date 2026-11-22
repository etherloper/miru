//! Search: the floating Ctrl+K panel and the full results page.

use super::*;

impl LibraryView {
    /// Opens the floating search panel.
    pub(super) fn open_search(&mut self, cx: &mut Context<Self>) {
        // Always start from an empty field.
        self.search_draft.clear();
        self.run_search("");
        self.search_selected = 0;
        self.search_overlay = true;
        self.search_focus_pending = true;
        cx.notify();
    }

    pub(super) fn close_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.search_overlay = false;
        window.focus(&self.focus, cx);
        cx.notify();
    }

    /// Live results shown in the panel: the best title matches, then episode matches.
    pub(super) fn search_hits(&self) -> Vec<SearchHit> {
        self.search_titles
            .iter()
            .take(PANEL_TITLE_HITS)
            .cloned()
            .map(SearchHit::Title)
            .chain(
                self.search_episodes
                    .iter()
                    .take(PANEL_EPISODE_HITS)
                    .cloned()
                    .map(SearchHit::Episode),
            )
            .collect()
    }

    pub(super) fn open_hit(&mut self, hit: SearchHit, window: &mut Window, cx: &mut Context<Self>) {
        self.search_query = self.search_draft.trim().to_string();
        self.close_search(window, cx);
        match hit {
            SearchHit::Title(item) => self.open(&item, cx),
            // Episodes open their show, on the right season.
            SearchHit::Episode(file) => {
                let show = match &file.media {
                    ParsedMedia::Episode { show, .. } => show.clone(),
                    ParsedMedia::Movie { .. } => return,
                };
                if let Some(item) = self
                    .shows
                    .iter()
                    .find(|s| s.key.title.eq_ignore_ascii_case(&show))
                    .cloned()
                {
                    self.open(&item, cx);
                    if let Page::Show { season, .. } = &mut self.page {
                        *season = season_of(&file.media);
                    }
                }
            }
        }
    }

    /// Shows every match for the drafted query on the full results page.
    pub(super) fn submit_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let query = self.search_draft.trim().to_string();
        if query.is_empty() {
            return;
        }
        self.search_query = query.clone();
        self.run_search(&query);
        self.close_search(window, cx);
        self.navigate(Page::Browse(Tab::Search), cx);
    }

    /// Text editing and navigation for the search panel: typing, paste, Backspace (Ctrl deletes
    /// a word), Up/Down move through results, Enter opens the highlighted one, Esc closes.
    pub(super) fn on_search_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let k = &event.keystroke;
        let command = k.modifiers.control || k.modifiers.platform;
        let hits = self.search_hits();
        let before = self.search_draft.clone();
        match k.key.as_str() {
            "enter" => {
                match hits.into_iter().nth(self.search_selected) {
                    Some(hit) if !command => self.open_hit(hit, window, cx),
                    // Ctrl+Enter (or nothing highlighted) shows all results.
                    _ => self.submit_search(window, cx),
                }
                cx.stop_propagation();
                return;
            }
            "escape" => {
                self.close_search(window, cx);
                cx.stop_propagation();
                return;
            }
            "down" => {
                self.search_selected = (self.search_selected + 1).min(hits.len().saturating_sub(1))
            }
            "up" => self.search_selected = self.search_selected.saturating_sub(1),
            "backspace" if command => {
                let draft = &mut self.search_draft;
                let trimmed = draft.trim_end().len();
                let cut = draft[..trimmed].rfind(' ').map_or(0, |i| i + 1);
                draft.truncate(cut);
            }
            "backspace" => {
                self.search_draft.pop();
            }
            "v" if command => {
                if let Some(text) = cx.read_from_clipboard().and_then(|c| c.text()) {
                    let clean: String = text
                        .chars()
                        .map(|c| if c.is_control() { ' ' } else { c })
                        .collect();
                    self.search_draft.push_str(clean.trim());
                }
            }
            _ if command || k.modifiers.alt => {}
            _ => {
                if let Some(typed) = &k.key_char
                    && !typed.chars().any(char::is_control)
                {
                    self.search_draft.push_str(typed);
                }
            }
        }
        if self.search_draft != before {
            let draft = self.search_draft.clone();
            self.run_search(&draft);
            self.search_selected = 0;
        }
        cx.notify();
        cx.stop_propagation();
    }

    pub(super) fn run_search(&mut self, query: &str) {
        self.search_titles.clear();
        self.search_episodes.clear();
        self.search_art.clear();
        let query = query.trim().to_string();
        if query.is_empty() {
            return;
        }
        let result = (|| -> miru_core::db::Result<()> {
            for key in self.db.search_titles(&query, 40)? {
                let list = match key.kind {
                    ItemKind::Movie => &self.movies,
                    ItemKind::Show => &self.shows,
                };
                if let Some(item) = list.iter().find(|i| {
                    i.key.kind == key.kind
                        && i.key.year == key.year
                        && i.key.title.eq_ignore_ascii_case(&key.title)
                }) {
                    self.search_titles.push(item.clone());
                }
            }
            self.search_episodes = self.db.search_episodes(&query, 30)?;
            let shows: BTreeSet<String> = self
                .search_episodes
                .iter()
                .filter_map(|f| match &f.media {
                    ParsedMedia::Episode { show, .. } => Some(show.clone()),
                    ParsedMedia::Movie { .. } => None,
                })
                .collect();
            for show in shows {
                self.search_art.extend(self.db.episode_art(&show)?);
            }
            Ok(())
        })();
        if let Err(e) = result {
            tracing::warn!("search failed: {e}");
        }
    }

    /// Results page for the last submitted query.
    pub(super) fn render_search(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let query = self.search_query.trim().to_string();
        let heading = |text: String| {
            div()
                .font_family(theme::FONT_DISPLAY)
                .text_size(px(22.))
                .font_weight(FontWeight::SEMIBOLD)
                .child(text)
        };
        let count = self.search_titles.len() + self.search_episodes.len();

        // Same shape as the Movies and TV Shows headers: title, quiet count, one pill on the right.
        let new_search = div()
            .id("search-again")
            .tab_index(0)
            .focus_visible(theme::focus_ring)
            .h(px(40.))
            .pl_3()
            .pr_4()
            .flex()
            .flex_shrink_0()
            .items_center()
            .gap_2()
            .rounded(px(20.))
            .bg(theme::glass())
            .border_1()
            .border_color(theme::glass_border())
            .text_sm()
            .font_weight(FontWeight::MEDIUM)
            .cursor_pointer()
            .hover(|s| s.bg(theme::glass_hover()))
            .child(icon("icons/search.svg", 14., theme::text_secondary()))
            .child("New search")
            .on_click(cx.listener(|this, _, _, cx| this.open_search(cx)));
        let header = div()
            .flex()
            .items_center()
            .gap_3()
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .font_family(theme::FONT_DISPLAY)
                    .text_size(px(36.))
                    .font_weight(FontWeight::BOLD)
                    .child(format!("“{query}”")),
            )
            .child(
                div()
                    .flex_shrink_0()
                    .text_color(theme::text_tertiary())
                    .child(match count {
                        0 => "No matches".to_string(),
                        1 => "1 match".to_string(),
                        n => format!("{n} matches"),
                    }),
            )
            .child(div().flex_1())
            .child(new_search);

        let body: AnyElement = if count == 0 {
            div()
                .pt_10()
                .text_color(theme::text_secondary())
                .child("Try a different spelling, or search for an episode name.")
                .into_any_element()
        } else {
            let episodes = self
                .search_episodes
                .iter()
                .map(|file| {
                    let show = match &file.media {
                        ParsedMedia::Episode { show, .. } => show.clone(),
                        ParsedMedia::Movie { .. } => String::new(),
                    };
                    let show_item = self
                        .shows
                        .iter()
                        .find(|s| s.key.title.eq_ignore_ascii_case(&show));
                    let meta = show_item
                        .and_then(|s| s.metadata.clone())
                        .unwrap_or_default();
                    let show_title =
                        show_item.map_or(show.clone(), |s| s.display_title().to_string());
                    div()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child(
                            div()
                                .text_sm()
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(theme::text_tertiary())
                                .child(show_title.clone()),
                        )
                        .child(self.episode_card(
                            "search-episodes",
                            &show_title,
                            &meta,
                            file,
                            None,
                            self.search_art.get(&file.id),
                            cx,
                        ))
                })
                .collect::<Vec<_>>();
            let posters = self
                .search_titles
                .iter()
                .map(|item| self.poster_card("search", item, POSTER_W, cx))
                .collect::<Vec<_>>();
            div()
                .flex()
                .flex_col()
                .gap_10()
                .when(!posters.is_empty(), |d| {
                    d.child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .child(heading("Movies & TV Shows".into()))
                            .child(
                                div()
                                    .flex()
                                    .flex_wrap()
                                    .gap(px(SHELF_GAP))
                                    .pt_3()
                                    .children(posters),
                            ),
                    )
                })
                .when(!episodes.is_empty(), |d| {
                    d.child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .child(heading("Episodes".into()))
                            .child(
                                div()
                                    .flex()
                                    .flex_wrap()
                                    .gap(px(SHELF_GAP))
                                    .pt_3()
                                    .children(episodes),
                            ),
                    )
                })
                .into_any_element()
        };

        div()
            .id("search-page")
            .size_full()
            .overflow_y_scroll()
            .track_scroll(&self.detail_scroll)
            .child(
                div()
                    .pt(px(NAV_HEIGHT + 24.))
                    .px(px(PAGE_PADDING))
                    .pb_16()
                    .flex()
                    .flex_col()
                    .gap_8()
                    .child(header)
                    .child(body),
            )
    }

    /// Floating search panel (Ctrl+K). Compact while empty; matches appear beneath the field
    /// as you type.
    pub(super) fn render_search_overlay(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let focused = self.search_focus.is_focused(window);
        let draft = self.search_draft.clone();
        let empty = draft.trim().is_empty();
        let hits = self.search_hits();
        let caret = || {
            div()
                .flex_shrink_0()
                .w(px(2.))
                .h(px(20.))
                .rounded_full()
                .bg(theme::text())
                .with_animation(
                    "search-caret",
                    Animation::new(Duration::from_millis(1060)).repeat(),
                    |el, t| el.opacity(if t < 0.55 { 1. } else { 0. }),
                )
        };

        let field = div()
            .h(px(56.))
            .px_5()
            .flex()
            .items_center()
            .gap_3()
            .cursor_text()
            .child(icon("icons/search.svg", 18., theme::text_tertiary()))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .items_center()
                    .text_size(px(17.))
                    .when(draft.is_empty(), |d| {
                        d.when(focused, |d| d.child(caret())).child(
                            div()
                                .text_color(theme::text_tertiary())
                                .child("Search movies, shows and episodes"),
                        )
                    })
                    .when(!draft.is_empty(), |d| {
                        d.child(div().truncate().child(draft.clone()))
                            .when(focused, |d| d.child(caret()))
                    }),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(theme::text_tertiary())
                    .child("Esc"),
            );

        let results: Option<AnyElement> = (!empty).then(|| {
            if hits.is_empty() {
                return div()
                    .px_5()
                    .py_5()
                    .text_sm()
                    .text_color(theme::text_tertiary())
                    .child(format!("No matches for “{}”", draft.trim()))
                    .into_any_element();
            }
            let rows = hits
                .into_iter()
                .enumerate()
                .map(|(ix, hit)| self.search_hit_row(ix, hit, cx));
            div()
                .id("search-hits")
                .max_h(px(440.))
                .overflow_y_scroll()
                .p_1p5()
                .flex()
                .flex_col()
                .children(rows)
                .into_any_element()
        });

        // Plain-text hints rather than key boxes, so the panel reads as one quiet surface.
        let hint = |keys: &'static str, action: &'static str| {
            div()
                .flex()
                .items_center()
                .gap_1p5()
                .child(div().text_color(theme::text_secondary()).child(keys))
                .child(action)
        };
        let footer = div()
            .h(px(40.))
            .px_5()
            .border_t_1()
            .border_color(rgba(0xffffff0d))
            .flex()
            .items_center()
            .gap_5()
            .text_xs()
            .text_color(theme::text_tertiary())
            .child(hint("↑ ↓", "Navigate"))
            .child(hint("↵", "Open"))
            .child(hint("Ctrl ↵", "All results"));

        let panel = div()
            .id("search-panel")
            // Clicks inside the panel must not reach the scrim (which closes it).
            .occlude()
            // Keep keyboard focus in the panel while it is open.
            .on_action(|_: &FocusNext, _, _| {})
            .on_action(|_: &FocusPrev, _, _| {})
            .track_focus(&self.search_focus)
            .on_key_down(cx.listener(Self::on_search_key))
            .w(px(640.))
            .rounded(px(16.))
            .overflow_hidden()
            .bg(theme::panel())
            .border_1()
            .border_color(theme::glass_border())
            .shadow(theme::frost_shadow())
            .flex()
            .flex_col()
            .child(field)
            .children(results.map(|r| div().border_t_1().border_color(rgba(0xffffff0d)).child(r)))
            .child(footer);

        div()
            .id("search-overlay")
            .absolute()
            .inset_0()
            .bg(rgba(0x00000099))
            .flex()
            .justify_center()
            // Top-aligned: the panel keeps its natural height instead of stretching.
            .items_start()
            .pt(px(120.))
            .on_mouse_down(
                gpui::MouseButton::Left,
                cx.listener(|this, _, window, cx| this.close_search(window, cx)),
            )
            .child(panel)
            .with_animation(
                "search-overlay-in",
                Animation::new(theme::motion(160)).with_easing(ease_out_quint()),
                |el, t| el.opacity(t),
            )
    }

    pub(super) fn search_hit_row(
        &self,
        ix: usize,
        hit: SearchHit,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let selected = ix == self.search_selected;
        let (thumb, title, subtitle): (AnyElement, String, String) = match &hit {
            SearchHit::Title(item) => {
                let meta = item.metadata.clone().unwrap_or_default();
                let subtitle = match item.key.kind {
                    ItemKind::Movie => item
                        .key
                        .year
                        .map_or("Movie".to_string(), |y| format!("Movie · {y}")),
                    ItemKind::Show => format!("TV Show · {} episodes", item.file_count),
                };
                let thumb = div()
                    .w(px(32.))
                    .h(px(48.))
                    .flex_shrink_0()
                    .rounded(px(5.))
                    .overflow_hidden()
                    .bg(theme::placeholder(&item.key.title, meta.accent_rgb))
                    .child(art_layer(meta.poster_file.clone(), 6., ART_THUMB))
                    .relative()
                    .into_any_element();
                (thumb, item.display_title().to_string(), subtitle)
            }
            SearchHit::Episode(file) => {
                let show = match &file.media {
                    ParsedMedia::Episode { show, .. } => show.clone(),
                    ParsedMedia::Movie { .. } => String::new(),
                };
                let show_item = self
                    .shows
                    .iter()
                    .find(|s| s.key.title.eq_ignore_ascii_case(&show));
                let show_title = show_item.map_or(show.clone(), |s| s.display_title().to_string());
                let accent = show_item
                    .and_then(|s| s.metadata.as_ref())
                    .and_then(|m| m.accent_rgb);
                let still = self
                    .search_art
                    .get(&file.id)
                    .filter(|a| a.source != ArtSource::None)
                    .and_then(|a| a.still_file.clone());
                let name = self
                    .search_art
                    .get(&file.id)
                    .and_then(|a| a.name.clone())
                    .unwrap_or_else(|| episode_label(&file.media));
                let thumb = div()
                    .relative()
                    .w(px(64.))
                    .h(px(36.))
                    .flex_shrink_0()
                    .rounded(px(6.))
                    .overflow_hidden()
                    .bg(theme::placeholder(&show_title, accent))
                    .child(art_layer(still, 6., ART_THUMB))
                    .into_any_element();
                (
                    thumb,
                    name,
                    format!("{show_title} · {}", short_episode(&file.media)),
                )
            }
        };

        div()
            .id(("search-hit", ix))
            .flex()
            .items_center()
            .gap_3()
            .px_3()
            .py_1p5()
            .rounded(px(8.))
            .cursor_pointer()
            // Hovering moves the selection, so the selected fill doubles as the hover state.
            .when(selected, |d| d.bg(rgba(0xffffff14)))
            .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                if *hovered && this.search_selected != ix {
                    this.search_selected = ix;
                    cx.notify();
                }
            }))
            .on_click(
                cx.listener(move |this, _, window, cx| this.open_hit(hit.clone(), window, cx)),
            )
            .child(thumb)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap_0p5()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(if selected {
                                theme::text()
                            } else {
                                theme::text_secondary()
                            })
                            .truncate()
                            .child(title),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme::text_tertiary())
                            .truncate()
                            .child(subtitle),
                    ),
            )
            .when(selected, |d| {
                d.child(icon("icons/chevron-right.svg", 14., theme::text_tertiary()))
            })
            .into_any_element()
    }
}

/// A result in the search panel.
#[derive(Clone)]
// At most ten exist at a time, so boxing the larger variant would buy nothing.
#[allow(clippy::large_enum_variant)]
pub(super) enum SearchHit {
    Title(LibraryItem),
    Episode(MediaFile),
}
