//! The performance counter, the clock Windows Graphics Capture stamps its frames with.

use std::sync::OnceLock;

use windows::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};

fn frequency() -> i64 {
    static FREQ: OnceLock<i64> = OnceLock::new();
    *FREQ.get_or_init(|| {
        let mut f = 0i64;
        // SAFETY: writes one i64 we own; it never fails on Windows XP or later.
        unsafe { QueryPerformanceFrequency(&mut f) }.expect("QueryPerformanceFrequency");
        f
    })
}

/// Now, in 100 ns units of the performance counter: the units of a capture frame's
/// `SystemRelativeTime`, so the two can be subtracted.
pub fn now_100ns() -> i64 {
    let mut c = 0i64;
    // SAFETY: as above.
    unsafe { QueryPerformanceCounter(&mut c) }.expect("QueryPerformanceCounter");
    let f = frequency();
    ((c as i128 * 10_000_000) / f as i128) as i64
}

/// Now, in seconds since some fixed moment, for measuring durations.
pub fn now_secs() -> f64 {
    now_100ns() as f64 / 10_000_000.0
}
