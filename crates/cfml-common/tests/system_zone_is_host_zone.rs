//! With no zone configured (no `setTimeZone()`, `this.timezone` or cfconfig
//! `runtime.timezone`), CFML dates are in the HOST's zone, as Lucee's are in
//! the JVM's default zone: `now()` is the wall-clock time the computer shows.
//!
//! `system_zone()` is found from the `TZ` environment variable, then the
//! `/etc/localtime` symlink, else UTC. Windows has neither, so on a Windows
//! computer outside UTC the system zone is UTC: in Brisbane (UTC+10) `now()`
//! reads 10 hours behind the computer's clock, and every date an application
//! writes with it is 10 hours out.
//!
//! The CFML suite can't show this: `tests/.cfconfig.json` pins every request
//! to Europe/London, so the system zone is never consulted there. Nor can a
//! UTC box (CI), where the UTC fallback happens to be right, or macOS and most
//! Linux, which have the symlink. So this compares the system zone's offset
//! with the one the operating system itself gives (`chrono::Local`: the C
//! library on Unix, `GetTimeZoneInformation` on Windows). It fails on a
//! Windows computer in any zone but UTC, and passes everywhere else.

use cfml_common::datetime::{offset_secs_at, system_zone};

#[test]
fn system_zone_has_the_hosts_utc_offset() {
    // TZ names the zone on purpose, and wins over the host's own (on Windows
    // chrono::Local doesn't read it), so there's nothing to compare against.
    if std::env::var("TZ").is_ok_and(|tz| !tz.is_empty()) {
        eprintln!("TZ is set, so the system zone is the one it names: skipped");
        return;
    }
    let host = chrono::Local::now();
    let host_offset = host.offset().local_minus_utc();
    let zone = system_zone();
    let zone_offset = offset_secs_at(&zone, host.timestamp_millis());
    assert_eq!(
        zone_offset, host_offset,
        "the system zone ({zone}) is UTC{:+} hours, but this computer's clock is UTC{:+} hours",
        zone_offset as f64 / 3600.0,
        host_offset as f64 / 3600.0
    );
}
