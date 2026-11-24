//! A thin overlay scrollbar for a scrolling page: shows where you are, and its thumb can be
//! dragged or its track clicked to jump.
//!
//! GPUI has no scrollbar of its own. This is drawn as its own element over the page and
//! reads the page's `ScrollHandle` while painting, so it follows the scroll exactly (including
//! the smooth-scroll slices) without the view re-rendering for it.

use std::cell::Cell;
use std::rc::Rc;

use gpui::{
    App, Bounds, CursorStyle, DispatchPhase, Element, ElementId, EntityId, GlobalElementId, Hitbox,
    HitboxBehavior, InspectorElementId, IntoElement, LayoutId, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, Pixels, Point, ScrollHandle, Style, Window, fill, point, px,
    relative, rgba, size,
};

/// Gap between the thumb and the page's right edge.
const EDGE: f32 = 4.;
/// Thumb width at rest and while hovered or dragged.
const WIDTH: f32 = 6.;
const WIDTH_ACTIVE: f32 = 10.;
/// Width of the strip that responds to the pointer.
const HIT_WIDTH: f32 = 16.;
const MIN_THUMB: f32 = 36.;
/// Space kept clear at the ends of the track.
const PAD: f32 = 4.;

/// Drag state kept by the view between frames: while dragging, where in the thumb the
/// pointer took hold of it.
pub type ScrollbarDrag = Rc<Cell<Option<f32>>>;

pub struct Scrollbar {
    handle: ScrollHandle,
    drag: ScrollbarDrag,
    /// Keeps the track below an overlay at the top of the page (the title bar).
    top_inset: f32,
    /// The view to re-render when the scrollbar moves the page.
    view: EntityId,
}

impl Scrollbar {
    /// An overlay filling its (relatively positioned) parent; place it after the page.
    pub fn new(handle: ScrollHandle, drag: ScrollbarDrag, top_inset: f32, view: EntityId) -> Self {
        Self {
            handle,
            drag,
            top_inset,
            view,
        }
    }
}

#[derive(Clone, Copy)]
struct Geometry {
    track_top: f32,
    track_h: f32,
    thumb_top: f32,
    thumb_h: f32,
    right: f32,
    max: f32,
}

impl Geometry {
    fn measure(handle: &ScrollHandle, area: Bounds<Pixels>, top_inset: f32) -> Option<Self> {
        let max = f32::from(handle.max_offset().y);
        let view = handle.bounds();
        let view_h = f32::from(view.size.height);
        if max < 1. || view_h < 1. {
            return None;
        }
        let track_top = f32::from(view.top()).max(f32::from(area.top()) + top_inset) + PAD;
        let track_h = f32::from(view.bottom()) - PAD - track_top;
        if track_h < MIN_THUMB * 2. {
            return None;
        }
        let thumb_h = (track_h * view_h / (view_h + max)).clamp(MIN_THUMB, track_h);
        let progress = (-f32::from(handle.offset().y) / max).clamp(0., 1.);
        Some(Self {
            track_top,
            track_h,
            thumb_top: track_top + (track_h - thumb_h) * progress,
            thumb_h,
            right: f32::from(view.right()) - EDGE,
            max,
        })
    }

    fn hit_bounds(&self) -> Bounds<Pixels> {
        Bounds::new(
            point(px(self.right + EDGE - HIT_WIDTH), px(self.track_top)),
            size(px(HIT_WIDTH), px(self.track_h)),
        )
    }

    /// The scroll offset that puts the thumb's top at `thumb_top`.
    fn offset_for(&self, thumb_top: f32) -> f32 {
        let room = (self.track_h - self.thumb_h).max(1.);
        -((thumb_top - self.track_top) / room).clamp(0., 1.) * self.max
    }
}

pub struct ScrollbarPrepaint {
    geometry: Option<Geometry>,
    hitbox: Option<Hitbox>,
}

impl IntoElement for Scrollbar {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for Scrollbar {
    type RequestLayoutState = ();
    type PrepaintState = ScrollbarPrepaint;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let mut style = Style {
            position: gpui::Position::Absolute,
            ..Style::default()
        };
        style.inset.top = px(0.).into();
        style.inset.left = px(0.).into();
        style.size.width = relative(1.).into();
        style.size.height = relative(1.).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        _: &mut App,
    ) -> ScrollbarPrepaint {
        let geometry = Geometry::measure(&self.handle, bounds, self.top_inset);
        // Wheel scrolling over the strip still reaches the page underneath.
        let hitbox = geometry
            .map(|g| window.insert_hitbox(g.hit_bounds(), HitboxBehavior::BlockMouseExceptScroll));
        ScrollbarPrepaint { geometry, hitbox }
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut (),
        prepaint: &mut ScrollbarPrepaint,
        window: &mut Window,
        _: &mut App,
    ) {
        let (Some(g), Some(hitbox)) = (prepaint.geometry, prepaint.hitbox.clone()) else {
            // Nothing to scroll (or the page went away mid-drag): let go.
            self.drag.set(None);
            return;
        };
        let dragging = self.drag.get().is_some();
        let hovered = hitbox.is_hovered(window);
        let active = dragging || hovered;
        let width = if active { WIDTH_ACTIVE } else { WIDTH };
        let thumb = Bounds::new(
            point(px(g.right - width), px(g.thumb_top)),
            size(px(width), px(g.thumb_h)),
        );
        if active {
            window.paint_quad(
                fill(
                    Bounds::new(
                        point(px(g.right - width), px(g.track_top)),
                        size(px(width), px(g.track_h)),
                    ),
                    rgba(0xffffff14),
                )
                .corner_radii(px(width / 2.)),
            );
        }
        window.paint_quad(
            fill(
                thumb,
                if dragging {
                    rgba(0xffffffb3)
                } else if hovered {
                    rgba(0xffffff8c)
                } else {
                    rgba(0xffffff4d)
                },
            )
            .corner_radii(px(width / 2.)),
        );
        window.set_cursor_style(CursorStyle::Arrow, &hitbox);

        let view = self.view;
        let handle = self.handle.clone();
        let scroll_to = move |thumb_top: f32, window: &mut Window, cx: &mut App| {
            let x = handle.offset().x;
            handle.set_offset(Point::new(x, px(g.offset_for(thumb_top))));
            cx.notify(view);
            window.refresh();
        };

        {
            let (drag, hitbox, scroll_to) = (self.drag.clone(), hitbox.clone(), scroll_to.clone());
            window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
                if phase != DispatchPhase::Bubble
                    || event.button != MouseButton::Left
                    || !hitbox.is_hovered(window)
                {
                    return;
                }
                let y = f32::from(event.position.y);
                let grab = if (g.thumb_top..g.thumb_top + g.thumb_h).contains(&y) {
                    y - g.thumb_top
                } else {
                    // A press on the track brings the thumb's middle to the pointer.
                    let grab = g.thumb_h / 2.;
                    scroll_to(y - grab, window, cx);
                    grab
                };
                drag.set(Some(grab));
                window.refresh();
                cx.stop_propagation();
            });
        }
        {
            let (drag, hitbox) = (self.drag.clone(), hitbox.clone());
            let was_hovered = Cell::new(hovered);
            window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
                if phase != DispatchPhase::Bubble {
                    return;
                }
                match drag.get() {
                    Some(grab) if event.pressed_button == Some(MouseButton::Left) => {
                        scroll_to(f32::from(event.position.y) - grab, window, cx);
                        cx.stop_propagation();
                    }
                    Some(_) => {
                        drag.set(None);
                        window.refresh();
                    }
                    None => {
                        let now = hitbox.is_hovered(window);
                        if was_hovered.replace(now) != now {
                            window.refresh();
                        }
                    }
                }
            });
        }
        {
            let drag = self.drag.clone();
            window.on_mouse_event(move |_: &MouseUpEvent, phase, window, _| {
                if phase == DispatchPhase::Capture && drag.take().is_some() {
                    window.refresh();
                }
            });
        }
    }
}
