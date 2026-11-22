//! Process memory figures for diagnostics (`RUST_LOG=miru=debug`).

/// Private bytes committed by the process (what Task Manager's "Memory" column tracks).
#[cfg(windows)]
pub fn private_bytes() -> Option<usize> {
    use windows_sys::Win32::System::ProcessStatus::{
        K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX,
    };
    use windows_sys::Win32::System::Threading::GetCurrentProcess;

    let mut counters = PROCESS_MEMORY_COUNTERS_EX {
        cb: std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32,
        ..Default::default()
    };
    // SAFETY: the pseudo-handle from GetCurrentProcess is always valid, and `counters` is a
    // correctly sized PROCESS_MEMORY_COUNTERS_EX, which the API accepts in place of the base
    // struct when `cb` says so.
    let ok = unsafe {
        K32GetProcessMemoryInfo(
            GetCurrentProcess(),
            (&raw mut counters).cast::<PROCESS_MEMORY_COUNTERS>(),
            counters.cb,
        )
    };
    (ok != 0).then_some(counters.PrivateUsage)
}

#[cfg(not(windows))]
pub fn private_bytes() -> Option<usize> {
    None
}

/// Megabytes, for log lines.
pub fn mb(bytes: usize) -> f64 {
    (bytes as f64 / (1024. * 1024.) * 10.).round() / 10.
}
