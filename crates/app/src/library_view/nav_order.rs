//! Settings → Sidebar: drag the destinations into the order you want. The row follows the
//! pointer and the others slide out of its way; Alt+Up/Down moves a focused row.

use miru_core::settings::NavItem;

use super::*;

/// Height of each reorderable row.
const ROW_H: f32 = 52.;
/// Time constant for rows sliding into place.
const SLIDE_SECS: f32 = 0.07;

pub(super) struct NavDrag {
    from: usize,
    start_y: f32,
    pointer_y: f32,
}

impl NavDrag {
    fn dy(&self) -> f32 {
        self.pointer_y - self.start_y
    }

    /// The slot the dragged row would drop into.
    fn target(&self, len: usize) -> usize {
        let moved = (self.dy() / ROW_H).round() as isize;
        (self.from as isize + moved).clamp(0, len as isize - 1) as usize
    }
}

impl LibraryView {
    fn start_nav_drag(&mut self, from: usize, y: f32, cx: &mut Context<Self>) {
        self.nav_drag = Some(NavDrag {
            from,
            start_y: y,
            pointer_y: y,
        });
        self.nav_frame = Instant::now();
        cx.notify();
    }

    pub(super) fn drag_nav(&mut self, event: &gpui::MouseMoveEvent, cx: &mut Context<Self>) {
        let Some(drag) = &mut self.nav_drag else {
            return;
        };
        if event.pressed_button != Some(gpui::MouseButton::Left) {
            self.end_nav_drag(cx);
            return;
        }
        drag.pointer_y = f32::from(event.position.y);
        cx.notify();
    }

    pub(super) fn end_nav_drag(&mut self, cx: &mut Context<Self>) {
        let Some(drag) = self.nav_drag.take() else {
            return;
        };
        let len = self.settings.sidebar_order.len();
        let target = drag.target(len);
        if target != drag.from {
            // The row lands from where it was let go, the others from where they slid to.
            let mut shift = vec![0.; len];
            shift[target] = (drag.from as f32 - target as f32) * ROW_H + drag.dy();
            self.nav_shift = shift;
            self.update_settings(
                |s| {
                    let item = s.sidebar_order.remove(drag.from);
                    s.sidebar_order.insert(target, item);
                },
                cx,
            );
        } else {
            let mut shift = vec![0.; len];
            if let Some(s) = shift.get_mut(drag.from) {
                *s = drag.dy();
            }
            self.nav_shift = shift;
        }
        self.nav_frame = Instant::now();
        cx.notify();
    }

    fn move_nav_item(&mut self, ix: usize, up: bool, cx: &mut Context<Self>) {
        let len = self.settings.sidebar_order.len();
        let target = if up { ix.checked_sub(1) } else { Some(ix + 1) };
        if let Some(target) = target.filter(|t| *t < len && ix < len) {
            let mut shift = vec![0.; len];
            shift[target] = (ix as f32 - target as f32) * ROW_H;
            shift[ix] = (target as f32 - ix as f32) * ROW_H;
            self.nav_shift = shift;
            self.nav_frame = Instant::now();
            self.update_settings(|s| s.sidebar_order.swap(ix, target), cx);
        }
    }

    /// Eases each row's offset toward where it should be this frame (called from `render`).
    pub(super) fn step_nav_order(&mut self, window: &mut Window) {
        let len = self.settings.sidebar_order.len();
        let goals: Vec<f32> = (0..len)
            .map(|i| match &self.nav_drag {
                Some(drag) => {
                    let target = drag.target(len);
                    if i == drag.from {
                        drag.dy()
                    } else if drag.from < target && i > drag.from && i <= target {
                        -ROW_H
                    } else if target < drag.from && i >= target && i < drag.from {
                        ROW_H
                    } else {
                        0.
                    }
                }
                None => 0.,
            })
            .collect();
        self.nav_shift.resize(len, 0.);
        let now = Instant::now();
        let dt = now.duration_since(self.nav_frame).as_secs_f32().min(0.1);
        self.nav_frame = now;
        let ease = if theme::motion_enabled() {
            1. - (-dt / SLIDE_SECS).exp()
        } else {
            1.
        };
        let mut moving = false;
        for (i, goal) in goals.iter().enumerate() {
            let dragged = self.nav_drag.as_ref().is_some_and(|d| d.from == i);
            let shift = &mut self.nav_shift[i];
            if dragged {
                *shift = *goal;
            } else {
                *shift += (goal - *shift) * ease;
                if (goal - *shift).abs() < 0.3 {
                    *shift = *goal;
                } else {
                    moving = true;
                }
            }
        }
        if moving {
            window.request_animation_frame();
        }
    }

    /// The reorderable list of sidebar destinations.
    pub(super) fn sidebar_order_rows(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let order = &self.settings.sidebar_order;
        let dragging = self.nav_drag.as_ref().map(|d| d.from);
        let row = |ix: usize, item: NavItem, lifted: bool, cx: &mut Context<Self>| {
            let icon_path = match item {
                NavItem::Search => "icons/search.svg",
                NavItem::Home => "icons/home.svg",
                NavItem::Movies => "icons/film.svg",
                NavItem::Shows => "icons/tv.svg",
            };
            div()
                .id(("nav-order", ix + if lifted { 100 } else { 0 }))
                .h(px(ROW_H))
                .px_5()
                .flex()
                .items_center()
                .gap_4()
                .border_b_1()
                .border_color(theme::glass_border())
                .cursor_grab()
                .when(lifted, |d| {
                    d.bg(rgba(0x1c1c22f2))
                        .rounded(px(10.))
                        .border_1()
                        .border_color(rgba(0xffffff26))
                        .shadow(theme::card_shadow(1.))
                })
                .child(icon(icon_path, 18., theme::text_secondary()))
                .child(
                    div()
                        .flex_1()
                        .font_weight(FontWeight::MEDIUM)
                        .child(item.label()),
                )
                .child(icon("icons/grip.svg", 16., theme::text_tertiary()))
                .when(!lifted, |d| {
                    d.tab_index(0)
                        .focus_visible(theme::focus_ring)
                        .tooltip(theme::text_tooltip("Drag to reorder  Alt+↑ ↓"))
                        .on_mouse_down(
                            gpui::MouseButton::Left,
                            cx.listener(move |this, event: &gpui::MouseDownEvent, _, cx| {
                                this.start_nav_drag(ix, f32::from(event.position.y), cx);
                                cx.stop_propagation();
                            }),
                        )
                        .on_key_down(cx.listener(move |this, event: &KeyDownEvent, _, cx| {
                            let k = &event.keystroke;
                            if k.modifiers.alt && (k.key == "up" || k.key == "down") {
                                this.move_nav_item(ix, k.key == "up", cx);
                                cx.stop_propagation();
                            }
                        }))
                })
        };
        let rows = order.iter().enumerate().map(|(ix, item)| {
            let shift = self.nav_shift.get(ix).copied().unwrap_or(0.);
            row(ix, *item, false, cx)
                .relative()
                .top(px(if dragging == Some(ix) { 0. } else { shift }))
                // The dragged row's place stays empty; its lifted copy is drawn on top.
                .when(dragging == Some(ix), |d| d.opacity(0.))
                .into_any_element()
        });
        let rows: Vec<AnyElement> = rows.collect();
        let lifted = dragging.and_then(|from| {
            let item = *order.get(from)?;
            let top = from as f32 * ROW_H + self.nav_shift.get(from).copied().unwrap_or(0.);
            Some(
                div()
                    .absolute()
                    .left_0()
                    .right_0()
                    .top(px(top))
                    .child(row(from, item, true, cx)),
            )
        });
        vec![
            div()
                .relative()
                .flex()
                .flex_col()
                .children(rows)
                .children(lifted)
                .into_any_element(),
        ]
    }
}
