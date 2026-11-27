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

/// How big a scrollbar is drawn.
#[derive(Clone, Copy, PartialEq)]
pub enum ScrollbarSize {
    /// For pages and panels.
    Regular,
    /// For small menus: thinner, closer to the edge, with a narrower strip for the pointer, so
    /// it takes little room from the rows beside it.
    Compact,
}

/// Measurements of one size: gap from the right edge, thumb width, the width of the strip that responds to the pointer, and the shortest thumb.
#[derive(Clone, Copy)]
struct Metrics {
    edge: f32,
    width: f32,
    hit_width: f32,
    min_thumb: f32,
}

impl ScrollbarSize {
    fn metrics(self) -> Metrics {
        match self {
            Self::Regular => Metrics {
                edge: 3.,
                width: 7.,
                // Wider than the thumb, so it stays easy to grab.
                hit_width: 14.,
                min_thumb: 36.,
            },
            Self::Compact => Metrics {
                edge: 3.,
                width: 4.,
                hit_width: 10.,
                min_thumb: 24.,
            },
        }
    }

    /// Room to leave at the right of an area's content so the thumb and its pointer strip
    /// never cover it.
    pub fn gutter(self) -> f32 {
        self.metrics().hit_width
    }
}

/// Space kept clear at the ends of the track.
const PAD: f32 = 4.;

/// Drag state kept by the view between frames: while dragging, where in the thumb the
/// pointer took hold of it.
pub type ScrollbarDrag = Rc<Cell<Option<f32>>>;

/// A smaller scrolling area (a panel, list or menu) and its scrollbar, kept by the view.
///
/// Track the area with `.track_scroll(&area.handle)` and add [`ScrollArea::scrollbar`] as its
/// last child: the scrollbar draws over the area's right edge from the handle, wherever the
/// content has scrolled to.
#[derive(Clone, Default)]
pub struct ScrollArea {
    pub handle: ScrollHandle,
    drag: ScrollbarDrag,
}

impl ScrollArea {
    pub fn scrollbar(&self, view: EntityId) -> Scrollbar {
        Scrollbar {
            inside: true,
            ..Scrollbar::new(self.handle.clone(), self.drag.clone(), 0., view)
        }
    }

    /// A thin scrollbar for a small menu. Leave [`ScrollbarSize::gutter`] free at the right of
    /// the menu's rows so it never sits over their text.
    pub fn compact_scrollbar(&self, view: EntityId) -> Scrollbar {
        Scrollbar {
            size: ScrollbarSize::Compact,
            ..self.scrollbar(view)
        }
    }

    /// Back to the top, for an area that opens afresh (a menu shown again starts at the top,
    /// as it did when it kept no handle).
    pub fn reset(&self) {
        if self.handle.offset() != Point::default() {
            self.handle.set_offset(Point::default());
        }
        self.drag.set(None);
    }
}

pub struct Scrollbar {
    handle: ScrollHandle,
    drag: ScrollbarDrag,
    /// Keeps the track below an overlay at the top of the page (the title bar).
    top_inset: f32,
    /// The view to re-render when the scrollbar moves the page.
    view: EntityId,
    size: ScrollbarSize,
    /// A child of the scroll container itself (moving with its content), rather than an
    /// overlay on a still parent.
    inside: bool,
}

impl Scrollbar {
    /// An overlay filling its (relatively positioned) parent; place it after the page.
    pub fn new(handle: ScrollHandle, drag: ScrollbarDrag, top_inset: f32, view: EntityId) -> Self {
        Self {
            handle,
            drag,
            top_inset,
            view,
            size: ScrollbarSize::Regular,
            inside: false,
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
    metrics: Metrics,
}

impl Geometry {
    fn measure(
        handle: &ScrollHandle,
        area: Option<Bounds<Pixels>>,
        top_inset: f32,
        metrics: Metrics,
    ) -> Option<Self> {
        let max = f32::from(handle.max_offset().y);
        let view = handle.bounds();
        let view_h = f32::from(view.size.height);
        if max < 1. || view_h < 1. {
            return None;
        }
        // An overlay keeps its track below whatever covers the top of its parent (the nav
        // bar). Inside a scroll container `area` would move with the content, so there is none.
        let floor = area.map_or(f32::NEG_INFINITY, |area| f32::from(area.top()) + top_inset);
        let track_top = f32::from(view.top()).max(floor) + PAD;
        let track_h = f32::from(view.bottom()) - PAD - track_top;
        if track_h < metrics.min_thumb * 2. {
            return None;
        }
        let thumb_h = (track_h * view_h / (view_h + max)).clamp(metrics.min_thumb, track_h);
        let progress = (-f32::from(handle.offset().y) / max).clamp(0., 1.);
        Some(Self {
            track_top,
            track_h,
            thumb_top: track_top + (track_h - thumb_h) * progress,
            thumb_h,
            right: f32::from(view.right()) - metrics.edge,
            max,
            metrics,
        })
    }

    fn hit_bounds(&self) -> Bounds<Pixels> {
        Bounds::new(
            point(
                px(self.right + self.metrics.edge - self.metrics.hit_width),
                px(self.track_top),
            ),
            size(px(self.metrics.hit_width), px(self.track_h)),
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
        if self.inside {
            // Zero-sized at its static position (the start of the content). A scroll container
            // measures its content from its children's bounds, so a full-size child here would
            // count the container's padding as content and make a list that fits scroll by a
            // few pixels. The track is placed from the scroll handle instead.
            style.size.width = px(0.).into();
            style.size.height = px(0.).into();
        } else {
            // An overlay filling its still parent, which places the track under the nav bar.
            style.inset.top = px(0.).into();
            style.inset.left = px(0.).into();
            style.size.width = relative(1.).into();
            style.size.height = relative(1.).into();
        }
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
        let geometry = Geometry::measure(
            &self.handle,
            (!self.inside).then_some(bounds),
            self.top_inset,
            self.size.metrics(),
        );
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
        // A fixed width; hovering only brightens it.
        let width = g.metrics.width;
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
