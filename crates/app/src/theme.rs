//! Design tokens and shared building blocks.
//!
//! The look: a deep neutral canvas, full-bleed artwork, frosted translucent surfaces, white
//! primary actions, and a single violet accent used sparingly.

use std::sync::OnceLock;
use std::time::Duration;

use gpui::{
    AnyElement, App, Background, BoxShadow, ClickEvent, Div, FontWeight, Hsla, Rgba, SharedString,
    Stateful, Window, WindowControlArea, div, hsla, linear_color_stop, linear_gradient, point,
    prelude::*, px, rgb, rgba, svg,
};

/// Large titles. Falls back to the platform UI font when unavailable.
pub const FONT_DISPLAY: &str = "Segoe UI Variable Display";
pub const FONT: &str = "Segoe UI Variable Text";

pub const RADIUS_CARD: f32 = 12.;

pub fn bg() -> Rgba {
    rgb(0x08080b)
}
pub fn text() -> Rgba {
    rgb(0xf5f5f7)
}
pub fn text_secondary() -> Rgba {
    rgb(0xa1a1ab)
}
pub fn text_tertiary() -> Rgba {
    rgb(0x6e6e78)
}
/// Brand accent: a moonlit violet. Used for small highlights only.
pub fn accent() -> Rgba {
    rgb(0x9d8cff)
}
/// Translucent "glass" fill for controls over artwork.
pub fn glass() -> Rgba {
    rgba(0xffffff1f)
}
pub fn glass_hover() -> Rgba {
    rgba(0xffffff33)
}
pub fn glass_border() -> Rgba {
    rgba(0xffffff1a)
}
pub fn panel() -> Rgba {
    rgba(0x1c1c22f2)
}

pub fn card_shadow(lift: f32) -> Vec<BoxShadow> {
    vec![BoxShadow {
        color: hsla(0., 0., 0., 0.35 + 0.25 * lift),
        offset: point(px(0.), px(6. + 10. * lift)),
        blur_radius: px(16. + 18. * lift),
        spread_radius: px(0.),
        inset: false,
    }]
}

/// Keyboard focus ring: a crisp white halo, applied with `.focus_visible(theme::focus_ring)` so
/// it only shows for keyboard navigation, not mouse clicks.
pub fn focus_ring(style: gpui::StyleRefinement) -> gpui::StyleRefinement {
    style.shadow(vec![BoxShadow {
        color: hsla(0., 0., 1., 0.95),
        offset: point(px(0.), px(0.)),
        blur_radius: px(0.),
        spread_radius: px(3.),
        inset: false,
    }])
}

/// Whether the OS allows UI animations (Windows: "Animation effects" setting).
pub fn motion_enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(platform_motion_enabled)
}

#[cfg(windows)]
fn platform_motion_enabled() -> bool {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        SPI_GETCLIENTAREAANIMATION, SystemParametersInfoW,
    };
    let mut enabled: i32 = 1;
    // SAFETY: SPI_GETCLIENTAREAANIMATION writes a BOOL to the provided pointer.
    let ok = unsafe {
        SystemParametersInfoW(SPI_GETCLIENTAREAANIMATION, 0, (&raw mut enabled).cast(), 0)
    };
    ok == 0 || enabled != 0
}

#[cfg(not(windows))]
fn platform_motion_enabled() -> bool {
    true
}

/// Lines one mouse-wheel notch scrolls (Windows "Choose how many lines to scroll", default 3).
pub fn wheel_lines_per_notch() -> f32 {
    static LINES: OnceLock<f32> = OnceLock::new();
    *LINES.get_or_init(platform_wheel_lines)
}

#[cfg(windows)]
fn platform_wheel_lines() -> f32 {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        SPI_GETWHEELSCROLLLINES, SystemParametersInfoW,
    };
    let mut lines: u32 = 3;
    // SAFETY: SPI_GETWHEELSCROLLLINES writes a UINT to the provided pointer.
    let ok =
        unsafe { SystemParametersInfoW(SPI_GETWHEELSCROLLLINES, 0, (&raw mut lines).cast(), 0) };
    // u32::MAX means "one screen per notch"; treat it, failures and 0 as the default.
    if ok == 0 || lines == 0 || lines == u32::MAX {
        3.
    } else {
        lines as f32
    }
}

#[cfg(not(windows))]
fn platform_wheel_lines() -> f32 {
    3.
}

/// Animation duration, collapsed to (almost) nothing when motion is reduced.
pub fn motion(ms: u64) -> Duration {
    Duration::from_millis(if motion_enabled() { ms } else { 1 })
}

/// Deterministic two-tone gradient for titles without artwork, so tiles stay distinct.
pub fn placeholder(seed: &str, accent_rgb: Option<u32>) -> Background {
    if let Some(rgb_value) = accent_rgb {
        let top = Hsla::from(rgb(rgb_value));
        let bottom = Hsla {
            l: (top.l * 0.45).max(0.03),
            ..top
        };
        return linear_gradient(
            180.,
            linear_color_stop(top, 0.),
            linear_color_stop(bottom, 1.),
        );
    }
    let hash = seed.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
    });
    // High bits: FNV-1a's low bits barely change between similar strings.
    let hue = ((hash >> 40) % 360) as f32 / 360.;
    let top = hsla(hue, 0.42, 0.30, 1.);
    let bottom = hsla((hue + 0.08) % 1., 0.50, 0.10, 1.);
    linear_gradient(
        160.,
        linear_color_stop(top, 0.),
        linear_color_stop(bottom, 1.),
    )
}

/// An SVG icon. GPUI only paints SVGs that carry their own text colour (it is not inherited).
pub fn icon(path: &'static str, size: f32, color: Rgba) -> impl IntoElement {
    svg()
        .path(path)
        .size(px(size))
        .flex_shrink_0()
        .text_color(color)
}

type ClickHandler = Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>;

/// Height shared by the pill buttons and the round buttons beside them.
pub const BUTTON_H: f32 = 48.;
/// Apple TV style buttons: the focused (hovered) one is white; the rest are smoked glass, a
/// see-through black with a faint light rim.
const SMOKE: u32 = 0x0000006b;
const SMOKE_RIM: u32 = 0xffffff33;
const WHITE: u32 = 0xf5f5f7ff;
const INK: u32 = 0x0b0b0fff;
/// Group on each button, so its icon can turn dark while the button is hovered.
const BUTTON_GROUP: &str = "button";
/// Group for a row of buttons: hovering any of them moves the white off the primary button,
/// so only one button is white at a time.
pub const BUTTON_ROW: &str = "button-row";

/// The primary action (Play): white at rest, like the focused button on Apple TV. In a
/// `BUTTON_ROW`, it turns charcoal while another button in the row is hovered.
pub fn primary_button(
    id: &'static str,
    icon_path: Option<&'static str>,
    label: impl Into<gpui::SharedString>,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    button_base(id, icon_path, true, label.into(), Box::new(on_click))
        .bg(rgba(WHITE))
        .border_color(rgba(WHITE))
        .text_color(rgba(INK))
        .shadow(card_shadow(0.2))
        .group_hover(BUTTON_ROW, |s| {
            s.bg(rgba(SMOKE))
                .border_color(rgba(SMOKE_RIM))
                .text_color(text())
        })
        .hover(|s| {
            s.bg(rgba(WHITE))
                .border_color(rgba(WHITE))
                .text_color(rgba(INK))
        })
}

/// A secondary action: charcoal at rest, white while hovered.
pub fn glass_button(
    id: &'static str,
    icon_path: Option<&'static str>,
    label: impl Into<gpui::SharedString>,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    button_base(id, icon_path, false, label.into(), Box::new(on_click))
        .bg(rgba(SMOKE))
        .border_color(rgba(SMOKE_RIM))
        .text_color(text())
        .hover(|s| {
            s.bg(rgba(WHITE))
                .border_color(rgba(WHITE))
                .text_color(rgba(INK))
                .shadow(card_shadow(0.2))
        })
}

/// An icon that follows its button's colours. SVGs don't inherit text colour, so the light
/// and dark versions are stacked and shown by hover state: dark while its own button is
/// hovered; for a primary button also dark at rest, and light while another button in its
/// row is hovered.
fn button_icon(path: &'static str, size: f32, primary: bool) -> Div {
    let layer = |color: Rgba| {
        svg()
            .path(path)
            .absolute()
            .top_0()
            .left_0()
            .size(px(size))
            .text_color(color)
    };
    let mut stack = div().relative().size(px(size)).flex_shrink_0();
    stack = if primary {
        stack
            .child(layer(rgba(INK)).group_hover(BUTTON_ROW, |s| s.opacity(0.)))
            .child(
                layer(text())
                    .opacity(0.)
                    .group_hover(BUTTON_ROW, |s| s.opacity(1.)),
            )
    } else {
        stack.child(layer(text()))
    };
    stack.child(
        layer(rgba(INK))
            .opacity(0.)
            .group_hover(BUTTON_GROUP, |s| s.opacity(1.)),
    )
}

fn button_base(
    id: &'static str,
    icon_path: Option<&'static str>,
    primary: bool,
    label: gpui::SharedString,
    on_click: ClickHandler,
) -> Stateful<Div> {
    div()
        .id(id)
        .group(BUTTON_GROUP)
        .tab_index(0)
        .focus_visible(focus_ring)
        .flex()
        .flex_shrink_0()
        .items_center()
        .gap(px(10.))
        .h(px(BUTTON_H))
        .border_1()
        .pl(px(if icon_path.is_some() { 24. } else { 30. }))
        .pr(px(30.))
        .rounded_full()
        .text_size(px(16.))
        .font_weight(FontWeight::MEDIUM)
        .cursor_pointer()
        .children(icon_path.map(|p| button_icon(p, 18., primary)))
        .child(label)
        .on_click(on_click)
}

/// Round icon button (back buttons, the "⋯" beside Play): charcoal, white while hovered.
pub fn round_button(
    id: impl Into<gpui::ElementId>,
    icon_path: &'static str,
    size: f32,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    round_button_styled(id, icon_path, size, false, on_click)
}

/// Round button that can stay white (selected), e.g. while its menu is open.
pub fn round_button_styled(
    id: impl Into<gpui::ElementId>,
    icon_path: &'static str,
    size: f32,
    selected: bool,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    let icon_size = size * 0.42;
    div()
        .id(id)
        .group(BUTTON_GROUP)
        .tab_index(0)
        .focus_visible(focus_ring)
        .size(px(size))
        .flex_shrink_0()
        .rounded_full()
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .border_1()
        .when(selected, |d| {
            d.bg(rgba(WHITE))
                .border_color(rgba(WHITE))
                .child(icon(icon_path, icon_size, rgba(INK)))
        })
        .when(!selected, |d| {
            d.bg(rgba(SMOKE))
                .border_color(rgba(SMOKE_RIM))
                .hover(|s| {
                    s.bg(rgba(WHITE))
                        .border_color(rgba(WHITE))
                        .shadow(card_shadow(0.2))
                })
                .child(button_icon(icon_path, icon_size, false))
        })
        .on_click(on_click)
}

/// Borderless round icon button for quiet controls (Settings rows). A disabled one is dimmed
/// and ignores clicks.
pub fn icon_button(
    id: impl Into<gpui::ElementId>,
    icon_path: &'static str,
    size: f32,
    enabled: bool,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    let ink = if enabled { text() } else { text_tertiary() };
    div()
        .id(id)
        .size(px(size))
        .flex_shrink_0()
        .rounded_full()
        .flex()
        .items_center()
        .justify_center()
        .child(icon(icon_path, size * 0.5, ink))
        .when(enabled, |d| {
            d.tab_index(0)
                .focus_visible(focus_ring)
                .cursor_pointer()
                .hover(|s| s.bg(glass_hover()))
                .on_click(on_click)
        })
}

/// Frosted circle for controls floating over video. GPUI can't blur what's behind an element,
/// so frosting is approximated with layers: a dark tint that mutes the picture, a milky white
/// wash on top, a bright rim, an inner top highlight and a soft drop shadow.
/// Drop shadow plus an inner top highlight, like light catching the edge of frosted glass.
pub fn frost_shadow() -> Vec<BoxShadow> {
    vec![
        BoxShadow {
            color: hsla(0., 0., 0., 0.30),
            offset: point(px(0.), px(6.)),
            blur_radius: px(20.),
            spread_radius: px(0.),
            inset: false,
        },
        BoxShadow {
            color: hsla(0., 0., 1., 0.22),
            offset: point(px(0.), px(1.)),
            blur_radius: px(0.),
            spread_radius: px(0.),
            inset: true,
        },
    ]
}

pub fn glass_round_button(
    id: impl Into<gpui::ElementId>,
    icon_path: &'static str,
    size: f32,
    selected: bool,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    let ink = if selected { bg() } else { text() };
    let group = gpui::SharedString::from(format!("frost-{size}-{icon_path}"));
    div()
        .id(id)
        .tab_index(0)
        .focus_visible(focus_ring)
        .group(group.clone())
        .relative()
        .size(px(size))
        .flex_shrink_0()
        .rounded_full()
        .overflow_hidden()
        .flex()
        .items_center()
        .justify_center()
        .bg(if selected { text() } else { rgba(0x14141c73) })
        .border_1()
        .border_color(rgba(0xffffff40))
        .shadow(frost_shadow())
        .cursor_pointer()
        .when(!selected, |d| {
            d.child(
                div()
                    .absolute()
                    .inset_0()
                    .rounded_full()
                    .bg(rgba(0xe6e6f024))
                    .group_hover(group.clone(), |s| s.bg(rgba(0xe6e6f040))),
            )
        })
        .child(
            div()
                .relative()
                .flex()
                .child(icon(icon_path, size * 0.42, ink)),
        )
        .on_click(on_click)
}

/// Minimise / maximise / close for the custom (transparent) title bar.
pub fn window_controls() -> impl IntoElement {
    let button =
        |id: &'static str, icon_path: &'static str, area: WindowControlArea, danger: bool| {
            div()
                .id(id)
                // Sized for a tall custom title bar (Windows uses 46x32 for a standard one).
                .w(px(48.))
                .h(px(40.))
                .flex()
                .items_center()
                .justify_center()
                .text_color(text())
                .window_control_area(area)
                // Windows runs the button on mouse up only if nothing in the app handled the
                // click, so cards and posters under it must not see the press at all.
                .block_mouse_except_scroll()
                .hover(move |s| {
                    if danger {
                        s.bg(rgb(0xc42b1c))
                    } else {
                        s.bg(glass())
                    }
                })
                .child(icon(icon_path, 12., text()))
        };
    div()
        .flex()
        .flex_shrink_0()
        .child(button(
            "win-min",
            "icons/window-min.svg",
            WindowControlArea::Min,
            false,
        ))
        .child(button(
            "win-max",
            "icons/window-max.svg",
            WindowControlArea::Max,
            false,
        ))
        .child(button(
            "win-close",
            "icons/window-close.svg",
            WindowControlArea::Close,
            true,
        ))
}

/// Thin progress track with a white fill.
pub fn progress_bar(fraction: f32, height: f32) -> impl IntoElement {
    div()
        .h(px(height))
        .w_full()
        .rounded_full()
        .bg(rgba(0xffffff33))
        .child(
            div()
                .h_full()
                .w(gpui::relative(fraction.clamp(0., 1.)))
                .rounded_full()
                .bg(text()),
        )
}

/// Empty element, for optional slots.
pub fn nothing() -> AnyElement {
    div().into_any_element()
}

/// Small label shown after hovering an icon-only control (the collapsed sidebar).
pub struct TextTooltip(SharedString);

impl Render for TextTooltip {
    fn render(&mut self, _: &mut Window, _: &mut gpui::Context<Self>) -> impl IntoElement {
        div()
            .px_2p5()
            .py_1()
            .rounded(px(8.))
            .bg(rgba(0x22222af5))
            .border_1()
            .border_color(glass_border())
            .shadow(card_shadow(0.))
            .font_family(FONT)
            .text_xs()
            .font_weight(FontWeight::MEDIUM)
            .text_color(text())
            .child(self.0.clone())
    }
}

pub fn text_tooltip(label: &'static str) -> impl Fn(&mut Window, &mut App) -> gpui::AnyView {
    move |_, cx| cx.new(|_| TextTooltip(label.into())).into()
}

/// Like [`text_tooltip`], for a label made at runtime.
pub fn text_tooltip_owned(label: SharedString) -> impl Fn(&mut Window, &mut App) -> gpui::AnyView {
    move |_, cx| cx.new(|_| TextTooltip(label.clone())).into()
}
