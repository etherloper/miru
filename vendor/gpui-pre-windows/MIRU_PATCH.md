# Miru patch to gpui-pre-windows 0.3.12

Upstream: crates.io `gpui-pre-windows` 0.3.12 (Apache-2.0), a snapshot of Zed's `gpui_windows`.

Single change in `src/directx_renderer.rs`, `DirectComposition::new`:

    CreateTargetForHwnd(hwnd, false)  ->  CreateTargetForHwnd(hwnd, true)

With `topmost = true`, the DirectComposition visual tree is composed above the window's child
HWNDs. Miru embeds mpv in a child HWND (its own D3D11 swap chain, hardware decoding), and
GPUI draws with a transparent background over it, so controls overlay the video.

When bumping gpui, re-copy the new crate version and reapply this one line.

## Patch 2: smooth mouse-wheel scrolling

`src/events.rs` (`handle_mouse_wheel_msg`, new `deliver_smooth_scroll`, a call at the top of
`draw_window`) and a `smooth_scroll` field on `WindowsWindowState` in `src/window.rs`.

Upstream delivers each wheel notch as one `ScrollDelta::Lines` jump. The patch queues the notch's
distance and `draw_window`, which runs on every vsync, delivers the share that is due since the
last frame: the remainder decays exponentially (`SMOOTH_SCROLL_TIME_CONSTANT`, 60 ms), so the
glide has the same ease-out shape at any refresh rate. (An earlier version used a `WM_TIMER`,
whose ~15.6 ms resolution stuttered on high-refresh displays.) A notch in the opposite direction
discards the queued remainder. Every `MIRU PATCH` comment marks a changed spot.
