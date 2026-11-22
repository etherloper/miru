//! Window chrome: the sidebar, the top strip and the empty-library screen.

use super::*;

impl LibraryView {
    // ---- chrome ----------------------------------------------------------------------------

    /// Left navigation, kept quiet: the name, four plain rows and Settings at the bottom, on a
    /// flat surface divided from the page by a hairline. Only the selected row gets a faint
    /// fill. Collapses to an icon rail with hover labels (Ctrl+B or the toggle at the top).
    pub(super) fn render_sidebar(&self, cx: &mut Context<Self>) -> AnyElement {
        let collapsed = self.settings.sidebar_collapsed;
        let item = |id: &'static str,
                    label: &'static str,
                    icon_path: &'static str,
                    selected: bool,
                    on_click: ClickHandler| {
            let ink = if selected {
                theme::text()
            } else {
                theme::text_secondary()
            };
            div()
                .id(id)
                .tab_index(0)
                .focus_visible(theme::focus_ring)
                .h(px(NAV_ITEM_H))
                .mx_3()
                .px_3()
                .flex()
                .items_center()
                .when(collapsed, |d| d.justify_center())
                .gap_3()
                .rounded(px(8.))
                .cursor_pointer()
                .text_sm()
                .font_weight(FontWeight::MEDIUM)
                .text_color(ink)
                .when(selected, |d| d.bg(rgba(0xffffff0f)))
                .when(!selected, |d| d.hover(|s| s.text_color(theme::text())))
                .child(icon(icon_path, 18., ink))
                .when(collapsed, |d| d.tooltip(theme::text_tooltip(label)))
                .when(!collapsed, |d| d.child(label))
                .on_click(on_click)
        };
        let tab_item = |id, label, icon_path, tab: Tab, cx: &mut Context<Self>| {
            item(
                id,
                label,
                icon_path,
                matches!(self.page, Page::Browse(t) if t == tab),
                Box::new(cx.listener(move |this, _, _, cx| this.set_tab(tab, cx))),
            )
        };

        let toggle = div()
            .id("sidebar-toggle")
            .tab_index(0)
            .focus_visible(theme::focus_ring)
            .size(px(30.))
            .flex_shrink_0()
            .rounded(px(8.))
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .hover(|s| s.bg(rgba(0xffffff0f)))
            // Same size as the navigation icons below it.
            .child(icon("icons/sidebar.svg", 18., theme::text_tertiary()))
            .tooltip(theme::text_tooltip(if collapsed {
                "Expand sidebar  Ctrl+B"
            } else {
                "Collapse sidebar  Ctrl+B"
            }))
            .on_click(cx.listener(|this, _, _, cx| this.toggle_sidebar(cx)));

        let target = self.sidebar_width();
        let from = if collapsed {
            SIDEBAR_W
        } else {
            SIDEBAR_COLLAPSED_W
        };
        // Header: name and collapse toggle. Only the empty space is a window drag handle;
        // marking the whole header as one swallowed clicks on the toggle.
        let drag = || {
            div()
                .flex_1()
                .h_full()
                .window_control_area(WindowControlArea::Drag)
        };
        let header = div()
            .h(px(NAV_HEIGHT))
            .flex_shrink_0()
            .flex()
            .items_center();
        let header = if collapsed {
            header.child(drag()).child(toggle).child(drag())
        } else {
            header
                .pl_6()
                .pr_3()
                .child(
                    div()
                        .flex_1()
                        .h_full()
                        .flex()
                        .items_center()
                        .window_control_area(WindowControlArea::Drag)
                        .font_family(theme::FONT_DISPLAY)
                        .text_size(px(19.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .gap(px(10.))
                        .child(icon("icons/enso.svg", 20., theme::text()))
                        .child("miru"),
                )
                .child(toggle)
        };

        let sidebar = div()
            .w(px(target))
            .h_full()
            .flex_shrink_0()
            .overflow_hidden()
            .flex()
            .flex_col()
            .bg(rgba(0x0b0b0fe6))
            .border_r_1()
            .border_color(rgba(0xffffff0a))
            .child(header)
            .child(
                div()
                    .pt_2()
                    .flex()
                    .flex_col()
                    .gap(px(NAV_ITEM_GAP))
                    .child(item(
                        "nav-search",
                        "Search",
                        "icons/search.svg",
                        matches!(self.page, Page::Browse(Tab::Search)),
                        Box::new(cx.listener(|this, _, _, cx| this.open_search(cx))),
                    ))
                    .child(tab_item(
                        "nav-home",
                        "Home",
                        "icons/home.svg",
                        Tab::Home,
                        cx,
                    ))
                    .child(tab_item(
                        "nav-movies",
                        "Movies",
                        "icons/film.svg",
                        Tab::Movies,
                        cx,
                    ))
                    .child(tab_item(
                        "nav-shows",
                        "TV Shows",
                        "icons/tv.svg",
                        Tab::Shows,
                        cx,
                    )),
            )
            .child(div().flex_1())
            // A quiet notice when a new version is waiting; it opens Settings, where it installs.
            .when(self.update.is_available(), |d| {
                d.child(
                    div()
                        .id("nav-update")
                        .tab_index(0)
                        .focus_visible(theme::focus_ring)
                        .mx_3()
                        .mb_1()
                        .h(px(NAV_ITEM_H))
                        .px_3()
                        .flex()
                        .items_center()
                        .when(collapsed, |d| d.justify_center())
                        .gap_3()
                        .rounded(px(8.))
                        .cursor_pointer()
                        .text_sm()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(theme::text())
                        .hover(|s| s.bg(rgba(0xffffff0f)))
                        .child(
                            div()
                                .size(px(18.))
                                .flex()
                                .items_center()
                                .justify_center()
                                .child(div().size(px(8.)).rounded_full().bg(theme::accent())),
                        )
                        .when(collapsed, |d| {
                            d.tooltip(theme::text_tooltip("Update available"))
                        })
                        .when(!collapsed, |d| d.child("Update available"))
                        .on_click(cx.listener(|this, _, _, cx| this.navigate(Page::Settings, cx))),
                )
            })
            .child(div().pb_4().child(item(
                "nav-settings",
                "Settings",
                "icons/settings.svg",
                matches!(self.page, Page::Settings),
                Box::new(cx.listener(|this, _, _, cx| this.navigate(Page::Settings, cx))),
            )));

        // Slide between widths right after a toggle; otherwise render at rest.
        if self.sidebar_changed_at.elapsed() < Duration::from_millis(260) {
            sidebar
                .with_animation(
                    ("sidebar", self.sidebar_epoch),
                    Animation::new(theme::motion(220)).with_easing(ease_out_quint()),
                    move |el, t| el.w(px(from + (target - from) * t)),
                )
                .into_any_element()
        } else {
            sidebar.into_any_element()
        }
    }

    /// Window strip over the content: drag area, background-task status and window controls.
    /// It has no background of its own, so the page shows through as it scrolls.
    pub(super) fn render_top_bar(&self) -> impl IntoElement {
        let status: Option<(SharedString, bool)> = match &self.sync {
            SyncStatus::Idle => None,
            SyncStatus::Scanning => Some(("Scanning…".into(), false)),
            SyncStatus::FetchingMetadata => Some(("Updating artwork…".into(), false)),
            SyncStatus::Failed(msg) => Some((msg.clone(), true)),
        };
        div()
            .absolute()
            .top_0()
            .left_0()
            .right_0()
            .h(px(NAV_HEIGHT))
            .flex()
            .items_start()
            .child(
                div()
                    .flex_1()
                    .h(px(NAV_HEIGHT))
                    .window_control_area(WindowControlArea::Drag),
            )
            .children(status.map(|(text, failed)| {
                div()
                    .mt_2()
                    .h(px(28.))
                    .px_3()
                    .mr_3()
                    .flex()
                    .items_center()
                    .max_w(px(360.))
                    .rounded_full()
                    .bg(rgba(0x0c0c12b3))
                    .text_xs()
                    .text_color(if failed {
                        rgba(0xff9b9bff)
                    } else {
                        theme::text_secondary()
                    })
                    .child(div().truncate().child(text))
            }))
            .child(
                // A soft pill behind the window controls keeps them legible over bright art.
                div()
                    .rounded_bl(px(12.))
                    .bg(rgba(0x0c0c1280))
                    .child(theme::window_controls()),
            )
    }

    pub(super) fn render_empty(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap_3()
            .child(
                div()
                    .font_family(theme::FONT_DISPLAY)
                    .text_size(px(40.))
                    .font_weight(FontWeight::BOLD)
                    .child("Your library awaits"),
            )
            .child(
                div()
                    .text_color(theme::text_secondary())
                    .child("Add a folder of movies or TV shows to get started."),
            )
            .child(div().mt_4().child(primary_button(
                "empty-add",
                Some("icons/plus.svg"),
                "Add folder",
                cx.listener(Self::add_folders),
            )))
    }
}
