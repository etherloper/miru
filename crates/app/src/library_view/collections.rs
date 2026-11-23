//! Collections in the app: the sidebar list (with inline naming), the collection page, the
//! "Add to collection" menu entries, and the "⋯" menu on posters.

use gpui::{Pixels, Point};
use miru_core::collections::Collection;

use super::*;
use crate::text_edit::{self, KeyOutcome, TextEdit};

/// How long a collection's sidebar row glows after a title lands in it.
pub(super) const COLLECTION_FLASH: Duration = Duration::from_millis(900);

/// A collection being named in the sidebar.
pub(super) struct CollectionEdit {
    pub id: i64,
    pub name: TextEdit,
}

/// The "⋯" menu opened from a poster, with what it needs to know about the title.
pub(super) struct CardMenu {
    pub key: TitleKey,
    pub position: Point<Pixels>,
    /// The collection page it was opened on (offers "Remove from" that collection).
    pub collection: Option<i64>,
    pub watched: bool,
    pub has_history: bool,
    pub file_ids: Vec<i64>,
}

impl LibraryView {
    pub(super) fn collection(&self, id: i64) -> Option<&Collection> {
        self.collections.iter().find(|c| c.id == id)
    }

    /// The library entry for a key, if the title is still in the library.
    pub(super) fn item_for_key(&self, key: &TitleKey) -> Option<LibraryItem> {
        match key.kind {
            ItemKind::Movie => &self.movies,
            ItemKind::Show => &self.shows,
        }
        .iter()
        .find(|i| &i.key == key)
        .cloned()
    }

    /// A collection's titles that are in the library, most recently added first.
    pub(super) fn collection_items(&self, id: i64) -> Vec<LibraryItem> {
        match self.db.collection_keys(id) {
            Ok(keys) => keys.iter().filter_map(|k| self.item_for_key(k)).collect(),
            Err(e) => {
                tracing::error!("reading collection: {e}");
                Vec::new()
            }
        }
    }

    pub(super) fn reload_collections(&mut self) {
        match self.db.collections() {
            Ok(collections) => self.collections = collections,
            Err(e) => tracing::error!("reading collections: {e}"),
        }
        if let Page::Collection { id, .. } = self.page {
            let fresh = self.collection_items(id);
            if let Page::Collection { items, .. } = &mut self.page {
                *items = fresh;
            }
        }
    }

    pub(super) fn open_collection(&mut self, id: i64, cx: &mut Context<Self>) {
        if matches!(self.page, Page::Collection { id: open, .. } if open == id) {
            return;
        }
        let items = self.collection_items(id);
        self.navigate(Page::Collection { id, items }, cx);
    }

    fn next_collection_name(&self) -> String {
        let taken = |name: &str| self.collections.iter().any(|c| c.name == name);
        let mut name = "New Collection".to_string();
        let mut n = 2;
        while taken(&name) {
            name = format!("New Collection {n}");
            n += 1;
        }
        name
    }

    /// Makes a collection (holding `item`, if given) and starts naming it in the sidebar.
    pub(super) fn new_collection(&mut self, item: Option<TitleKey>, cx: &mut Context<Self>) {
        let name = self.next_collection_name();
        let created = self.db.create_collection(&name).and_then(|id| {
            if let Some(key) = &item {
                self.db.add_to_collection(id, key)?;
            }
            Ok(id)
        });
        match created {
            Ok(id) => {
                self.reload_collections();
                if item.is_some() {
                    self.flash_collection(id);
                }
                self.start_rename(id, cx);
            }
            Err(e) => self.show_error(format!("Could not create the collection: {e}"), cx),
        }
    }

    pub(super) fn add_to_collection(&mut self, id: i64, key: &TitleKey, cx: &mut Context<Self>) {
        match self.db.add_to_collection(id, key) {
            Ok(_) => {
                self.flash_collection(id);
                self.reload_collections();
            }
            Err(e) => self.show_error(format!("Could not add to the collection: {e}"), cx),
        }
        cx.notify();
    }

    pub(super) fn remove_from_collection(
        &mut self,
        id: i64,
        key: &TitleKey,
        cx: &mut Context<Self>,
    ) {
        if let Err(e) = self.db.remove_from_collection(id, key) {
            self.show_error(format!("Could not remove from the collection: {e}"), cx);
        }
        self.reload_collections();
        cx.notify();
    }

    pub(super) fn flash_collection(&mut self, id: i64) {
        self.collection_flash = Some((id, Instant::now()));
        self.collection_flash_epoch += 1;
    }

    /// Starts renaming a collection in the sidebar (opening the sidebar if it's collapsed).
    pub(super) fn start_rename(&mut self, id: i64, cx: &mut Context<Self>) {
        let Some(name) = self.collection(id).map(|c| c.name.clone()) else {
            return;
        };
        if self.settings.sidebar_collapsed {
            self.toggle_sidebar(cx);
        }
        let mut edit = TextEdit::new(name);
        edit.select_all();
        self.collection_edit = Some(CollectionEdit { id, name: edit });
        self.collection_focus_pending = true;
        cx.notify();
    }

    fn commit_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(edit) = self.collection_edit.take() else {
            return;
        };
        let name = edit.name.text().trim().to_string();
        if !name.is_empty()
            && let Err(e) = self.db.rename_collection(edit.id, &name)
        {
            self.show_error(format!("Could not rename the collection: {e}"), cx);
        }
        self.reload_collections();
        window.focus(&self.focus, cx);
        cx.notify();
    }

    pub(super) fn on_collection_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(edit) = &mut self.collection_edit else {
            return;
        };
        match event.keystroke.key.as_str() {
            "enter" => self.commit_rename(window, cx),
            "escape" => {
                self.collection_edit = None;
                window.focus(&self.focus, cx);
            }
            _ => {
                if edit.name.handle_key(&event.keystroke, cx) == KeyOutcome::Unhandled {
                    return;
                }
            }
        }
        cx.stop_propagation();
        cx.notify();
    }

    fn delete_collection(&mut self, id: i64, cx: &mut Context<Self>) {
        self.collection_delete = None;
        if let Err(e) = self.db.delete_collection(id) {
            self.show_error(format!("Could not delete the collection: {e}"), cx);
            return;
        }
        if matches!(self.page, Page::Collection { id: open, .. } if open == id) {
            // Leave the deleted page without keeping it in Back/Forward.
            self.page = Page::Browse(Tab::Home);
            self.back_stack
                .retain(|p| !matches!(p, Page::Collection { id: gone, .. } if *gone == id));
            self.forward_stack
                .retain(|p| !matches!(p, Page::Collection { id: gone, .. } if *gone == id));
            self.show_page(cx);
        }
        self.reload_collections();
        cx.notify();
    }

    /// "Add to …" / "Remove from …" for each collection, and "New collection…".
    pub(super) fn collection_entries(&self, key: &TitleKey) -> Vec<MenuEntry> {
        let member = self.db.collections_with(key).unwrap_or_default();
        let mut entries: Vec<MenuEntry> = self
            .collections
            .iter()
            .map(|c| {
                let (id, key) = (c.id, key.clone());
                if member.contains(&c.id) {
                    MenuEntry::new(
                        format!("Remove from {}", c.name),
                        "icons/minus.svg",
                        move |this, _, cx| this.remove_from_collection(id, &key, cx),
                    )
                } else {
                    MenuEntry::new(
                        format!("Add to {}", c.name),
                        "icons/collection.svg",
                        move |this, _, cx| this.add_to_collection(id, &key, cx),
                    )
                }
            })
            .collect();
        let key = key.clone();
        entries.push(MenuEntry::new(
            "New collection…",
            "icons/plus.svg",
            move |this, _, cx| this.new_collection(Some(key.clone()), cx),
        ));
        if let Some(first) = entries.first_mut() {
            first.divided = true;
        }
        entries
    }

    // ---- poster menu -----------------------------------------------------------------------

    pub(super) fn open_card_menu(
        &mut self,
        key: TitleKey,
        scope: &'static str,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        let Some(item) = self.item_for_key(&key) else {
            return;
        };
        let (file_ids, has_history) = match key.kind {
            ItemKind::Movie => (
                vec![item.file_id],
                self.db.progress(item.file_id).ok().flatten().is_some(),
            ),
            ItemKind::Show => (
                self.db
                    .episodes(&key.title)
                    .map(|e| e.iter().map(|f| f.id).collect())
                    .unwrap_or_default(),
                self.db
                    .show_progress(&key.title)
                    .is_ok_and(|p| !p.is_empty()),
            ),
        };
        let collection = match self.page {
            Page::Collection { id, .. } if scope == "collection" => Some(id),
            _ => None,
        };
        self.close_more_menu();
        self.card_menu = Some(CardMenu {
            watched: self.is_watched(&item),
            key,
            position,
            collection,
            has_history,
            file_ids,
        });
        cx.notify();
    }

    pub(super) fn close_card_menu(&mut self) {
        if self.card_menu.take().is_some() {
            self.card_menu_closed = Instant::now();
        }
    }

    pub(super) fn render_card_menu(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(menu) = &self.card_menu else {
            return theme::nothing();
        };
        let (ids, watched) = (menu.file_ids.clone(), menu.watched);
        let mut entries = vec![MenuEntry::new(
            if watched {
                "Mark as unwatched"
            } else {
                "Mark as watched"
            },
            if watched {
                "icons/circle.svg"
            } else {
                "icons/check.svg"
            },
            move |this, _, cx| this.mark_watched(ids.clone(), !watched, cx),
        )];
        if menu.has_history {
            let ids = menu.file_ids.clone();
            entries.push(MenuEntry::new(
                "Remove from watch history",
                "icons/trash.svg",
                move |this, _, cx| {
                    for id in &ids {
                        if let Err(e) = this.db.remove_from_history(*id) {
                            tracing::error!("removing from history: {e}");
                        }
                    }
                    this.refresh(cx);
                },
            ));
        }
        if let Some(name) = menu
            .collection
            .and_then(|id| self.collection(id))
            .map(|c| (c.id, c.name.clone()))
        {
            let (id, key) = (name.0, menu.key.clone());
            entries.push(
                MenuEntry::new(
                    format!("Remove from {}", name.1),
                    "icons/minus.svg",
                    move |this, _, cx| this.remove_from_collection(id, &key, cx),
                )
                .divided(),
            );
        } else {
            entries.extend(self.collection_entries(&menu.key));
        }
        let panel = self.menu_panel("card-menu", entries, |this| this.close_card_menu(), cx);
        gpui::deferred(
            gpui::anchored()
                .position(menu.position)
                .snap_to_window_with_margin(px(12.))
                .child(div().mt_1().child(panel)),
        )
        .with_priority(2)
        .into_any_element()
    }

    // ---- sidebar ---------------------------------------------------------------------------

    /// The Collections list under the sidebar's destinations, with a + to make one. While a
    /// poster is dragged, the rows are drop targets and a "New collection" target appears.
    pub(super) fn render_sidebar_collections(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let collapsed = self.settings.sidebar_collapsed;
        let dragging = self.poster_drag.as_ref().is_some_and(|d| d.lifted);
        let add = theme::icon_button(
            "collection-new",
            "icons/plus.svg",
            24.,
            true,
            cx.listener(|this, _, _, cx| this.new_collection(None, cx)),
        )
        .tooltip(theme::text_tooltip("New collection"));
        let heading = (!collapsed).then(|| {
            div()
                .h(px(28.))
                .mt_5()
                .mb_1()
                .pl_6()
                .pr_4()
                .flex()
                .items_center()
                .text_xs()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme::text_tertiary())
                .child(div().flex_1().child("COLLECTIONS"))
                .child(add)
        });
        let divider = collapsed.then(|| div().mx_5().my_3().h(px(1.)).bg(rgba(0xffffff0f)));

        let rows = self.collections.iter().map(|c| {
            let selected = matches!(self.page, Page::Collection { id, .. } if id == c.id);
            let editing = self.collection_edit.as_ref().filter(|e| e.id == c.id);
            let target = DropTarget::Collection(c.id);
            let hovered = dragging && self.drop_target == Some(target);
            let flash = self
                .collection_flash
                .filter(|(id, at)| *id == c.id && at.elapsed() < COLLECTION_FLASH)
                .map(|_| self.collection_flash_epoch);
            let label: AnyElement = match editing {
                Some(edit) => {
                    let focused = self.collection_focus.is_focused(window);
                    text_edit::mouse_handlers(
                        div()
                            .id("collection-name")
                            .flex_1()
                            .min_w_0()
                            .h(px(26.))
                            .px_2()
                            .ml(px(-8.))
                            .flex()
                            .items_center()
                            .rounded(px(6.))
                            .bg(rgba(0xffffff0f))
                            .border_1()
                            .border_color(rgba(0xffffff40))
                            .cursor_text()
                            .track_focus(&self.collection_focus)
                            .on_key_down(cx.listener(Self::on_collection_key))
                            .on_mouse_down_out(
                                cx.listener(|this, _, window, cx| this.commit_rename(window, cx)),
                            )
                            .child(edit.name.element(
                                "Collection name",
                                theme::text(),
                                theme::text_tertiary(),
                                focused,
                            )),
                        &self.collection_focus,
                        |this: &mut Self| this.collection_edit.as_mut().map(|e| &mut e.name),
                        cx,
                    )
                    .into_any_element()
                }
                None => div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .child(c.name.clone())
                    .into_any_element(),
            };
            let ink = if selected || hovered {
                theme::text()
            } else {
                theme::text_secondary()
            };
            let id = c.id;
            let row = div()
                .id(("nav-collection", id as usize))
                .tab_index(0)
                .focus_visible(theme::focus_ring)
                .relative()
                .h(px(NAV_ITEM_H))
                .mx_3()
                .px_3()
                .flex()
                .items_center()
                .when(collapsed, |d| d.justify_center())
                .gap_3()
                .rounded(px(8.))
                .border_1()
                .border_color(rgba(0xffffff00))
                .cursor_pointer()
                .text_sm()
                .font_weight(FontWeight::MEDIUM)
                .text_color(ink)
                .when(selected, |d| d.bg(rgba(0xffffff0f)))
                .when(!selected && !dragging, |d| {
                    d.hover(|s| s.text_color(theme::text()))
                })
                .when(hovered, |d| {
                    d.bg(rgba(0xffffff1a)).border_color(rgba(0xffffff59))
                })
                // A glow that fades after a title lands here.
                .children(flash.map(|epoch| {
                    div()
                        .absolute()
                        .inset_0()
                        .rounded(px(8.))
                        .bg(rgba(0xffffff33))
                        .with_animation(
                            ("collection-flash", epoch),
                            Animation::new(theme::motion(COLLECTION_FLASH.as_millis() as u64))
                                .with_easing(ease_out_quint()),
                            |el, t| el.opacity(1. - t),
                        )
                }))
                .child(icon("icons/collection.svg", 18., ink))
                .when(!collapsed, |d| {
                    d.child(label).when(editing.is_none(), |d| {
                        d.child(
                            div()
                                .text_xs()
                                .text_color(theme::text_tertiary())
                                .child(c.item_count.to_string()),
                        )
                    })
                })
                .when(collapsed, |d| {
                    d.tooltip(theme::text_tooltip_owned(c.name.clone().into()))
                })
                .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                    this.set_drop_target(target, *hovered, cx)
                }));
            if editing.is_some() {
                row.into_any_element()
            } else {
                row.on_click(cx.listener(move |this, _, _, cx| {
                    if !this.click_ends_drag() {
                        this.open_collection(id, cx);
                    }
                }))
                .into_any_element()
            }
        });
        let rows: Vec<AnyElement> = rows.collect();

        // While dragging: a target that makes a new collection from the dropped title.
        let new_target = dragging.then(|| {
            let target = DropTarget::NewCollection;
            let hovered = self.drop_target == Some(target);
            div()
                .id("nav-collection-new")
                .h(px(NAV_ITEM_H))
                .mx_3()
                .px_3()
                .flex()
                .items_center()
                .when(collapsed, |d| d.justify_center())
                .gap_3()
                .rounded(px(8.))
                .border_1()
                .border_dashed()
                .border_color(if hovered {
                    rgba(0xffffff8c)
                } else {
                    rgba(0xffffff33)
                })
                .when(hovered, |d| d.bg(rgba(0xffffff1a)))
                .text_sm()
                .font_weight(FontWeight::MEDIUM)
                .text_color(if hovered {
                    theme::text()
                } else {
                    theme::text_secondary()
                })
                .child(icon("icons/plus.svg", 18., theme::text_secondary()))
                .when(!collapsed, |d| d.child("New collection"))
                .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                    this.set_drop_target(target, *hovered, cx)
                }))
                .with_animation(
                    "new-collection-target",
                    Animation::new(theme::motion(220)).with_easing(ease_out_quint()),
                    |el, t| el.opacity(t),
                )
        });

        div()
            .flex()
            .flex_col()
            .gap(px(NAV_ITEM_GAP))
            .children(heading)
            .children(divider)
            .children(rows)
            .children(new_target)
            .when(collapsed && !dragging, |d| {
                d.child(
                    div().mt_1().flex().justify_center().child(
                        theme::icon_button(
                            "collection-new-rail",
                            "icons/plus.svg",
                            30.,
                            true,
                            cx.listener(|this, _, _, cx| this.new_collection(None, cx)),
                        )
                        .tooltip(theme::text_tooltip("New collection")),
                    ),
                )
            })
            .into_any_element()
    }

    // ---- page ------------------------------------------------------------------------------

    pub(super) fn render_collection(
        &self,
        id: i64,
        items: &[LibraryItem],
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(collection) = self.collection(id) else {
            return div().into_any_element();
        };
        let count = items.len();
        let menu = self.more_menu(
            vec![
                MenuEntry::new("Rename", "icons/pencil.svg", move |this, _, cx| {
                    this.start_rename(id, cx)
                }),
                MenuEntry::new(
                    "Delete collection",
                    "icons/trash.svg",
                    move |this, _, cx| {
                        this.collection_delete = Some(id);
                        cx.notify();
                    },
                ),
            ],
            None,
            cx,
        );
        let title_row = div()
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
                    .child(collection.name.clone()),
            )
            .child(
                div()
                    .flex_shrink_0()
                    .text_color(theme::text_tertiary())
                    .child(if count == 1 {
                        "1 title".to_string()
                    } else {
                        format!("{count} titles")
                    }),
            )
            .child(div().flex_1())
            .child(menu);
        let confirm = (self.collection_delete == Some(id)).then(|| {
            div()
                .mt_4()
                .px_5()
                .py_3()
                .flex()
                .items_center()
                .gap_4()
                .rounded(px(12.))
                .bg(rgba(0xffffff0a))
                .border_1()
                .border_color(theme::glass_border())
                .child(div().flex_1().text_sm().child(format!(
                    "Delete “{}”? The titles stay in your library.",
                    collection.name
                )))
                .child(small_button(
                    "collection-delete-cancel",
                    "Cancel",
                    false,
                    cx.listener(|this, _, _, cx| {
                        this.collection_delete = None;
                        cx.notify();
                    }),
                ))
                .child(small_button(
                    "collection-delete-confirm",
                    "Delete",
                    true,
                    cx.listener(move |this, _, _, cx| this.delete_collection(id, cx)),
                ))
        });
        let header = div()
            .flex_shrink_0()
            .pt(px(NAV_HEIGHT + 24.))
            .pb_2()
            .px(px(PAGE_PADDING))
            .child(title_row)
            .children(confirm);

        let body: AnyElement = if items.is_empty() {
            div()
                .flex_1()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap_2()
                .child(icon("icons/collection.svg", 32., theme::text_tertiary()))
                .child(
                    div()
                        .font_weight(FontWeight::MEDIUM)
                        .child("Nothing in this collection yet"),
                )
                .child(
                    div()
                        .text_sm()
                        .text_color(theme::text_secondary())
                        .child("Drag a poster onto it in the sidebar, or use ⋯ on a title."),
                )
                .into_any_element()
        } else {
            let width =
                f32::from(window.viewport_size().width) - self.sidebar_width() - PAGE_PADDING * 2.;
            let cols =
                (((width + SHELF_GAP) / (MIN_GRID_TILE + SHELF_GAP)).floor() as usize).max(2);
            let tile = (width - SHELF_GAP * (cols as f32 - 1.)) / cols as f32;
            div()
                .id("collection-grid")
                .flex_1()
                .overflow_y_scroll()
                .px(px(PAGE_PADDING))
                .pt_3()
                .pb(px(48.))
                .flex()
                .flex_wrap()
                .gap_x(px(SHELF_GAP))
                .gap_y_8()
                .content_start()
                .children(
                    items
                        .iter()
                        .map(|item| self.poster_card("collection", item, tile, cx)),
                )
                .into_any_element()
        };
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(header)
            .child(body)
            .into_any_element()
    }
}
