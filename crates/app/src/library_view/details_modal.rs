//! "Read more" on a detail page: a dialog with the title's full synopsis and details, for
//! synopses too long for the three lines the page shows.

use super::*;

const PANEL_W: f32 = 640.;

pub(super) struct DetailsModal {
    item: LibraryItem,
    /// Year, runtime and so on, as the page shows them.
    facts: String,
}

impl LibraryView {
    pub(super) fn open_details(
        &mut self,
        item: LibraryItem,
        facts: String,
        cx: &mut Context<Self>,
    ) {
        self.details_scroll.reset();
        self.details_modal = Some(DetailsModal { item, facts });
        cx.notify();
    }

    pub(super) fn close_details(&mut self, cx: &mut Context<Self>) {
        if self.details_modal.take().is_some() {
            cx.notify();
        }
    }

    pub(super) fn render_details_modal(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(modal) = &self.details_modal else {
            return theme::nothing();
        };
        let Some(meta) = modal.item.metadata.as_ref() else {
            return theme::nothing();
        };
        let section = |label: &'static str, content: AnyElement| {
            div()
                .flex()
                .flex_col()
                .gap_1p5()
                .child(
                    div()
                        .text_xs()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(theme::text_tertiary())
                        .child(label),
                )
                .child(content)
        };
        let text = |s: String| {
            div()
                .text_sm()
                .line_height(px(22.))
                .text_color(rgba(0xe8e8edff))
                .child(s)
                .into_any_element()
        };

        let header = div()
            .flex_shrink_0()
            .px_6()
            .pt_5()
            .pb_4()
            .flex()
            .items_start()
            .gap_3()
            .border_b_1()
            .border_color(rgba(0xffffff1f))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .font_family(theme::FONT_DISPLAY)
                            .text_size(px(22.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(modal.item.display_title().to_string()),
                    )
                    .when(!modal.facts.is_empty(), |d| {
                        d.child(
                            div()
                                .text_sm()
                                .text_color(theme::text_secondary())
                                .child(modal.facts.clone()),
                        )
                    }),
            )
            .child(
                round_button(
                    "details-close",
                    "icons/window-close.svg",
                    32.,
                    cx.listener(|this, _, _, cx| this.close_details(cx)),
                )
                .tooltip(theme::text_tooltip("Close  Esc")),
            );

        let ratings = self.rating_badges(&modal.item.key);
        let body = div()
            .id("details-body")
            .relative()
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .track_scroll(&self.details_scroll.handle)
            .px_6()
            .py_5()
            .flex()
            .flex_col()
            .gap_5()
            .when(!ratings.is_empty(), |d| {
                d.child(
                    div()
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .gap_x_4()
                        .gap_y_2()
                        .children(ratings),
                )
            })
            .children(meta.overview.clone().map(|o| {
                div()
                    .line_height(px(24.))
                    .text_color(rgba(0xe8e8edff))
                    .child(o)
            }))
            .when(!meta.genres.is_empty(), |d| {
                d.child(section("Genres", text(meta.genres.join(", "))))
            })
            .when(!meta.cast.is_empty(), |d| {
                d.child(section("Starring", text(meta.cast.join(", "))))
            })
            .child(self.details_scroll.scrollbar(cx.entity_id()));

        let max_h = f32::from(window.viewport_size().height) * 0.8;
        let panel = div()
            .id("details-panel")
            .occlude()
            .w(px(PANEL_W))
            .max_h(px(max_h))
            .rounded(px(16.))
            .overflow_hidden()
            // True black with a light outline, like the other dialogs.
            .bg(rgba(0x000000ff))
            .border_1()
            .border_color(rgba(0xffffff38))
            .shadow(theme::frost_shadow())
            .flex()
            .flex_col()
            .child(header)
            .child(body);

        div()
            .id("details-overlay")
            .absolute()
            .inset_0()
            .bg(rgba(0x00000099))
            .flex()
            .justify_center()
            .items_center()
            .on_mouse_down(
                gpui::MouseButton::Left,
                cx.listener(|this, _, _, cx| this.close_details(cx)),
            )
            .child(panel)
            .with_animation(
                "details-overlay-in",
                Animation::new(theme::motion(160)).with_easing(ease_out_quint()),
                |el, t| el.opacity(t),
            )
            .into_any_element()
    }
}
