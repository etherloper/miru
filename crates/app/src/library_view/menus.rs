//! The "⋯" menu on detail pages (Mark as watched, Fix match).

use super::*;

pub(super) type MenuAction = Rc<dyn Fn(&mut LibraryView, &mut Window, &mut Context<LibraryView>)>;

pub(super) struct MenuEntry {
    pub label: SharedString,
    pub icon: &'static str,
    pub action: MenuAction,
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
        }
    }
}

impl LibraryView {
    pub(super) fn close_more_menu(&mut self) {
        if std::mem::take(&mut self.more_menu) {
            self.more_menu_closed = Instant::now();
        }
    }

    /// Round "⋯" button beside a detail page's play buttons, opening a small menu below it.
    pub(super) fn more_menu(&self, entries: Vec<MenuEntry>, cx: &mut Context<Self>) -> AnyElement {
        let open = self.more_menu;
        let trigger = theme::round_button_styled(
            "detail-more",
            "icons/more.svg",
            44.,
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
            let rows = entries.into_iter().enumerate().map(|(ix, entry)| {
                let action = entry.action.clone();
                div()
                    .id(("more-option", ix))
                    .h(px(36.))
                    .px_3()
                    .flex()
                    .items_center()
                    .gap_3()
                    .rounded(px(8.))
                    .text_sm()
                    .text_color(theme::text_secondary())
                    .cursor_pointer()
                    .hover(|s| s.bg(rgba(0xffffff14)).text_color(theme::text()))
                    .child(icon(entry.icon, 15., theme::text_secondary()))
                    .child(entry.label)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.close_more_menu();
                        action(this, window, cx);
                        cx.notify();
                    }))
            });
            let list = div()
                .id("more-menu")
                .occlude()
                .mt_2()
                .min_w(px(220.))
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
                    this.close_more_menu();
                    cx.notify();
                }));
            div().absolute().top_full().left_0().child(
                gpui::deferred(
                    gpui::anchored()
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
