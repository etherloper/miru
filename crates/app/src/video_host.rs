//! Native child window that hosts mpv's video output inside the GPUI window.
//!
//! The patched GPUI Windows backend composites its UI above child windows, so this host sits
//! underneath a transparent GPUI scene. The host is disabled, which makes Windows route mouse
//! input over the video to the GPUI window instead.

use anyhow::{Context as _, Result, bail};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DestroyWindow, HWND_BOTTOM, SWP_NOACTIVATE, SetWindowPos, WS_CHILD,
    WS_CLIPSIBLINGS, WS_DISABLED, WS_VISIBLE,
};

pub struct VideoHost {
    hwnd: HWND,
    size: (i32, i32),
}

impl VideoHost {
    pub fn new(parent: &impl HasWindowHandle) -> Result<Self> {
        let RawWindowHandle::Win32(handle) = parent
            .window_handle()
            .map_err(|e| anyhow::anyhow!("no native window handle: {e}"))?
            .as_raw()
        else {
            bail!("video host requires a Win32 window");
        };
        let class: Vec<u16> = "STATIC\0".encode_utf16().collect();
        // SAFETY: valid parent HWND from GPUI; "STATIC" is a system window class.
        let hwnd = unsafe {
            CreateWindowExW(
                0,
                class.as_ptr(),
                std::ptr::null(),
                WS_CHILD | WS_VISIBLE | WS_DISABLED | WS_CLIPSIBLINGS,
                0,
                0,
                1,
                1,
                handle.hwnd.get() as HWND,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null(),
            )
        };
        if hwnd.is_null() {
            return Err(std::io::Error::last_os_error()).context("CreateWindowExW failed");
        }
        Ok(Self { hwnd, size: (0, 0) })
    }

    /// Value for mpv's `wid` option.
    pub fn wid(&self) -> i64 {
        self.hwnd as i64
    }

    /// Resizes the host to the given physical pixel size, anchored at the top-left.
    pub fn resize(&mut self, width: i32, height: i32) {
        if self.size == (width, height) {
            return;
        }
        self.size = (width, height);
        // SAFETY: hwnd is a live child window owned by this struct.
        unsafe {
            SetWindowPos(self.hwnd, HWND_BOTTOM, 0, 0, width, height, SWP_NOACTIVATE);
        }
    }
}

impl Drop for VideoHost {
    fn drop(&mut self) {
        // SAFETY: hwnd was created by us and is destroyed exactly once.
        unsafe {
            DestroyWindow(self.hwnd);
        }
    }
}

/// Hides the mouse cursor until it next moves (GPUI restores it on the next `WM_SETCURSOR`).
pub fn hide_cursor_until_moved() {
    // SAFETY: SetCursor with a null handle only hides the cursor for this thread's windows.
    unsafe {
        windows_sys::Win32::UI::WindowsAndMessaging::SetCursor(std::ptr::null_mut());
    }
}
