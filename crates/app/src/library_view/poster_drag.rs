//! Dragging a poster onto a collection in the sidebar.
//!
//! Pressing a poster and moving it picks it up: in grids straight away, on Home's rows by
//! moving up or down or by holding still for a moment (a sideways swipe scrolls the row). The
//! poster then shrinks into a small card that follows the pointer with a slight lag. Over a
//! collection it tightens and names the target; letting go there drops it in with a glow on
//! the collection. Letting go anywhere else lets it fade away.

use gpui::{Pixels, Point};

use super::*;

/// Width of the poster while it is carried.
const CARRY_W: f32 = 96.;
/// Narrower over a drop target, as if about to drop in.
const CARRY_TARGET_W: f32 = 80.;
/// The carried poster's follow lag (time constant of its easing toward the pointer).
const FOLLOW_SECS: f32 = 0.055;
/// Size easing (lift and over-target).
const SIZE_SECS: f32 = 0.09;
/// Holding a poster on a row this long picks it up.
const HOLD_TO_LIFT: Duration = Duration::from_millis(320);
/// Pointer travel that picks up a poster in a grid, or a vertical move on a row.
const LIFT_DISTANCE: f32 = 8.;
/// How long the drop (or cancel) animation runs.
const LANDING: Duration = Duration::from_millis(340);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DropTarget {
    Collection(i64),
    NewCollection,
}

/// What the carried poster shows.
#[derive(Clone)]
pub(super) struct CarryArt {
    pub poster: Option<String>,
    pub title: SharedString,
    pub accent: Option<u32>,
}

pub(super) struct PosterDrag {
    key: TitleKey,
    art: CarryArt,
    /// Pressed on a horizontally scrolling row (where sideways movement scrolls instead).
    on_row: bool,
    press: Point<Pixels>,
    pointer: Point<Pixels>,
    pub lifted: bool,
    /// Eased pointer position and carried width, advanced once per frame.
    shown: (f32, f32),
    width: f32,
    last_frame: Instant,
}

/// The carried poster after release: dropping into a collection, or fading away.
pub(super) struct Landing {
    art: CarryArt,
    at: (f32, f32),
    width: f32,
    into: bool,
    started: Instant,
    epoch: usize,
}

impl LibraryView {
    /// A poster was pressed: get ready to pick it up if the pointer moves (or holds).
    pub(super) fn press_poster(
        &mut self,
        key: TitleKey,
        art: CarryArt,
        scope: &'static str,
        position: Point<Pixels>,
        card_width: f32,
        cx: &mut Context<Self>,
    ) {
        self.close_card_menu();
        let on_row = scope.starts_with("shelf");
        let x = (f32::from(position.x), f32::from(position.y));
        self.poster_drag = Some(PosterDrag {
            key,
            art,
            on_row,
            press: position,
            pointer: position,
            lifted: false,
            shown: x,
            width: card_width,
            last_frame: Instant::now(),
        });
        self.poster_press_epoch += 1;
        if on_row {
            // Holding still picks it up.
            let epoch = self.poster_press_epoch;
            cx.spawn(async move |this, cx| {
                cx.background_executor().timer(HOLD_TO_LIFT).await;
                this.update(cx, |view, cx| {
                    let still = view.poster_press_epoch == epoch
                        && !view.shelf_drag.as_ref().is_some_and(|d| d.moved)
                        && view.poster_drag.as_ref().is_some_and(|d| !d.lifted);
                    if still {
                        view.lift_poster(cx);
                    }
                })
                .ok();
            })
            .detach();
        }
    }

    fn lift_poster(&mut self, cx: &mut Context<Self>) {
        let Some(drag) = &mut self.poster_drag else {
            return;
        };
        drag.lifted = true;
        drag.last_frame = Instant::now();
        // The row stays where it is while the poster is carried.
        self.shelf_drag = None;
        self.shelf_glide = None;
        self.drop_target = None;
        cx.notify();
    }

    /// Follows the pointer: picks the poster up once the press turns into a drag.
    pub(super) fn drag_poster(&mut self, event: &gpui::MouseMoveEvent, cx: &mut Context<Self>) {
        let Some(drag) = &mut self.poster_drag else {
            return;
        };
        if event.pressed_button != Some(gpui::MouseButton::Left) {
            self.end_poster_drag(cx);
            return;
        }
        drag.pointer = event.position;
        if drag.lifted {
            cx.notify();
            return;
        }
        let dx = f32::from(event.position.x - drag.press.x);
        let dy = f32::from(event.position.y - drag.press.y);
        let distance = dx.hypot(dy);
        let lift = if drag.on_row {
            // Sideways is the row's own drag; up or down picks the poster up.
            if self.shelf_drag.as_ref().is_some_and(|d| d.moved) {
                self.poster_drag = None;
                return;
            }
            distance > LIFT_DISTANCE && dy.abs() > dx.abs()
        } else {
            distance > LIFT_DISTANCE
        };
        if lift {
            self.lift_poster(cx);
        }
    }

    /// Release: drop into the collection under the pointer, if any.
    pub(super) fn end_poster_drag(&mut self, cx: &mut Context<Self>) {
        let Some(drag) = self.poster_drag.take() else {
            return;
        };
        if !drag.lifted {
            return;
        }
        self.poster_drag_ended = Some(Instant::now());
        let target = self.drop_target.take();
        self.landing_epoch += 1;
        self.landing = Some(Landing {
            art: drag.art,
            at: drag.shown,
            width: drag.width,
            into: target.is_some(),
            started: Instant::now(),
            epoch: self.landing_epoch,
        });
        match target {
            Some(DropTarget::Collection(id)) => self.add_to_collection(id, &drag.key, cx),
            Some(DropTarget::NewCollection) => self.new_collection(Some(drag.key), cx),
            None => {}
        }
        cx.notify();
    }

    pub(super) fn set_drop_target(
        &mut self,
        target: DropTarget,
        hovered: bool,
        cx: &mut Context<Self>,
    ) {
        let before = self.drop_target;
        if hovered {
            self.drop_target = Some(target);
        } else if self.drop_target == Some(target) {
            self.drop_target = None;
        }
        if before != self.drop_target && self.poster_drag.as_ref().is_some_and(|d| d.lifted) {
            cx.notify();
        }
    }

    pub(super) fn poster_dragging(&self) -> bool {
        self.poster_drag.as_ref().is_some_and(|d| d.lifted)
    }

    /// Advances the carried poster's easing for this frame (called from `render`).
    pub(super) fn step_poster_drag(&mut self, window: &mut Window) {
        if self
            .landing
            .as_ref()
            .is_some_and(|l| l.started.elapsed() > LANDING)
        {
            self.landing = None;
        }
        let over_target = self.drop_target.is_some();
        let Some(drag) = &mut self.poster_drag else {
            return;
        };
        if !drag.lifted {
            return;
        }
        let now = Instant::now();
        let dt = now.duration_since(drag.last_frame).as_secs_f32().min(0.1);
        drag.last_frame = now;
        let target = (f32::from(drag.pointer.x), f32::from(drag.pointer.y));
        let target_w = if over_target { CARRY_TARGET_W } else { CARRY_W };
        let (follow, grow) = if theme::motion_enabled() {
            (1. - (-dt / FOLLOW_SECS).exp(), 1. - (-dt / SIZE_SECS).exp())
        } else {
            (1., 1.)
        };
        drag.shown.0 += (target.0 - drag.shown.0) * follow;
        drag.shown.1 += (target.1 - drag.shown.1) * follow;
        drag.width += (target_w - drag.width) * grow;
        let settled = (target.0 - drag.shown.0).abs() < 0.5
            && (target.1 - drag.shown.1).abs() < 0.5
            && (target_w - drag.width).abs() < 0.5;
        if !settled {
            window.request_animation_frame();
        }
    }

    /// The carried poster (or its landing) above everything, plus a grabbing cursor.
    pub(super) fn render_poster_drag(&self) -> AnyElement {
        let carried = self.poster_drag.as_ref().filter(|d| d.lifted).map(|drag| {
            let label = self.drop_target.map(|t| match t {
                DropTarget::Collection(id) => self
                    .collection(id)
                    .map(|c| format!("Add to {}", c.name))
                    .unwrap_or_default(),
                DropTarget::NewCollection => "New collection".to_string(),
            });
            // Its width eases from the card's own size, so picking it up shrinks it smoothly.
            carry_card(&drag.art, drag.shown, drag.width, label).into_any_element()
        });
        let landing = self.landing.as_ref().map(|l| {
            let (art, at, width, into) = (l.art.clone(), l.at, l.width, l.into);
            div()
                .absolute()
                .inset_0()
                .with_animation(
                    ("poster-landing", l.epoch),
                    Animation::new(theme::motion(LANDING.as_millis() as u64))
                        .with_easing(ease_out_quint()),
                    move |el, t| {
                        // Into a collection: shrinks away into it. Elsewhere: fades out.
                        let scale = if into { 1. - 0.75 * t } else { 1. - 0.1 * t };
                        let w = width * scale;
                        el.child(carry_card(&art, at, w, None)).opacity(1. - t)
                    },
                )
                .into_any_element()
        });
        let dragging = carried.is_some();
        div()
            .absolute()
            .inset_0()
            .children(carried)
            .children(landing)
            .when(dragging, |d| {
                d.child(
                    gpui::canvas(
                        |_, _, _| {},
                        |_, _, window, _| {
                            window.set_window_cursor_style(gpui::CursorStyle::ClosedHand)
                        },
                    )
                    .absolute()
                    .size_0(),
                )
            })
            .into_any_element()
    }

    /// Whether a click should be ignored because it ends a poster drag.
    pub(super) fn click_ends_poster_drag(&self) -> bool {
        self.poster_dragging()
            || self
                .poster_drag_ended
                .is_some_and(|t| t.elapsed() < Duration::from_millis(250))
    }
}

/// The small carried poster, centred on `at` (the pointer), with the target's name beneath.
fn carry_card(
    art: &CarryArt,
    at: (f32, f32),
    width: f32,
    label: Option<String>,
) -> impl IntoElement {
    let height = width * 1.5;
    let poster = div()
        .relative()
        .w(px(width))
        .h(px(height))
        .rounded(px(10.))
        .overflow_hidden()
        .bg(theme::placeholder(&art.title, art.accent))
        .border_1()
        .border_color(gpui::Rgba {
            r: 1.,
            g: 1.,
            b: 1.,
            a: 0.2,
        })
        .shadow(theme::card_shadow(1.))
        .child(match &art.poster {
            Some(path) => art_image(path.clone(), ART_POSTER)
                .size_full()
                .object_fit(ObjectFit::Cover)
                .with_fallback(|| div().into_any_element())
                .into_any_element(),
            None => div()
                .size_full()
                .p_2()
                .flex()
                .items_end()
                .text_xs()
                .font_weight(FontWeight::BOLD)
                .text_color(theme::on_image())
                .child(art.title.clone())
                .into_any_element(),
        });
    div()
        .absolute()
        // The pointer sits a third of the way down the poster, centred across it.
        .left(px(at.0 - width / 2.))
        .top(px(at.1 - height * 0.35))
        .flex()
        .flex_col()
        .items_center()
        .gap_2()
        .child(poster)
        .children(label.filter(|l| !l.is_empty()).map(|label| {
            div()
                .px_3()
                .py_1()
                .rounded_full()
                .bg(theme::text())
                .text_color(theme::bg())
                .text_xs()
                .font_weight(FontWeight::SEMIBOLD)
                .whitespace_nowrap()
                .child(label)
        }))
}
