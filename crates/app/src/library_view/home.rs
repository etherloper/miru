//! Home page: rotating hero, Continue Watching and the title shelves.

use super::*;
use hero::HeroView;

impl LibraryView {
    // ---- home ------------------------------------------------------------------------------

    pub(super) fn render_home(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let viewport_h = f32::from(window.viewport_size().height);
        let hero_h = (viewport_h * 0.74).max(520.);

        div()
            .id("home")
            .size_full()
            .overflow_y_scroll()
            .track_scroll(&self.home_scroll)
            .child(self.hero_element(hero_h, cx))
            .child(
                // Shelves rise into the bottom of the hero for a layered, cinematic feel.
                div()
                    .relative()
                    .mt(px(-96.))
                    .flex()
                    .flex_col()
                    .gap_8()
                    .pb_16()
                    .child(self.render_continue_shelf(cx))
                    .child(self.render_shelf("shelf-recent", "Recently Added", &self.recent, cx))
                    .child(self.render_shelf("shelf-movies", "Movies", &self.movies, cx))
                    .child(self.render_shelf("shelf-shows", "TV Shows", &self.shows, cx)),
            )
    }

    /// The hero's contents, drawn by `HeroView` (which supplies `cx` for its listeners;
    /// they forward to the library).
    pub(super) fn hero_contents(&self, height: f32, cx: &Context<HeroView>) -> AnyElement {
        let Some(item) = self.featured.get(self.hero_index).cloned() else {
            return div().h(px(NAV_HEIGHT + 120.)).into_any_element();
        };
        let meta = item.metadata.clone().unwrap_or_default();
        let fade = |id: &str, epoch: usize| {
            (
                ElementId::from((SharedString::from(id.to_string()), epoch)),
                Animation::new(theme::motion(HERO_FADE.as_millis() as u64))
                    .with_easing(ease_in_out),
            )
        };

        // Only crossfade a change that just happened. Animation state is lost while the player
        // is in front, so without this check the fade would replay on returning to Home.
        let fresh = self.hero_epoch > 0 && self.hero_changed_at.elapsed() < HERO_FADE;
        let mut stage = div().absolute().inset_0();
        if fresh {
            if let Some(previous) = self.hero_previous.and_then(|i| self.featured.get(i)) {
                stage = stage.child(hero_art(previous));
            }
            let (id, animation) = fade("hero-art", self.hero_epoch);
            stage = stage.child(
                div()
                    .absolute()
                    .inset_0()
                    .child(hero_art(&item))
                    .with_animation(id, animation, |el, t| el.opacity(t)),
            );
        } else {
            stage = stage.child(div().absolute().inset_0().child(hero_art(&item)));
        }

        let subtitle = match item.key.kind {
            ItemKind::Movie => {
                let mut parts = vec!["Movie".to_string()];
                if let Some(year) = item.key.year {
                    parts.push(year.to_string());
                }
                parts.join("  ·  ")
            }
            ItemKind::Show => format!("TV Show  ·  {} episodes", item.file_count),
        };
        let (play_item, info_item) = (item.clone(), item.clone());
        let primary = match item.key.kind {
            ItemKind::Movie => primary_button(
                "hero-play",
                Some("icons/play.svg"),
                "Play",
                cx.listener(move |hero: &mut HeroView, _, _, cx| {
                    hero.on_library(cx, |this, cx| this.play_movie(&play_item, true, cx))
                }),
            ),
            ItemKind::Show => primary_button(
                "hero-play",
                Some("icons/play.svg"),
                "Watch",
                cx.listener(move |hero: &mut HeroView, _, _, cx| {
                    hero.on_library(cx, |this, cx| this.open(&play_item, cx))
                }),
            ),
        };

        let (copy_id, copy_animation) = fade("hero-copy", self.hero_epoch);
        let copy = div()
            .absolute()
            .left(px(PAGE_PADDING))
            .bottom(px(150.))
            .w(px(600.))
            .flex()
            .flex_col()
            .gap_3()
            .child(self.title_art(&item, &meta, 130., 480., 56.))
            .child(
                div()
                    .text_sm()
                    .text_color(theme::text_secondary())
                    .child(subtitle),
            )
            .children(meta.overview.clone().map(|o| {
                div()
                    .mt_1()
                    .max_w(px(540.))
                    .line_clamp(3)
                    .text_color(rgba(0xe8e8edff))
                    .child(o)
            }))
            .child(
                div()
                    .mt_4()
                    .flex()
                    .gap_3()
                    .child(primary)
                    .child(glass_button(
                        "hero-info",
                        Some("icons/info.svg"),
                        "Details",
                        cx.listener(move |hero: &mut HeroView, _, _, cx| {
                            hero.on_library(cx, |this, cx| this.open(&info_item, cx))
                        }),
                    )),
            )
            .with_animation(copy_id, copy_animation, move |el, t| {
                el.opacity(if fresh { t } else { 1. })
            });

        // Page dots, centred along the bottom of the banner (above the shelves that overlap it).
        let dots = (self.featured.len() > 1).then(|| {
            div()
                .absolute()
                .left_0()
                .right_0()
                .bottom(px(HERO_DOTS_BOTTOM))
                .flex()
                .justify_center()
                .items_center()
                .gap_2()
                .children((0..self.featured.len()).map(|i| {
                    let active = i == self.hero_index;
                    div()
                        .id(("hero-dot", i))
                        .h(px(8.))
                        .w(px(if active { 24. } else { 8. }))
                        .rounded_full()
                        .cursor_pointer()
                        .bg(if active {
                            theme::text()
                        } else {
                            rgba(0xffffff59)
                        })
                        .hover(|s| s.bg(theme::text()))
                        .on_click(cx.listener(move |hero: &mut HeroView, _, _, cx| {
                            hero.on_library(cx, |this, cx| this.show_hero(i, cx))
                        }))
                }))
        });

        // Previous / next arrows in the page margins, shown while the banner is hovered.
        let hovering = self.hovered.as_ref().is_some_and(|h| h.starts_with("hero"));
        let arrow = |side: &'static str, icon_path: &'static str, direction: isize| {
            let group = SharedString::from(format!("hero-{side}-arrow"));
            div()
                .id(SharedString::from(format!("hero-{side}")))
                .group(group.clone())
                .absolute()
                .top(px(NAV_HEIGHT))
                .bottom(px(HERO_DOTS_BOTTOM))
                .w(px(PAGE_PADDING - 4.))
                .when(direction < 0, |d| d.left(px(2.)))
                .when(direction > 0, |d| d.right(px(2.)))
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .child(
                    div()
                        .size(px(ARROW_BOX))
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(
                            svg()
                                .path(icon_path)
                                .size(px(30.))
                                .flex_shrink_0()
                                .text_color(theme::text_secondary())
                                .group_hover(group, |s| s.text_color(theme::text()).size(px(36.))),
                        ),
                )
                .on_click(cx.listener(move |hero: &mut HeroView, _, _, cx| {
                    cx.stop_propagation();
                    hero.on_library(cx, |this, cx| this.step_hero(direction, cx));
                }))
        };
        let arrows = (hovering && self.featured.len() > 1).then(|| {
            [
                arrow("prev", "icons/chevron-left.svg", -1),
                arrow("next", "icons/chevron-right.svg", 1),
            ]
        });

        let hover_id = SharedString::from("hero");
        div()
            // Drawn by `HeroView`, outside the library view's tree: name the image cache.
            .image_cache(self.image_cache.clone())
            .id("hero")
            .relative()
            .w_full()
            .h(px(height))
            .overflow_hidden()
            .bg(theme::placeholder(&item.key.title, meta.accent_rgb))
            .on_hover(
                cx.listener(move |hero: &mut HeroView, hovered: &bool, _, cx| {
                    let (id, hovered) = (hover_id.clone(), *hovered);
                    hero.on_library(cx, |this, cx| this.set_hover(&id, hovered, cx))
                }),
            )
            .child(stage)
            // Scrims: left for legibility, bottom to melt into the page.
            .child(div().absolute().inset_0().bg(linear_gradient(
                90.,
                linear_color_stop(rgba(0x08080be6), 0.),
                linear_color_stop(rgba(0x08080b00), 0.65),
            )))
            // Bottom fade into the page, eased by layering a short solid fade and a long soft one
            // (a single two-stop fade leaves a line where it turns solid).
            .child(div().absolute().inset_0().bg(linear_gradient(
                0.,
                linear_color_stop(theme::bg(), 0.),
                linear_color_stop(rgba(0x08080b00), 0.25),
            )))
            .child(div().absolute().inset_0().bg(linear_gradient(
                0.,
                linear_color_stop(rgba(0x08080bb3), 0.),
                linear_color_stop(rgba(0x08080b00), 0.55),
            )))
            .child(copy)
            .children(dots)
            .children(arrows.into_iter().flatten())
            .into_any_element()
    }

    /// A titled horizontal row of cards with an optional "See All" link to the full grid.
    pub(super) fn shelf(
        &self,
        heading: &'static str,
        id: &'static str,
        cards: Vec<AnyElement>,
        see_all: Option<Tab>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if cards.is_empty() {
            return theme::nothing();
        }
        div()
            .flex()
            .flex_col()
            .flex_shrink_0()
            .child(
                div()
                    .px(px(PAGE_PADDING))
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .font_family(theme::FONT_DISPLAY)
                            .text_size(px(22.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(heading),
                    )
                    .children(see_all.map(|tab| {
                        div()
                            .id(SharedString::from(format!("{id}-see-all")))
                            .tab_index(0)
                            .focus_visible(theme::focus_ring)
                            .flex()
                            .items_center()
                            .gap_1()
                            .text_sm()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(theme::text_secondary())
                            .cursor_pointer()
                            .hover(|s| s.text_color(theme::text()))
                            .child("See All")
                            .child(icon(
                                "icons/chevron-right.svg",
                                14.,
                                theme::text_secondary(),
                            ))
                            .on_click(cx.listener(move |this, _, _, cx| this.set_tab(tab, cx)))
                    })),
            )
            .child(self.shelf_row(id, cards, cx))
            .into_any_element()
    }

    /// A horizontally scrolling row of cards with edge fades and previous/next buttons that
    /// appear while the row (or a button) is hovered and glide the row by most of its width.
    pub(super) fn shelf_row(
        &self,
        id: &'static str,
        cards: Vec<AnyElement>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let scroll = self.shelf_scrolls.get(id).cloned().unwrap_or_default();
        let offset = -f32::from(scroll.offset().x);
        let max = f32::from(scroll.max_offset().x);
        let (can_left, can_right) = (offset > 1., offset < max - 1.);
        let show_arrows = self.hovered_shelf == Some(id) || self.hovered_arrow == Some(id);

        let arrow_top = SHELF_PAD_TOP + shelf_art_height(id) / 2. - ARROW_BOX / 2.;
        // Edge zones the full height of the row: a soft scrim that deepens into the page colour
        // at the window edge, with a bare chevron that brightens and grows on hover.
        let chevron = |side: &'static str,
                       icon_path: &'static str,
                       direction: f32,
                       cx: &mut Context<Self>| {
            let group = SharedString::from(format!("{id}-{side}-arrow"));
            // Gradient angles point away from the window edge.
            let angle = if direction < 0. { 90. } else { 270. };
            let scrim = move |edge: u32| {
                linear_gradient(
                    angle,
                    linear_color_stop(rgba(edge), 0.),
                    linear_color_stop(rgba(0x08080b00), 1.),
                )
            };
            div()
                .id(SharedString::from(format!("{id}-{side}")))
                .group(group.clone())
                .absolute()
                .top_0()
                .bottom_0()
                .w(px(SHELF_ARROW_W))
                .when(direction < 0., |d| d.left_0().justify_start().pl(px(10.)))
                .when(direction > 0., |d| d.right_0().justify_end().pr(px(10.)))
                .flex()
                // Pinned to the middle of the artwork, not the row: the row grows when a hovered
                // card reveals more text, and a centred chevron would jump with it.
                .items_start()
                .pt(px(arrow_top))
                .bg(scrim(0x08080bf2))
                .hover(move |s| s.bg(scrim(0x08080bff)))
                .cursor_pointer()
                .child(
                    // A fixed box, so the chevron grows about its centre on hover.
                    div()
                        .size(px(ARROW_BOX))
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(
                            svg()
                                .path(icon_path)
                                .size(px(30.))
                                .flex_shrink_0()
                                .text_color(theme::text_secondary())
                                .group_hover(group, |s| s.text_color(theme::text()).size(px(36.))),
                        ),
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.glide_shelf(id, direction, cx);
                    cx.stop_propagation();
                }))
                // Without this the click also reaches the card underneath and opens it.
                .block_mouse_except_scroll()
                .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                    let next = hovered.then_some(id);
                    if this.hovered_arrow != next {
                        this.hovered_arrow = next;
                        cx.notify();
                    }
                }))
        };
        let fade = |angle: f32| {
            div()
                .absolute()
                .top_0()
                .bottom_0()
                .w(px(PAGE_PADDING))
                .bg(linear_gradient(
                    angle,
                    linear_color_stop(theme::bg(), 0.),
                    linear_color_stop(rgba(0x08080b00), 1.),
                ))
        };

        div()
            .id(SharedString::from(format!("{id}-row")))
            .relative()
            .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                if *hovered {
                    if this.hovered_shelf != Some(id) {
                        this.hovered_shelf = Some(id);
                        cx.notify();
                    }
                } else if this.hovered_shelf == Some(id) {
                    this.hovered_shelf = None;
                    cx.notify();
                }
            }))
            .child(
                div()
                    .id(id)
                    .flex()
                    .gap(px(SHELF_GAP))
                    .px(px(PAGE_PADDING))
                    // Room for the hover lift and shadow inside the scroll clip.
                    .pt(px(SHELF_PAD_TOP))
                    .pb(px(SHELF_PAD_BOTTOM))
                    .overflow_x_scroll()
                    .track_scroll(&scroll)
                    // Vertical wheel movement scrolls the page, not the row.
                    .restrict_scroll_to_axis()
                    .children(cards),
            )
            .when(can_left, |d| d.child(fade(90.).left_0()))
            .when(can_right, |d| d.child(fade(270.).right_0()))
            .when(show_arrows && can_left, |d| {
                d.child(chevron("prev", "icons/chevron-left.svg", -1., cx))
            })
            .when(show_arrows && can_right, |d| {
                d.child(chevron("next", "icons/chevron-right.svg", 1., cx))
            })
            .into_any_element()
    }

    /// Scrolls a shelf by most of its visible width. The glide is advanced in `render`, once
    /// per frame, so it moves in step with the display.
    pub(super) fn glide_shelf(&mut self, id: &'static str, direction: f32, cx: &mut Context<Self>) {
        let Some(scroll) = self.shelf_scrolls.get(id).cloned() else {
            return;
        };
        let max = f32::from(scroll.max_offset().x);
        // Pressing again mid-glide continues from where this glide is heading.
        let from = -f32::from(scroll.offset().x);
        let base = match &self.shelf_glide {
            Some(g) if g.id == id => g.to,
            _ => from,
        };
        let width = f32::from(scroll.bounds().size.width) * 0.8;
        let to = (base + direction * width).clamp(0., max);
        self.shelf_glide = Some(ShelfGlide {
            id,
            scroll,
            from,
            to,
            last: from,
            started: Instant::now(),
            duration: theme::motion(560).as_secs_f32(),
        });
        cx.notify();
    }

    /// Moves the active shelf glide to this frame's position.
    pub(super) fn step_shelf_glide(&mut self, window: &mut Window) {
        let Some(g) = &mut self.shelf_glide else {
            return;
        };
        let current = -f32::from(g.scroll.offset().x);
        // The wheel or a drag moved the row meanwhile: let that win.
        if (current - g.last).abs() > 1. {
            self.shelf_glide = None;
            return;
        }
        let t = if g.duration <= 0. {
            1.
        } else {
            (g.started.elapsed().as_secs_f32() / g.duration).min(1.)
        };
        // Ease-out quint: quick to respond, long soft landing.
        let eased = 1. - (1. - t).powi(5);
        let x = g.from + (g.to - g.from) * eased;
        g.scroll.set_offset(gpui::point(px(-x), px(0.)));
        g.last = x;
        if t < 1. {
            window.request_animation_frame();
        } else {
            self.shelf_glide = None;
        }
    }

    pub(super) fn render_continue_shelf(&self, cx: &mut Context<Self>) -> AnyElement {
        let cards = self
            .continue_cards
            .iter()
            .map(|card| self.continue_card(card, cx))
            .collect();
        self.shelf("Continue Watching", "shelf-continue", cards, None, cx)
    }

    pub(super) fn render_shelf(
        &self,
        id: &'static str,
        heading: &'static str,
        items: &[LibraryItem],
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let cards = items
            .iter()
            // Every card is laid out (and its poster decoded) even off screen, so a shelf
            // shows a sample; See All opens the full grid.
            .take(SHELF_LIMIT)
            .map(|item| self.poster_card(id, item, POSTER_W, cx))
            .collect();
        let see_all = match id {
            "shelf-movies" => Some(Tab::Movies),
            "shelf-shows" => Some(Tab::Shows),
            _ => None,
        };
        self.shelf(heading, id, cards, see_all, cx)
    }
}
