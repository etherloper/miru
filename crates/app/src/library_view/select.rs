//! Choosing several titles to change them together, on the Movies and TV Shows grids, a
//! collection's page and the Watchlist: mark watched or unwatched, add to or remove from the
//! watchlist, and add to (or remove from) collections.
//!
//! "Select" in the page's header (or Ctrl+click on a poster) starts it; clicking posters then
//! toggles them, Shift+click selects a run, Ctrl+A selects everything shown, and Esc stops.

use super::*;

/// The page a selection belongs to.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum SelectPlace {
    Grid(Tab),
    Collection(i64),
    Watchlist,
}

impl SelectPlace {
    /// The poster card scope used on that page.
    fn scope(self) -> &'static str {
        match self {
            Self::Grid(_) => "grid",
            Self::Collection(_) => "collection",
            Self::Watchlist => "watchlist",
        }
    }
}

pub(super) struct Selection {
    place: SelectPlace,
    /// In the order chosen.
    keys: Vec<TitleKey>,
    /// Where a Shift+click run starts: the poster clicked last.
    anchor: Option<TitleKey>,
}

impl LibraryView {
    /// The page on show, if titles can be selected on it.
    pub(super) fn select_place(&self) -> Option<SelectPlace> {
        match &self.page {
            Page::Browse(tab @ (Tab::Movies | Tab::Shows)) => Some(SelectPlace::Grid(*tab)),
            Page::Collection { id, .. } => Some(SelectPlace::Collection(*id)),
            Page::Watchlist => Some(SelectPlace::Watchlist),
            _ => None,
        }
    }

    /// The titles shown on a selectable page, in order.
    fn shown_keys(&self, place: SelectPlace) -> Vec<TitleKey> {
        match place {
            SelectPlace::Grid(tab) => self
                .genre_filter(tab)
                .map(|f| f.items.iter().map(|i| i.key.clone()).collect())
                .unwrap_or_default(),
            SelectPlace::Collection(id) => match &self.page {
                Page::Collection { id: open, items } if *open == id => {
                    items.iter().map(|i| i.key.clone()).collect()
                }
                _ => Vec::new(),
            },
            SelectPlace::Watchlist => self.watchlist.iter().map(|i| i.key.clone()).collect(),
        }
    }

    /// Whether `key`'s poster in `scope` shows a selection check, and if so whether it's
    /// ticked. `None` outside selection.
    pub(super) fn selection_state(&self, scope: &str, key: &TitleKey) -> Option<bool> {
        let selection = self.selection.as_ref()?;
        (selection.place.scope() == scope && self.select_place() == Some(selection.place))
            .then(|| selection.keys.contains(key))
    }

    pub(super) fn selecting(&self, place: SelectPlace) -> bool {
        self.selection.as_ref().is_some_and(|s| s.place == place)
    }

    fn start_selecting(
        &mut self,
        place: SelectPlace,
        first: Option<TitleKey>,
        cx: &mut Context<Self>,
    ) {
        self.close_card_menu();
        self.close_genre_menu();
        self.close_more_menu();
        self.selection = Some(Selection {
            place,
            anchor: first.clone(),
            keys: first.into_iter().collect(),
        });
        cx.notify();
    }

    /// Ctrl+click on a poster: starts selecting, with that poster, if its page allows it.
    pub(super) fn ctrl_click_poster(
        &mut self,
        scope: &str,
        key: TitleKey,
        cx: &mut Context<Self>,
    ) -> bool {
        match self.select_place().filter(|p| p.scope() == scope) {
            Some(place) => {
                self.start_selecting(place, Some(key), cx);
                true
            }
            None => false,
        }
    }

    pub(super) fn stop_selecting(&mut self, cx: &mut Context<Self>) {
        self.bulk_menu = false;
        if self.selection.take().is_some() {
            cx.notify();
        }
    }

    /// A click on a poster while selecting: toggles it, or with Shift selects every poster
    /// from the last one clicked to this one.
    pub(super) fn click_selected(&mut self, key: TitleKey, range: bool, cx: &mut Context<Self>) {
        let Some(place) = self.selection.as_ref().map(|s| s.place) else {
            return;
        };
        let shown = self.shown_keys(place);
        let Some(selection) = &mut self.selection else {
            return;
        };
        let run = range
            .then(|| {
                let from = shown
                    .iter()
                    .position(|k| Some(k) == selection.anchor.as_ref())?;
                let to = shown.iter().position(|k| *k == key)?;
                Some(shown[from.min(to)..=from.max(to)].to_vec())
            })
            .flatten();
        match run {
            Some(run) => {
                for k in run {
                    if !selection.keys.contains(&k) {
                        selection.keys.push(k);
                    }
                }
            }
            None => {
                if let Some(at) = selection.keys.iter().position(|k| *k == key) {
                    selection.keys.remove(at);
                } else {
                    selection.keys.push(key.clone());
                }
            }
        }
        selection.anchor = Some(key);
        cx.notify();
    }

    /// Selects every title the page shows, or clears the selection if they all are already.
    pub(super) fn select_all(&mut self, place: SelectPlace, cx: &mut Context<Self>) {
        let shown = self.shown_keys(place);
        if shown.is_empty() {
            return;
        }
        if !self.selecting(place) {
            self.start_selecting(place, None, cx);
        }
        let Some(selection) = &mut self.selection else {
            return;
        };
        if shown.iter().all(|k| selection.keys.contains(k)) {
            selection.keys.clear();
        } else {
            selection.keys = shown;
        }
        selection.anchor = None;
        cx.notify();
    }

    /// Drops selected titles the page no longer shows (after a filter change, a removal or a
    /// rescan).
    pub(super) fn prune_selection(&mut self) {
        let Some(place) = self.selection.as_ref().map(|s| s.place) else {
            return;
        };
        let shown: HashSet<TitleKey> = self.shown_keys(place).into_iter().collect();
        if let Some(selection) = &mut self.selection {
            selection.keys.retain(|k| shown.contains(k));
        }
    }

    fn selected_keys(&self) -> Vec<TitleKey> {
        self.selection
            .as_ref()
            .map(|s| s.keys.clone())
            .unwrap_or_default()
    }

    /// Every file of the selected titles: a movie's file, all of a show's episodes.
    fn selected_file_ids(&self) -> Vec<i64> {
        let mut ids = Vec::new();
        for key in self.selected_keys() {
            match key.kind {
                ItemKind::Movie => ids.extend(self.item_for_key(&key).map(|i| i.file_id)),
                ItemKind::Show => match self.db.episodes(&key.title) {
                    Ok(episodes) => ids.extend(episodes.iter().map(|e| e.id)),
                    Err(e) => tracing::error!("reading episodes: {e}"),
                },
            }
        }
        ids
    }

    fn bulk_watched(&mut self, watched: bool, cx: &mut Context<Self>) {
        let ids = self.selected_file_ids();
        if !ids.is_empty() {
            self.mark_watched(ids, watched, cx);
        }
        self.prune_selection();
    }

    fn bulk_watchlist(&mut self, on: bool, cx: &mut Context<Self>) {
        for key in self.selected_keys() {
            if let Err(e) = self.db.set_on_watchlist(&key, on) {
                self.show_error(format!("Could not update the watchlist: {e}"), cx);
                break;
            }
        }
        self.reload_watchlist();
        // Removing titles on the Watchlist page takes them off it.
        self.prune_selection();
        cx.notify();
    }

    fn bulk_add_to_collection(&mut self, id: i64, cx: &mut Context<Self>) {
        for key in self.selected_keys() {
            if let Err(e) = self.db.add_to_collection(id, &key) {
                self.show_error(format!("Could not add to the collection: {e}"), cx);
                break;
            }
        }
        self.flash_collection(id);
        self.reload_collections();
        cx.notify();
    }

    fn bulk_remove_from_collection(&mut self, id: i64, cx: &mut Context<Self>) {
        for key in self.selected_keys() {
            if let Err(e) = self.db.remove_from_collection(id, &key) {
                self.show_error(format!("Could not remove from the collection: {e}"), cx);
                break;
            }
        }
        self.reload_collections();
        self.prune_selection();
        cx.notify();
    }

    fn bulk_new_collection(&mut self, cx: &mut Context<Self>) {
        let keys = self.selected_keys();
        let Some((first, rest)) = keys.split_first() else {
            return;
        };
        self.new_collection(Some(first.clone()), cx);
        // The new collection is the one being named.
        if let Some(id) = self.collection_edit.as_ref().map(|e| e.id) {
            for key in rest {
                if let Err(e) = self.db.add_to_collection(id, key) {
                    self.show_error(format!("Could not add to the collection: {e}"), cx);
                    break;
                }
            }
            self.reload_collections();
        }
        cx.notify();
    }

    /// "Add to collection": a button opening the list of collections (and "New collection…").
    /// On a collection's page, that collection is left out.
    fn add_to_collection_button(
        &self,
        any: bool,
        skip: Option<i64>,
        cx: &mut Context<Self>,
    ) -> Div {
        let menu = (self.bulk_menu && any).then(|| {
            let mut entries: Vec<MenuEntry> = self
                .collections
                .iter()
                .filter(|c| Some(c.id) != skip)
                .map(|c| {
                    let id = c.id;
                    MenuEntry::new(
                        format!("Add to {}", c.name),
                        "icons/collection.svg",
                        move |this, _, cx| this.bulk_add_to_collection(id, cx),
                    )
                })
                .collect();
            entries.push(
                MenuEntry::new("New collection…", "icons/plus.svg", |this, _, cx| {
                    this.bulk_new_collection(cx)
                })
                .divided(),
            );
            let list = self.menu_panel(
                "bulk-collection-menu",
                entries,
                |this| {
                    this.bulk_menu = false;
                    this.bulk_menu_closed = Instant::now();
                },
                cx,
            );
            div().absolute().bottom_full().left_0().child(
                gpui::deferred(
                    gpui::anchored()
                        .anchor(gpui::Anchor::BottomLeft)
                        .snap_to_window_with_margin(px(12.))
                        .child(div().mb_2().child(list)),
                )
                .with_priority(1),
            )
        });
        div()
            .relative()
            .child(
                bar_button(
                    "bulk-collection",
                    "icons/collection.svg",
                    "Add to collection".into(),
                    any,
                )
                .when(any, |d| {
                    d.on_click(cx.listener(|this, _, _, cx| {
                        if this.bulk_menu {
                            this.bulk_menu = false;
                        } else if this.bulk_menu_closed.elapsed() > Duration::from_millis(200) {
                            this.bulk_menu = true;
                        }
                        cx.notify();
                    }))
                }),
            )
            .children(menu)
    }

    /// The bar along the bottom of a selectable page while selecting: the count, then what
    /// can be done to the selected titles there.
    pub(super) fn render_selection_bar(
        &self,
        place: SelectPlace,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(selection) = self.selection.as_ref().filter(|s| s.place == place) else {
            return theme::nothing();
        };
        let count = selection.keys.len();
        let any = count > 0;
        let shown = self.shown_keys(place).len();
        let all_shown = any && count == shown;
        let all_watched = any
            && selection
                .keys
                .iter()
                .all(|k| self.item_for_key(k).is_some_and(|i| self.is_watched(&i)));
        let all_listed = any && selection.keys.iter().all(|k| self.on_watchlist(k));

        let divider = || div().w(px(1.)).h(px(24.)).mx_1().bg(rgba(0xffffff26));
        let watched_button = bar_button(
            "bulk-watched",
            if all_watched {
                "icons/circle.svg"
            } else {
                "icons/check.svg"
            },
            if all_watched {
                "Mark unwatched"
            } else {
                "Mark watched"
            }
            .into(),
            any,
        )
        .when(any, |d| {
            d.on_click(cx.listener(move |this, _, _, cx| this.bulk_watched(!all_watched, cx)))
        });
        // On the Watchlist everything selected is on it, so the button only removes.
        let watchlist_on = all_listed || place == SelectPlace::Watchlist;
        let watchlist_button = bar_button(
            "bulk-watchlist",
            if watchlist_on {
                "icons/bookmark-filled.svg"
            } else {
                "icons/bookmark.svg"
            },
            if watchlist_on {
                "Remove from Watchlist"
            } else {
                "Add to Watchlist"
            }
            .into(),
            any,
        )
        .when(any, |d| {
            d.on_click(cx.listener(move |this, _, _, cx| this.bulk_watchlist(!watchlist_on, cx)))
        });
        let collection_actions: AnyElement = match place {
            SelectPlace::Collection(id) => {
                let name = self
                    .collection(id)
                    .map_or("collection".to_string(), |c| c.name.clone());
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .child(self.add_to_collection_button(any, Some(id), cx))
                    .child(
                        bar_button(
                            "bulk-remove-collection",
                            "icons/minus.svg",
                            format!("Remove from {name}").into(),
                            any,
                        )
                        .when(any, |d| {
                            d.on_click(cx.listener(move |this, _, _, cx| {
                                this.bulk_remove_from_collection(id, cx)
                            }))
                        }),
                    )
                    .into_any_element()
            }
            _ => self
                .add_to_collection_button(any, None, cx)
                .into_any_element(),
        };

        let bar = div()
            .id("selection-bar")
            .occlude()
            .max_w(px(1100.))
            .h(px(56.))
            .pl_5()
            .pr_2()
            .flex()
            .items_center()
            .gap_1()
            .rounded(px(28.))
            .bg(rgba(0x0b0b0ef2))
            .border_1()
            .border_color(rgba(0xffffff38))
            .shadow(theme::frost_shadow())
            .text_sm()
            .child(
                div()
                    .min_w(px(92.))
                    .flex_shrink_0()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(match count {
                        0 => "None selected".to_string(),
                        n => format!("{n} selected"),
                    }),
            )
            .child(
                bar_button(
                    "bulk-all",
                    "icons/select.svg",
                    if all_shown {
                        "Select none"
                    } else {
                        "Select all"
                    }
                    .into(),
                    shown > 0,
                )
                .on_click(cx.listener(move |this, _, _, cx| this.select_all(place, cx))),
            )
            .child(divider())
            .child(watched_button)
            .child(watchlist_button)
            .child(collection_actions)
            .child(divider())
            .child(
                round_button(
                    "bulk-done",
                    "icons/window-close.svg",
                    36.,
                    cx.listener(|this, _, _, cx| this.stop_selecting(cx)),
                )
                .tooltip(theme::text_tooltip("Done  Esc")),
            )
            .with_animation(
                "selection-bar-in",
                Animation::new(theme::motion(200)).with_easing(ease_out_quint()),
                |el, t| el.mb(px(-16. * (1. - t))).opacity(t),
            );

        div()
            .absolute()
            .left_0()
            .right_0()
            .bottom(px(28.))
            .px(px(PAGE_PADDING))
            .flex()
            .justify_center()
            .child(bar)
            .into_any_element()
    }

    /// "Select" in a page's header, or "Done" while selecting. Hidden while there's nothing
    /// to select.
    pub(super) fn select_button(
        &self,
        place: SelectPlace,
        count: usize,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if count == 0 {
            return theme::nothing();
        }
        let selecting = self.selecting(place);
        div()
            .id("page-select")
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
            .border_1()
            .text_sm()
            .font_weight(FontWeight::MEDIUM)
            .whitespace_nowrap()
            .cursor_pointer()
            .map(|d| {
                if selecting {
                    d.bg(theme::text())
                        .border_color(theme::text())
                        .text_color(theme::bg())
                        .child(icon("icons/check.svg", 15., theme::bg()))
                        .child("Done")
                } else {
                    d.bg(theme::glass())
                        .border_color(theme::glass_border())
                        .hover(|s| s.bg(theme::glass_hover()))
                        .child(icon("icons/select.svg", 15., theme::text()))
                        .child("Select")
                }
            })
            .tooltip(theme::text_tooltip(if selecting {
                "Stop selecting  Esc"
            } else {
                "Choose several titles  Ctrl+click"
            }))
            .on_click(cx.listener(move |this, _, _, cx| {
                if this.selecting(place) {
                    this.stop_selecting(cx);
                } else {
                    this.start_selecting(place, None, cx);
                }
            }))
            .into_any_element()
    }
}

/// A labelled button on the selection bar; dimmed and inert when `enabled` is false.
fn bar_button(
    id: &'static str,
    icon_path: &'static str,
    label: SharedString,
    enabled: bool,
) -> gpui::Stateful<Div> {
    div()
        .id(id)
        .tab_index(0)
        .focus_visible(theme::focus_ring)
        .h(px(40.))
        .px_3()
        .flex()
        .flex_shrink_0()
        .items_center()
        .gap_2()
        .rounded(px(20.))
        .font_weight(FontWeight::MEDIUM)
        .whitespace_nowrap()
        .child(icon(icon_path, 15., theme::text()))
        .child(label)
        .map(|d| {
            if enabled {
                d.cursor_pointer().hover(|s| s.bg(rgba(0xffffff1a)))
            } else {
                d.opacity(0.4)
            }
        })
}
