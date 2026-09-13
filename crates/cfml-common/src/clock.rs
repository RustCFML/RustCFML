//! Platform-portable clock.
//!
//! On native targets this is a thin wrapper over `std::time::SystemTime` and
//! `std::time::Instant`. On `wasm32-unknown-unknown` (e.g. Cloudflare Workers)
//! `SystemTime::now()` and `Instant::now()` panic because the platform has no
//! system clock — JS `Date.now()` and `performance.now()` are the only time
//! sources. This module provides a single set of functions that work in both
//! environments, so every callsite inside the RustCFML stack can switch off
//! `std::time::*::now()` and become wasm32-safe.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

// ─────────────────────────────────────────────
// Wall-clock helpers
// ─────────────────────────────────────────────

/// Current Unix epoch seconds.
#[inline]
pub fn now_unix_secs() -> u64 {
    (now_unix_millis() / 1_000) as u64
}

/// Current Unix epoch milliseconds.
#[inline]
pub fn now_unix_millis() -> u128 {
    #[cfg(not(target_arch = "wasm32"))]
    {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or_default()
    }
    #[cfg(target_arch = "wasm32")]
    {
        js_date_now_ms() as u128
    }
}

/// Current Unix epoch nanoseconds.
///
/// On wasm32 we only have millisecond resolution from `Date.now()`, so the
/// nanosecond value is `millis * 1_000_000`. Good enough for the
/// nano-resolution callers in RustCFML (UUIDs, getTickCount, etc.).
#[inline]
pub fn now_unix_nanos() -> u128 {
    #[cfg(not(target_arch = "wasm32"))]
    {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default()
    }
    #[cfg(target_arch = "wasm32")]
    {
        (js_date_now_ms() as u128) * 1_000_000
    }
}

/// Construct a `SystemTime` for *now*. Use this instead of `SystemTime::now()`
/// anywhere the result is stored or compared (e.g. mtime cache keys).
#[inline]
pub fn now_system_time() -> SystemTime {
    UNIX_EPOCH + Duration::from_nanos(now_unix_nanos().min(u64::MAX as u128) as u64)
}

// ─────────────────────────────────────────────
// Monotonic helpers (for deadline / elapsed comparisons)
// ─────────────────────────────────────────────

/// A monotonic timepoint. Use for deadlines and elapsed measurements; never
/// for wall-clock display.
///
/// On native this is backed by `std::time::Instant`. On wasm32 it stores
/// milliseconds from `performance.now()` (which is also monotonic per the
/// HTML spec, though Cloudflare clamps it to ~1ms resolution).
#[derive(Clone, Copy, Debug)]
pub struct Monotonic {
    #[cfg(not(target_arch = "wasm32"))]
    inner: std::time::Instant,
    #[cfg(target_arch = "wasm32")]
    millis: f64,
}

impl Monotonic {
    #[inline]
    pub fn now() -> Self {
        #[cfg(not(target_arch = "wasm32"))]
        {
            Self {
                inner: std::time::Instant::now(),
            }
        }
        #[cfg(target_arch = "wasm32")]
        {
            Self {
                millis: js_perf_now_ms(),
            }
        }
    }

    /// Time elapsed since this point.
    #[inline]
    pub fn elapsed(&self) -> Duration {
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.inner.elapsed()
        }
        #[cfg(target_arch = "wasm32")]
        {
            let now = js_perf_now_ms();
            let delta = (now - self.millis).max(0.0);
            Duration::from_micros((delta * 1_000.0) as u64)
        }
    }

    /// Returns `Some(elapsed)` if `earlier` precedes `self`, else `None`.
    #[inline]
    pub fn checked_duration_since(&self, earlier: Monotonic) -> Option<Duration> {
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.inner.checked_duration_since(earlier.inner)
        }
        #[cfg(target_arch = "wasm32")]
        {
            let delta = self.millis - earlier.millis;
            if delta < 0.0 {
                None
            } else {
                Some(Duration::from_micros((delta * 1_000.0) as u64))
            }
        }
    }
}

impl std::ops::Add<Duration> for Monotonic {
    type Output = Monotonic;
    #[inline]
    fn add(self, rhs: Duration) -> Monotonic {
        #[cfg(not(target_arch = "wasm32"))]
        {
            Monotonic {
                inner: self.inner + rhs,
            }
        }
        #[cfg(target_arch = "wasm32")]
        {
            Monotonic {
                millis: self.millis + (rhs.as_micros() as f64) / 1_000.0,
            }
        }
    }
}

impl PartialEq for Monotonic {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.inner == other.inner
        }
        #[cfg(target_arch = "wasm32")]
        {
            self.millis == other.millis
        }
    }
}

impl Eq for Monotonic {}

impl PartialOrd for Monotonic {
    #[inline]
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Monotonic {
    #[inline]
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.inner.cmp(&other.inner)
        }
        #[cfg(target_arch = "wasm32")]
        {
            self.millis
                .partial_cmp(&other.millis)
                .unwrap_or(std::cmp::Ordering::Equal)
        }
    }
}

// ─────────────────────────────────────────────
// wasm32 JS bridges
// ─────────────────────────────────────────────

#[cfg(target_arch = "wasm32")]
fn js_date_now_ms() -> f64 {
    js_sys::Date::now()
}

#[cfg(target_arch = "wasm32")]
fn js_perf_now_ms() -> f64 {
    use wasm_bindgen::{JsCast, JsValue};
    let global = js_sys::global();
    let perf = js_sys::Reflect::get(&global, &JsValue::from_str("performance"))
        .unwrap_or(JsValue::NULL);
    let now_fn = js_sys::Reflect::get(&perf, &JsValue::from_str("now"))
        .unwrap_or(JsValue::NULL);
    now_fn
        .dyn_ref::<js_sys::Function>()
        .and_then(|f| f.call0(&perf).ok())
        .and_then(|v| v.as_f64())
        // Fall back to Date.now() if performance.now() isn't available.
        .unwrap_or_else(js_date_now_ms)
}

// ─────────────────────────────────────────────
// Request timezone
// ─────────────────────────────────────────────

thread_local! {
    /// The request's timezone (`setTimeZone`, or `this.timezone`), as an IANA
    /// id. `None` = no request zone set, so the system zone applies.
    ///
    /// Date PARSING needs this and cannot reach the VM. An offset-bearing
    /// string names an absolute instant, and CFML reports instants in the
    /// REQUEST zone — `dateConvert` already did, via the VM intercept, but the
    /// parser resolved `+0000` against chrono's `Local` (the SYSTEM zone).
    /// With `setTimeZone("Asia/Kolkata")` on a UTC box, Lucee 7.1 parses
    /// "August, 25 2026 09:00:14 +0000" as 14:30:14 and we returned 09:00:14,
    /// so `dateDiff(dateConvert("utc2Local", d), thatString)` was out by the
    /// whole zone offset (GH #415's CI failure; measured on Lucee).
    ///
    /// It hid locally because the suite leaves a request zone set and most
    /// developer machines run that same zone as their system zone — the two
    /// only diverge on a UTC CI box, or under an explicit setTimeZone.
    ///
    /// It lives HERE, in the crate both sides already depend on, rather than in
    /// `cfml-stdlib`: the VM is the writer and the stdlib date parser the
    /// reader, but `cfml-stdlib` is an OPTIONAL dependency of `cfml-vm` (the
    /// wasm32 worker builds without it), so a direct `cfml_stdlib::` call from
    /// the VM broke the `wasm32-unknown-unknown` build — which a plain
    /// `cargo build` never compiles.
    static REQUEST_TZ: std::cell::RefCell<Option<String>> =
        const { std::cell::RefCell::new(None) };
}

/// Publish the request timezone to the date parser. Called by the VM wherever
/// its own `timezone` field is assigned; `None` restores the system zone.
pub fn set_request_timezone(id: Option<String>) {
    REQUEST_TZ.with(|t| *t.borrow_mut() = id.filter(|s| !s.is_empty()));
}

/// The request timezone's IANA id, if one is set.
pub fn request_timezone() -> Option<String> {
    REQUEST_TZ.with(|t| t.borrow().clone())
}
