//! The watchlist in the app: the Home shelf, the bookmark button on detail pages and the
//! "Add to Watchlist" menu entry.

use super::*;

impl LibraryView {
    /// Reads the watchlist again, keeping the titles still in the library.
    pub(super) fn reload_watchlist(&mut self) {
        match self.db.watchlist() {
            Ok(keys) => {
                self.watchlist = keys.iter().filter_map(|k| self.item_for_key(k)).collect();
            }
            Err(e) => tracing::error!("reading watchlist: {e}"),
        }
    }

    pub(super) fn on_watchlist(&self, key: &TitleKey) -> bool {
        self.watchlist.iter().any(|item| &item.key == key)
    }

    pub(super) fn set_on_watchlist(&mut self, key: &TitleKey, on: bool, cx: &mut Context<Self>) {
        if let Err(e) = self.db.set_on_watchlist(key, on) {
            self.show_error(format!("Could not update the watchlist: {e}"), cx);
        }
        self.reload_watchlist();
        cx.notify();
    }

    /// "Add to Watchlist" or "Remove from Watchlist", for a title's menus.
    pub(super) fn watchlist_entry(&self, key: &TitleKey) -> MenuEntry {
        let key = key.clone();
        if self.on_watchlist(&key) {
            MenuEntry::new(
                "Remove from Watchlist",
                "icons/bookmark-filled.svg",
                move |this, _, cx| this.set_on_watchlist(&key, false, cx),
            )
        } else {
            MenuEntry::new(
                "Add to Watchlist",
                "icons/bookmark.svg",
                move |this, _, cx| this.set_on_watchlist(&key, true, cx),
            )
        }
    }

    /// The bookmark beside a detail page's Play button: filled while the title is saved.
    pub(super) fn watchlist_button(&self, key: &TitleKey, cx: &mut Context<Self>) -> AnyElement {
        let on = self.on_watchlist(key);
        let key = key.clone();
        theme::round_button_styled(
            "detail-watchlist",
            if on {
                "icons/bookmark-filled.svg"
            } else {
                "icons/bookmark.svg"
            },
            theme::BUTTON_H,
            false,
            cx.listener(move |this, _, _, cx| this.set_on_watchlist(&key, !on, cx)),
        )
        .tooltip(theme::text_tooltip(if on {
            "Remove from Watchlist"
        } else {
            "Add to Watchlist"
        }))
        .into_any_element()
    }

    /// The Watchlist page (from the sidebar): every saved title, most recently saved first.
    pub(super) fn render_watchlist(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let count = self.watchlist.len();
        let header = div()
            .flex_shrink_0()
            .pt(px(NAV_HEIGHT + 24.))
            .pb_2()
            .px(px(PAGE_PADDING))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(
                        div()
                            .font_family(theme::FONT_DISPLAY)
                            .text_size(px(36.))
                            .font_weight(FontWeight::BOLD)
                            .child("Watchlist"),
                    )
                    .child(
                        div()
                            .text_color(theme::text_tertiary())
                            .child(if count == 1 {
                                "1 title".to_string()
                            } else {
                                format!("{count} titles")
                            }),
                    )
                    .child(div().flex_1())
                    .child(self.select_button(SelectPlace::Watchlist, count, cx)),
            );
        let body: AnyElement = if self.watchlist.is_empty() {
            div()
                .flex_1()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap_2()
                .child(icon("icons/bookmark.svg", 32., theme::text_tertiary()))
                .child(
                    div()
                        .font_weight(FontWeight::MEDIUM)
                        .child("Nothing saved yet"),
                )
                .child(
                    div()
                        .text_sm()
                        .text_color(theme::text_secondary())
                        .child("Use the bookmark on a title's page, or ⋯ on a poster."),
                )
                .into_any_element()
        } else {
            let width =
                f32::from(window.viewport_size().width) - self.sidebar_width() - PAGE_PADDING * 2.;
            let cols =
                (((width + SHELF_GAP) / (MIN_GRID_TILE + SHELF_GAP)).floor() as usize).max(2);
            let tile = (width - SHELF_GAP * (cols as f32 - 1.)) / cols as f32;
            div()
                .id("watchlist-grid")
                .flex_1()
                .overflow_y_scroll()
                .track_scroll(&self.collection_scroll)
                .px(px(PAGE_PADDING))
                .pt_3()
                // Room for the selection bar under the last row while selecting.
                .pb(px(if self.selecting(SelectPlace::Watchlist) {
                    132.
                } else {
                    48.
                }))
                .flex()
                .flex_wrap()
                .gap_x(px(SHELF_GAP))
                .gap_y_8()
                .content_start()
                .children(
                    self.watchlist
                        .iter()
                        .map(|item| self.poster_card("watchlist", item, tile, cx)),
                )
                .into_any_element()
        };
        div()
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .child(header)
            .child(body)
            .child(self.render_selection_bar(SelectPlace::Watchlist, cx))
            .into_any_element()
    }

    /// The Home shelf: saved titles not yet watched, most recently saved first. Hidden when
    /// there are none.
    pub(super) fn render_watchlist_shelf(&self, cx: &mut Context<Self>) -> AnyElement {
        let cards = self
            .watchlist
            .iter()
            .filter(|item| !self.is_watched(item))
            .take(SHELF_LIMIT)
            .map(|item| self.poster_card("shelf-watchlist", item, POSTER_W, cx))
            .collect();
        self.shelf("Watchlist", "shelf-watchlist", cards, None, cx)
    }
}
