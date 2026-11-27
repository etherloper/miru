//! The local wall-clock time some seconds from now, formatted the way Windows shows times
//! (12- or 24-hour, as the user has set it), for "Ends at 10:42 PM".

/// The time `seconds` from now, without seconds, e.g. "10:42 PM" or "22:42".
#[cfg(windows)]
pub fn time_in(seconds: f64) -> Option<String> {
    use windows_sys::Win32::Foundation::SYSTEMTIME;
    use windows_sys::Win32::Globalization::{GetTimeFormatEx, TIME_NOSECONDS};
    use windows_sys::Win32::System::SystemInformation::GetLocalTime;

    if !seconds.is_finite() || seconds < 0. {
        return None;
    }
    let mut now = SYSTEMTIME {
        wYear: 0,
        wMonth: 0,
        wDayOfWeek: 0,
        wDay: 0,
        wHour: 0,
        wMinute: 0,
        wSecond: 0,
        wMilliseconds: 0,
    };
    // SAFETY: GetLocalTime only writes the struct it is given.
    unsafe { GetLocalTime(&mut now) };
    // Only the time of day is formatted, so the date can stay as it is past midnight.
    let now_secs =
        u32::from(now.wHour) * 3600 + u32::from(now.wMinute) * 60 + u32::from(now.wSecond);
    let end = (u64::from(now_secs) + seconds.round() as u64) % 86_400;
    now.wHour = (end / 3600) as u16;
    now.wMinute = (end % 3600 / 60) as u16;
    now.wSecond = 0;
    now.wMilliseconds = 0;

    let mut buf = [0u16; 64];
    // SAFETY: a null locale name means the user's default; the buffer and its length match.
    let len = unsafe {
        GetTimeFormatEx(
            std::ptr::null(),
            TIME_NOSECONDS,
            &now,
            std::ptr::null(),
            buf.as_mut_ptr(),
            buf.len() as i32,
        )
    };
    // The length counts the terminating nul.
    let len = usize::try_from(len).ok().filter(|&n| n > 1)? - 1;
    Some(String::from_utf16_lossy(&buf[..len]))
}

#[cfg(not(windows))]
pub fn time_in(_seconds: f64) -> Option<String> {
    None
}
