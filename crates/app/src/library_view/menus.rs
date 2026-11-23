//! The "⋯" menus: on detail pages (Mark as watched, Fix match, collections) and on posters.

use super::*;

pub(super) type MenuAction = Rc<dyn Fn(&mut LibraryView, &mut Window, &mut Context<LibraryView>)>;

pub(super) struct MenuEntry {
    pub label: SharedString,
    pub icon: &'static str,
    pub action: MenuAction,
    /// Starts a new group, with a hairline above it.
    pub divided: bool,
    /// A destructive action, shown in red.
    pub danger: bool,
}

impl MenuEntry {
    pub fn new(
        label: impl Into<SharedString>,
        icon: &'static str,
        action: impl Fn(&mut LibraryView, &mut Window, &mut Context<LibraryView>) + 'static,
    ) -> Self {
        Self {
            label: label.into(),
            icon,
            action: Rc::new(action),
            divided: false,
            danger: false,
        }
    }

    pub fn danger(mut self) -> Self {
        self.danger = true;
        self
    }

    pub fn divided(mut self) -> Self {
        self.divided = true;
        self
    }
}

impl LibraryView {
    pub(super) fn close_more_menu(&mut self) {
        if std::mem::take(&mut self.more_menu) {
            self.more_menu_closed = Instant::now();
        }
    }

    /// A menu's panel: rows of icon and label; choosing one closes the menu (`close`) and
    /// runs it. Pressing outside closes it too.
    pub(super) fn menu_panel(
        &self,
        id: &'static str,
        entries: Vec<MenuEntry>,
        close: fn(&mut LibraryView),
        cx: &mut Context<Self>,
    ) -> AnyElement {
        self.menu_panel_sized(id, entries, 220., close, cx)
    }

    /// `menu_panel` at least `min_width` wide (it grows to fit its labels).
    pub(super) fn menu_panel_sized(
        &self,
        id: &'static str,
        entries: Vec<MenuEntry>,
        min_width: f32,
        close: fn(&mut LibraryView),
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let rows = entries.into_iter().enumerate().map(|(ix, entry)| {
            let action = entry.action.clone();
            let danger = entry.danger;
            let ink = if danger {
                theme::danger()
            } else {
                theme::text()
            };
            let row = div()
                .id((id, ix))
                .h(px(theme::MENU_ROW_H))
                .flex_shrink_0()
                .px_3()
                .flex()
                .items_center()
                .gap_3()
                .rounded(px(8.))
                .text_sm()
                .text_color(ink)
                .cursor_pointer()
                .hover(|s| s.bg(theme::menu_hover()))
                .child(icon(entry.icon, 16., ink))
                .child(div().min_w_0().truncate().child(entry.label))
                .on_click(cx.listener(move |this, _, window, cx| {
                    close(this);
                    action(this, window, cx);
                    cx.notify();
                }));
            if entry.divided && ix > 0 {
                div()
                    .flex()
                    .flex_col()
                    .child(div().mx_1().my_1().h(px(1.)).bg(theme::menu_divider()))
                    .child(row)
                    .into_any_element()
            } else {
                row.into_any_element()
            }
        });
        theme::menu_surface(div().id(id))
            .occlude()
            .min_w(px(min_width))
            .max_w(px(320.))
            .flex()
            .flex_col()
            .children(rows)
            .on_mouse_down_out(cx.listener(move |this, _, _, cx| {
                close(this);
                cx.notify();
            }))
            .with_animation(id, theme::menu_open_animation(), |el, t| {
                el.opacity(t).mt(px(-6. * (1. - t)))
            })
            .into_any_element()
    }

    /// Round "⋯" button beside a detail page's play buttons, opening a small menu below it.
    /// With `title`, the menu also offers adding it to (or removing it from) collections.
    pub(super) fn more_menu(
        &self,
        mut entries: Vec<MenuEntry>,
        title: Option<&TitleKey>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let open = self.more_menu;
        if open && let Some(key) = title {
            entries.extend(self.collection_entries(key));
        }
        let trigger = theme::round_button_styled(
            "detail-more",
            "icons/more.svg",
            theme::BUTTON_H,
            open,
            cx.listener(|this, _, _, cx| {
                if this.more_menu {
                    this.close_more_menu();
                } else if this.more_menu_closed.elapsed() > Duration::from_millis(200) {
                    this.more_menu = true;
                }
                cx.notify();
            }),
        )
        .tooltip(theme::text_tooltip("More"));

        let menu = open.then(|| {
            // Title menus list collections too and need the room; the others are short.
            let width = if title.is_some() { 220. } else { 140. };
            let list = self.menu_panel_sized(
                "more-menu",
                entries,
                width,
                |this| this.close_more_menu(),
                cx,
            );
            div().absolute().top_full().left_0().child(
                gpui::deferred(
                    gpui::anchored()
                        .snap_to_window_with_margin(px(12.))
                        .child(div().mt_2().child(list)),
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
