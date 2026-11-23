//! Fix match: search TMDB by hand and pick the right entry for a wrongly matched title.

use super::*;
use crate::library_sync::MatchCandidate;
use crate::text_edit::{self, KeyOutcome, TextEdit};

/// Rows of overview text shown per candidate.
const FIX_OVERVIEW_LINE_H: f32 = 18.;

pub(super) struct FixMatch {
    item: LibraryItem,
    query: TextEdit,
    /// The query the current results are for; Enter searches again when the text differs.
    searched: String,
    kind: ItemKind,
    results: Vec<MatchCandidate>,
    status: FixStatus,
    selected: usize,
    /// Bumped per search so a slow reply to an older search is dropped.
    request: usize,
}

#[derive(Clone, PartialEq)]
enum FixStatus {
    Idle,
    Searching,
    /// Saving the chosen TMDB id and fetching its artwork.
    Applying(i64),
    Failed(SharedString),
}

impl LibraryView {
    pub(super) fn open_fix_match(&mut self, item: LibraryItem, cx: &mut Context<Self>) {
        // Prefilled, with the caret at the end.
        let query = TextEdit::new(item.display_title());
        let kind = item.key.kind;
        self.fix_match = Some(FixMatch {
            item,
            query,
            searched: String::new(),
            kind,
            results: Vec::new(),
            status: FixStatus::Idle,
            selected: 0,
            request: 0,
        });
        self.fix_focus_pending = true;
        self.run_fix_search(cx);
    }

    pub(super) fn close_fix_match(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.fix_match = None;
        window.focus(&self.focus, cx);
        cx.notify();
    }

    fn run_fix_search(&mut self, cx: &mut Context<Self>) {
        let Some(fix) = &mut self.fix_match else {
            return;
        };
        let Some(tmdb) = self.tmdb.clone() else {
            fix.status = FixStatus::Failed(
                "Fix match needs a TMDB key. Add TMDB_API_KEY to your .env file.".into(),
            );
            cx.notify();
            return;
        };
        let query = fix.query.text().trim().to_string();
        if query.is_empty() {
            return;
        }
        fix.request += 1;
        fix.searched = fix.query.text().to_string();
        fix.status = FixStatus::Searching;
        let (request, kind, paths) = (fix.request, fix.kind, self.paths.clone());
        cx.notify();
        cx.spawn(async move |this, cx| {
            let found = cx
                .background_executor()
                .spawn(async move { library_sync::match_candidates(&paths, &tmdb, kind, &query) })
                .await;
            this.update(cx, |view, cx| {
                let Some(fix) = view.fix_match.as_mut().filter(|f| f.request == request) else {
                    return;
                };
                match found {
                    Ok(results) => {
                        fix.results = results;
                        fix.selected = 0;
                        fix.status = FixStatus::Idle;
                    }
                    Err(e) => {
                        fix.status = FixStatus::Failed(format!("Search failed: {e:#}").into())
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn set_fix_kind(&mut self, kind: ItemKind, cx: &mut Context<Self>) {
        if let Some(fix) = &mut self.fix_match
            && fix.kind != kind
        {
            fix.kind = kind;
            fix.results.clear();
            self.run_fix_search(cx);
        }
    }

    fn choose_match(&mut self, ix: usize, cx: &mut Context<Self>) {
        let (Some(fix), Some(tmdb)) = (&mut self.fix_match, self.tmdb.clone()) else {
            return;
        };
        if matches!(fix.status, FixStatus::Applying(_)) {
            return;
        }
        let Some(candidate) = fix.results.get(ix).cloned() else {
            return;
        };
        fix.status = FixStatus::Applying(candidate.found.id);
        let (item, paths) = (fix.item.clone(), self.paths.clone());
        cx.notify();
        cx.spawn(async move |this, cx| {
            let job_item = item.clone();
            let applied = cx
                .background_executor()
                .spawn(async move {
                    library_sync::apply_match(
                        &paths,
                        &tmdb,
                        &job_item,
                        candidate.kind,
                        candidate.found.id,
                    )
                })
                .await;
            this.update(cx, |view, cx| match applied {
                Ok(()) => {
                    view.fix_match = None;
                    view.fix_focus_restore = true;
                    view.after_rematch(&item.key, cx);
                }
                Err(e) => {
                    if let Some(fix) = &mut view.fix_match {
                        fix.status = FixStatus::Failed(format!("Could not update: {e:#}").into());
                    }
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    /// Shows a title's new metadata everywhere after Fix match, refetching episode art for
    /// shows (it belonged to the old match).
    fn after_rematch(&mut self, key: &TitleKey, cx: &mut Context<Self>) {
        self.reload();
        let updated = match key.kind {
            ItemKind::Movie => self.movies.iter().find(|i| &i.key == key),
            ItemKind::Show => self.shows.iter().find(|i| &i.key == key),
        }
        .cloned();
        if let Some(updated) = updated {
            match &mut self.page {
                Page::Movie { item, .. } | Page::Show { item, .. } if &item.key == key => {
                    *item = updated;
                }
                _ => {}
            }
        }
        if key.kind == ItemKind::Show {
            self.episode_job = None;
            self.continue_art_tried.remove(&key.title);
            self.refresh(cx);
            self.start_episode_art(key.title.clone(), cx);
        } else {
            self.refresh(cx);
        }
        cx.notify();
    }

    pub(super) fn on_fix_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(fix) = &mut self.fix_match else {
            return;
        };
        let k = &event.keystroke;
        match k.key.as_str() {
            "escape" => {
                self.close_fix_match(window, cx);
                cx.stop_propagation();
                return;
            }
            "enter" => {
                if fix.query.text() != fix.searched || fix.results.is_empty() {
                    self.run_fix_search(cx);
                } else {
                    let ix = fix.selected;
                    self.choose_match(ix, cx);
                }
                cx.stop_propagation();
                return;
            }
            "down" => fix.selected = (fix.selected + 1).min(fix.results.len().saturating_sub(1)),
            "up" => fix.selected = fix.selected.saturating_sub(1),
            _ => {
                if fix.query.handle_key(k, cx) == KeyOutcome::Unhandled {
                    return;
                }
            }
        }
        cx.stop_propagation();
        cx.notify();
    }

    /// Floating panel in the style of search: a query field, a Movie / TV Show switch and the
    /// TMDB candidates. The current match is marked; clicking another one applies it.
    pub(super) fn render_fix_match(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let Some(fix) = &self.fix_match else {
            return theme::nothing();
        };
        let focused = self.fix_focus.is_focused(window);
        let current_id = fix.item.metadata.as_ref().and_then(|m| m.tmdb_id);

        let heading = div()
            .px_5()
            .pt_4()
            .flex()
            .items_center()
            .gap_3()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .font_family(theme::FONT_DISPLAY)
                            .text_size(px(18.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Fix match"),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme::text_tertiary())
                            .truncate()
                            .child(format!(
                                "Choose the right entry for “{}”",
                                fix.item.key.title
                            )),
                    ),
            )
            .child(segmented(
                "fix-kind",
                vec![
                    ("Movie".to_string(), fix.kind == ItemKind::Movie),
                    ("TV Show".to_string(), fix.kind == ItemKind::Show),
                ],
                cx.listener(|this, ix: &usize, _, cx| {
                    let kind = if *ix == 0 {
                        ItemKind::Movie
                    } else {
                        ItemKind::Show
                    };
                    this.set_fix_kind(kind, cx);
                }),
            ));

        let field = div()
            .id("fix-field")
            .mx_5()
            .mt_4()
            .mb_3()
            .h(px(44.))
            .px_4()
            .flex()
            .items_center()
            .gap_3()
            .rounded(px(10.))
            .bg(rgba(0xffffff0d))
            .border_1()
            .border_color(theme::glass_border())
            .cursor_text()
            .child(icon("icons/search.svg", 16., theme::text_tertiary()))
            .child(div().flex_1().min_w_0().child(fix.query.element(
                "Title to search for",
                theme::text(),
                theme::text_tertiary(),
                focused,
            )))
            .child(div().text_xs().text_color(theme::text_tertiary()).child(
                if fix.query.text() != fix.searched {
                    "↵ Search"
                } else {
                    ""
                },
            ));
        let field = text_edit::mouse_handlers(
            field,
            &self.fix_focus,
            |this: &mut Self| this.fix_match.as_mut().map(|f| &mut f.query),
            cx,
        );

        let message = |text: SharedString| {
            div()
                .px_5()
                .py_5()
                .text_sm()
                .text_color(theme::text_tertiary())
                .child(text)
                .into_any_element()
        };
        let body: AnyElement = match &fix.status {
            FixStatus::Failed(e) => message(e.clone()),
            FixStatus::Searching if fix.results.is_empty() => message("Searching TMDB…".into()),
            _ if fix.results.is_empty() => message("No results. Try another title.".into()),
            _ => {
                let rows = fix.results.iter().enumerate().map(|(ix, candidate)| {
                    let selected = ix == fix.selected;
                    let current = Some(candidate.found.id) == current_id;
                    let applying = fix.status == FixStatus::Applying(candidate.found.id);
                    let title = match candidate.found.year {
                        Some(y) => format!("{} ({y})", candidate.found.title),
                        None => candidate.found.title.clone(),
                    };
                    let thumb = div()
                        .relative()
                        .w(px(40.))
                        .h(px(60.))
                        .flex_shrink_0()
                        .rounded(px(5.))
                        .overflow_hidden()
                        .bg(theme::placeholder(&candidate.found.title, None))
                        .child(art_layer(
                            candidate
                                .thumb
                                .as_ref()
                                .and_then(|p| p.to_str().map(str::to_string)),
                            5.,
                            ART_THUMB,
                        ));
                    let tag = if applying {
                        Some("Updating…")
                    } else if current {
                        Some("Current")
                    } else {
                        None
                    };
                    div()
                        .id(("fix-candidate", ix))
                        .flex()
                        .items_start()
                        .gap_3()
                        .px_3()
                        .py_2()
                        .rounded(px(8.))
                        .cursor_pointer()
                        .when(selected, |d| d.bg(rgba(0xffffff14)))
                        .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                            if let Some(fix) = &mut this.fix_match
                                && *hovered
                                && fix.selected != ix
                            {
                                fix.selected = ix;
                                cx.notify();
                            }
                        }))
                        .on_click(cx.listener(move |this, _, _, cx| this.choose_match(ix, cx)))
                        .child(thumb)
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .flex()
                                .flex_col()
                                .gap_1()
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap_2()
                                        .child(
                                            div()
                                                .min_w_0()
                                                .truncate()
                                                .text_sm()
                                                .font_weight(FontWeight::MEDIUM)
                                                .text_color(if selected {
                                                    theme::text()
                                                } else {
                                                    theme::text_secondary()
                                                })
                                                .child(title),
                                        )
                                        .children(tag.map(|t| {
                                            div()
                                                .flex_shrink_0()
                                                .px_2()
                                                .py_0p5()
                                                .rounded_full()
                                                .bg(rgba(0xffffff14))
                                                .text_xs()
                                                .text_color(theme::text_secondary())
                                                .child(t)
                                        })),
                                )
                                .children(candidate.found.overview.clone().map(|o| {
                                    div()
                                        .text_xs()
                                        .line_height(px(FIX_OVERVIEW_LINE_H))
                                        .max_h(px(FIX_OVERVIEW_LINE_H * 2.))
                                        .overflow_hidden()
                                        .text_color(theme::text_tertiary())
                                        .child(o)
                                })),
                        )
                });
                div()
                    .id("fix-results")
                    .max_h(px(420.))
                    .overflow_y_scroll()
                    .p_1p5()
                    .flex()
                    .flex_col()
                    .children(rows)
                    .into_any_element()
            }
        };

        let panel = div()
            .id("fix-panel")
            .occlude()
            .on_action(|_: &FocusNext, _, _| {})
            .on_action(|_: &FocusPrev, _, _| {})
            .track_focus(&self.fix_focus)
            .on_key_down(cx.listener(Self::on_fix_key))
            .w(px(600.))
            .rounded(px(16.))
            .overflow_hidden()
            // True black with a light outline, like the search panel and Settings cards.
            .bg(rgba(0x000000ff))
            .border_1()
            .border_color(rgba(0xffffff38))
            .shadow(theme::frost_shadow())
            .flex()
            .flex_col()
            .child(heading)
            .child(field)
            .child(
                div()
                    .border_t_1()
                    .border_color(rgba(0xffffff1f))
                    .child(body),
            );

        div()
            .id("fix-overlay")
            .absolute()
            .inset_0()
            .bg(rgba(0x00000099))
            .flex()
            .justify_center()
            .items_start()
            .pt(px(110.))
            .on_mouse_down(
                gpui::MouseButton::Left,
                cx.listener(|this, _, window, cx| this.close_fix_match(window, cx)),
            )
            .child(panel)
            .with_animation(
                "fix-overlay-in",
                Animation::new(theme::motion(160)).with_easing(ease_out_quint()),
                |el, t| el.opacity(t),
            )
            .into_any_element()
    }
}
