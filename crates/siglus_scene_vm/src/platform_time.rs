//! Cross-platform time helpers.
//!
//! wasm32-unknown-unknown does not support std::time::Instant::now() or
//! std::time::SystemTime::now(). Use this module anywhere runtime code needs
//! wall-clock or monotonic time.

pub use std::time::Duration;

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub use web_time::Instant;

#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
pub use std::time::Instant;

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub fn unix_time_millis() -> u128 {
    let ms = js_sys::Date::now();
    if ms.is_finite() && ms > 0.0 {
        ms as u128
    } else {
        0
    }
}

#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
pub fn unix_time_millis() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

pub fn unix_time_secs() -> u64 {
    (unix_time_millis() / 1000) as u64
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LocalTimeFields {
    pub year: i32,
    pub month: u32,
    pub day: u32,
    /// 0 = Sunday, 6 = Saturday, matching Windows SYSTEMTIME.wDayOfWeek.
    pub weekday_sunday0: u32,
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
    pub millisecond: u32,
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub fn local_time_fields() -> LocalTimeFields {
    let now = js_sys::Date::new_0();
    LocalTimeFields {
        year: now.get_full_year() as i32,
        month: now.get_month() + 1,
        day: now.get_date(),
        weekday_sunday0: now.get_day(),
        hour: now.get_hours(),
        minute: now.get_minutes(),
        second: now.get_seconds(),
        millisecond: now.get_milliseconds(),
    }
}

#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
pub fn local_time_fields() -> LocalTimeFields {
    use chrono::{Datelike, Timelike};

    let now = chrono::Local::now();
    LocalTimeFields {
        year: now.year(),
        month: now.month(),
        day: now.day(),
        weekday_sunday0: now.weekday().num_days_from_sunday(),
        hour: now.hour(),
        minute: now.minute(),
        second: now.second(),
        millisecond: now.timestamp_subsec_millis(),
    }
}

pub fn local_log_timestamp() -> String {
    let t = local_time_fields();
    format!(
        "[{year:04}-{month:02}-{day:02} {hour:02}:{minute:02}:{second:02}]",
        year = t.year,
        month = t.month,
        day = t.day,
        hour = t.hour,
        minute = t.minute,
        second = t.second,
    )
}


/// Milliseconds on a process-lifetime monotonic clock. This is the real-time
/// source for blocking waits; the hosted virtual clock replaces it wholesale
/// so a headless session can advance deterministically.
pub fn monotonic_now_ms() -> u64 {
    static EPOCH: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    let epoch = EPOCH.get_or_init(Instant::now);
    epoch.elapsed().as_millis() as u64
}

/// Astra hosted mode: a virtual monotonic clock. While enabled, every real
/// wait deadline is measured against the virtual time, which the session
/// advances by one frame's elapsed value per step.
#[cfg(feature = "astra-hosted")]
pub mod hosted_clock {
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

    static VIRTUAL: AtomicBool = AtomicBool::new(false);
    static VIRTUAL_NOW_MS: AtomicU64 = AtomicU64::new(0);

    pub fn enable() {
        VIRTUAL_NOW_MS.store(super::monotonic_now_ms(), Ordering::Release);
        VIRTUAL.store(true, Ordering::Release);
    }

    pub fn disable() {
        VIRTUAL.store(false, Ordering::Release);
    }

    pub fn enabled() -> bool {
        VIRTUAL.load(Ordering::Acquire)
    }

    pub fn now_ms() -> u64 {
        VIRTUAL_NOW_MS.load(Ordering::Acquire)
    }

    /// Advances the virtual clock. No-op while the virtual clock is off.
    pub fn advance_ms(ms: u64) {
        if VIRTUAL.load(Ordering::Relaxed) {
            VIRTUAL_NOW_MS.fetch_add(ms, Ordering::Relaxed);
        }
    }
}

/// The monotonic time source blocking waits must observe: the virtual clock
/// while the hosted session drives it, the real process clock otherwise.
#[cfg(feature = "astra-hosted")]
pub fn wait_now_ms() -> u64 {
    if hosted_clock::enabled() {
        hosted_clock::now_ms()
    } else {
        monotonic_now_ms()
    }
}
