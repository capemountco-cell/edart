//! The time and the local time zone, for the trading ledger's days.

use std::time::{SystemTime, UNIX_EPOCH};

use edart_core::ledger::Now;

/// Now, with the Mac's offset from UTC.
pub fn now() -> Now {
    let unix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    Now {
        unix,
        utc_offset: utc_offset(unix),
    }
}

/// Local time minus UTC at `unix`, in seconds, from the C library (which
/// knows about daylight saving). Zero when it cannot say.
#[cfg(unix)]
fn utc_offset(unix: i64) -> i32 {
    /// `struct tm` as macOS and Linux lay it out.
    #[repr(C)]
    struct Tm {
        sec: i32,
        min: i32,
        hour: i32,
        mday: i32,
        mon: i32,
        year: i32,
        wday: i32,
        yday: i32,
        isdst: i32,
        gmtoff: std::ffi::c_long,
        zone: *const std::ffi::c_char,
    }
    extern "C" {
        fn localtime_r(time: *const i64, result: *mut Tm) -> *mut Tm;
    }
    let mut tm = std::mem::MaybeUninit::<Tm>::zeroed();
    // SAFETY: both pointers are valid for the call, `time_t` is 64 bits
    // on every platform this builds for, and the result is only read
    // when the call says it filled it in.
    let filled = unsafe { localtime_r(&unix, tm.as_mut_ptr()) };
    if filled.is_null() {
        return 0;
    }
    // SAFETY: filled in by localtime_r.
    let offset = unsafe { tm.assume_init() }.gmtoff;
    // No time zone is more than a day from UTC.
    i32::try_from(offset)
        .ok()
        .filter(|o| o.abs() <= 86_400)
        .unwrap_or(0)
}

#[cfg(not(unix))]
fn utc_offset(_unix: i64) -> i32 {
    0
}
