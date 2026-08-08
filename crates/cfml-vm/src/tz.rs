//! Timezone resolution and offset computation backed by the IANA database
//! (`chrono-tz`).
//!
//! `chrono-tz` gives us faithful offsets, DST transitions and instant↔wall-clock
//! conversion for every IANA zone. What it does NOT carry is CLDR display data:
//! the locale-aware long names ("Eastern Standard Time") and the *theoretical*
//! DST abbreviations the JVM synthesises even for zones that never observe DST
//! (e.g. "JDT" / "Japan Daylight Time" for Asia/Tokyo). Those four name fields
//! are therefore tabulated from a Lucee 7.0.4 / OpenJDK 21 ground-truth capture
//! (`DISPLAY_NAMES`) rather than guessed — consistent with the engine's
//! "Lucee-verified or fail loud" rule for i18n shims. A valid zone that is not
//! in the table has its numeric facts available but no verified names, so the
//! name-bearing callers (getTimeZoneInfo, DateFormat `zzzz`) fail loudly.

use chrono::{DateTime, NaiveDateTime, Offset, TimeZone, Utc};
use chrono_tz::{OffsetComponents, Tz};

/// Verified zone display names: (shortStd, shortDst, longStd, longDst).
/// Captured byte-for-byte from Lucee 7.0.4 `getTimeZoneInfo()` (OpenJDK 21 CLDR).
/// Keyed by the canonical IANA id (`Tz::name()`).
type Names = (&'static str, &'static str, &'static str, &'static str);

fn display_names(canonical: &str) -> Option<Names> {
    let n = match canonical {
        "UTC" | "Etc/UTC" => ("UTC", "UTC", "Coordinated Universal Time", "Coordinated Universal Time"),
        "GMT" | "Etc/GMT" => ("GMT", "GMT", "Greenwich Mean Time", "Greenwich Mean Time"),
        "Europe/London" => ("GMT", "BST", "Greenwich Mean Time", "British Summer Time"),
        "Europe/Lisbon" => ("WET", "WEST", "Western European Standard Time", "Western European Summer Time"),
        "Europe/Paris" | "Europe/Berlin" | "Europe/Madrid" | "Europe/Rome"
        | "Europe/Amsterdam" | "Europe/Brussels" | "Europe/Vienna" | "Europe/Zurich"
        | "Europe/Warsaw" | "Europe/Prague" | "Europe/Stockholm" | "Europe/Oslo"
        | "Europe/Copenhagen" => (
            "CET", "CEST", "Central European Standard Time", "Central European Summer Time",
        ),
        "Europe/Athens" | "Europe/Helsinki" | "Europe/Bucharest" | "Europe/Kiev"
        | "Europe/Kyiv" | "Africa/Cairo" => (
            "EET", "EEST", "Eastern European Standard Time", "Eastern European Summer Time",
        ),
        "Europe/Moscow" => ("MSK", "MSD", "Moscow Standard Time", "Moscow Summer Time"),
        "America/New_York" | "America/Toronto" => ("EST", "EDT", "Eastern Standard Time", "Eastern Daylight Time"),
        "America/Chicago" | "America/Mexico_City" => ("CST", "CDT", "Central Standard Time", "Central Daylight Time"),
        "America/Denver" | "America/Phoenix" => ("MST", "MDT", "Mountain Standard Time", "Mountain Daylight Time"),
        "America/Los_Angeles" => ("PST", "PDT", "Pacific Standard Time", "Pacific Daylight Time"),
        "America/Anchorage" => ("AKST", "AKDT", "Alaska Standard Time", "Alaska Daylight Time"),
        "America/Sao_Paulo" => ("BRT", "BRST", "Brasilia Standard Time", "Brasilia Summer Time"),
        "Pacific/Honolulu" => ("HST", "HDT", "Hawaii-Aleutian Standard Time", "Hawaii-Aleutian Daylight Time"),
        "Asia/Tokyo" => ("JST", "JDT", "Japan Standard Time", "Japan Daylight Time"),
        "Asia/Shanghai" => ("CST", "CDT", "China Standard Time", "China Daylight Time"),
        "Asia/Hong_Kong" => ("HKT", "HKST", "Hong Kong Standard Time", "Hong Kong Summer Time"),
        "Asia/Singapore" => ("SGT", "SGST", "Singapore Standard Time", "Singapore Summer Time"),
        "Asia/Kolkata" => ("IST", "IDT", "India Standard Time", "India Daylight Time"),
        "Asia/Dubai" => ("GST", "GDT", "Gulf Standard Time", "Gulf Daylight Time"),
        "Asia/Tehran" => ("IRST", "IRDT", "Iran Standard Time", "Iran Daylight Time"),
        "Australia/Sydney" => ("AEST", "AEDT", "Australian Eastern Standard Time", "Australian Eastern Daylight Time"),
        "Australia/Adelaide" => ("ACST", "ACDT", "Australian Central Standard Time", "Australian Central Daylight Time"),
        "Pacific/Auckland" => ("NZST", "NZDT", "New Zealand Standard Time", "New Zealand Daylight Time"),
        "Africa/Johannesburg" => ("SAST", "SAST", "South Africa Standard Time", "South Africa Summer Time"),
        _ => return None,
    };
    Some(n)
}

/// Resolve a CFML timezone id to a `chrono_tz::Tz`. Case-insensitive (the
/// `case-insensitive` chrono-tz feature), with `UTC`/`GMT` short-circuited.
/// Returns `None` for an unknown id so callers can fail loudly.
pub fn resolve_tz(id: &str) -> Option<Tz> {
    let t = id.trim();
    if t.is_empty() {
        return Some(Tz::UTC);
    }
    match t.to_ascii_uppercase().as_str() {
        "UTC" | "Z" => return Some(Tz::UTC),
        "GMT" => return Some(Tz::GMT),
        _ => {}
    }
    // The `case-insensitive` feature adds `from_str_insensitive` (the plain
    // `FromStr` impl stays case-sensitive); CFML ids are case-insensitive.
    Tz::from_str_insensitive(t).ok()
}

/// Canonical IANA id for a resolved zone (e.g. "America/New_York").
pub fn canonical_name(tz: &Tz) -> String {
    tz.name().to_string()
}

/// The live offset facts for an instant in a zone, mirroring the numeric fields
/// of Lucee `getTimeZoneInfo()`.
pub struct OffsetInfo {
    /// Signed total UTC offset in seconds (east positive). NY EDT = -14400.
    pub total_secs: i64,
    /// Currently-applied DST saving in seconds (0 when not in DST right now).
    pub dst_secs: i64,
}

impl OffsetInfo {
    pub fn is_dst(&self) -> bool {
        self.dst_secs != 0
    }
}

/// Offset facts for `tz` at the given UTC instant.
pub fn offset_info_at(tz: &Tz, utc: NaiveDateTime) -> OffsetInfo {
    let off = tz.offset_from_utc_datetime(&utc);
    OffsetInfo {
        total_secs: off.fix().local_minus_utc() as i64,
        dst_secs: off.dst_offset().num_seconds(),
    }
}

/// Offset facts for `tz` right now.
pub fn offset_info_now(tz: &Tz) -> OffsetInfo {
    offset_info_at(tz, Utc::now().naive_utc())
}

/// Offset facts for a *local* wall-clock time interpreted in `tz` (used when a
/// formatter is handed a bare wall clock rather than an absolute instant).
pub fn offset_info_for_local(tz: &Tz, local: NaiveDateTime) -> OffsetInfo {
    let off = match tz.offset_from_local_datetime(&local) {
        chrono::LocalResult::Single(o) => o,
        chrono::LocalResult::Ambiguous(o, _) => o,
        chrono::LocalResult::None => tz.offset_from_utc_datetime(&local),
    };
    OffsetInfo {
        total_secs: off.fix().local_minus_utc() as i64,
        dst_secs: off.dst_offset().num_seconds(),
    }
}

/// The host system timezone id (`TZ` env var, then `/etc/localtime`, else UTC).
/// Mirrors the JVM's `TimeZone.getDefault()` for the no-`setTimeZone` case.
/// Resolved once per process (the JVM likewise caches its default): this is
/// consulted per tz-BIF call when no timezone is configured, and the fallback
/// path is an env read plus a `readlink` syscall.
pub fn system_tz_id() -> String {
    static SYSTEM_TZ: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    SYSTEM_TZ
        .get_or_init(|| {
            if let Ok(tz) = std::env::var("TZ") {
                if !tz.is_empty() {
                    return tz;
                }
            }
            #[cfg(unix)]
            {
                if let Ok(link) = std::fs::read_link("/etc/localtime") {
                    let link_str = link.to_string_lossy().to_string();
                    if let Some(pos) = link_str.find("zoneinfo/") {
                        return link_str[pos + 9..].to_string();
                    }
                }
            }
            "UTC".to_string()
        })
        .clone()
}

/// Verified display names for a zone, or `None` if it is not tabulated.
/// `(shortStd, shortDst, longStd, longDst)`.
pub fn names_for(tz: &Tz) -> Option<Names> {
    display_names(&canonical_name(tz))
}

/// Convert a local wall-clock time in `tz` to a UTC wall-clock time.
/// Picks the earlier instant for a fold/gap ambiguity (matches the JVM default).
pub fn local_to_utc(tz: &Tz, local: NaiveDateTime) -> Option<NaiveDateTime> {
    match tz.from_local_datetime(&local) {
        chrono::LocalResult::Single(dt) => Some(dt.naive_utc()),
        chrono::LocalResult::Ambiguous(a, _) => Some(a.naive_utc()),
        chrono::LocalResult::None => {
            // Spring-forward gap: nudge forward by the dst saving and retry.
            local
                .checked_add_signed(chrono::Duration::hours(1))
                .and_then(|t| tz.from_local_datetime(&t).single())
                .map(|dt| dt.naive_utc())
        }
    }
}

/// Convert a UTC wall-clock time to local wall-clock time in `tz`.
pub fn utc_to_local(tz: &Tz, utc: NaiveDateTime) -> NaiveDateTime {
    tz.from_utc_datetime(&utc).naive_local()
}

/// Interpret an absolute epoch-millis instant as a wall-clock time in `tz`,
/// returning the wall clock plus the signed offset seconds at that instant.
pub fn epoch_millis_to_wall(tz: &Tz, ms: i64) -> Option<(NaiveDateTime, i64)> {
    let utc = DateTime::from_timestamp_millis(ms)?.naive_utc();
    let off = tz.offset_from_utc_datetime(&utc).fix().local_minus_utc() as i64;
    Some((utc_to_local(tz, utc), off))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn ndt(y: i32, m: u32, d: u32, h: u32, mi: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(y, m, d).unwrap().and_hms_opt(h, mi, 0).unwrap()
    }

    #[test]
    fn resolves_common_and_aliases() {
        assert_eq!(resolve_tz("UTC"), Some(Tz::UTC));
        assert_eq!(resolve_tz(""), Some(Tz::UTC));
        // Case-insensitive.
        assert_eq!(
            canonical_name(&resolve_tz("america/new_york").unwrap()),
            "America/New_York"
        );
        assert!(resolve_tz("Not/AZone").is_none());
    }

    #[test]
    fn dst_aware_offsets_match_lucee() {
        let ny = resolve_tz("America/New_York").unwrap();
        // Summer -> EDT (-4h), DST applied.
        let summer = offset_info_for_local(&ny, ndt(2026, 6, 22, 12, 0));
        assert_eq!(summer.total_secs, -14400);
        assert_eq!(summer.dst_secs, 3600);
        assert!(summer.is_dst());
        // Winter -> EST (-5h), no DST.
        let winter = offset_info_for_local(&ny, ndt(2026, 1, 22, 12, 0));
        assert_eq!(winter.total_secs, -18000);
        assert_eq!(winter.dst_secs, 0);
        assert!(!winter.is_dst());
    }

    #[test]
    fn local_utc_roundtrip_dst() {
        let ny = resolve_tz("America/New_York").unwrap();
        // 12:00 EDT == 16:00 UTC.
        let utc = local_to_utc(&ny, ndt(2026, 6, 22, 12, 0)).unwrap();
        assert_eq!(utc, ndt(2026, 6, 22, 16, 0));
        assert_eq!(utc_to_local(&ny, ndt(2026, 6, 22, 16, 0)), ndt(2026, 6, 22, 12, 0));
        // 12:00 EST == 17:00 UTC.
        assert_eq!(local_to_utc(&ny, ndt(2026, 1, 22, 12, 0)).unwrap(), ndt(2026, 1, 22, 17, 0));
    }

    #[test]
    fn names_only_for_tabulated_zones() {
        assert_eq!(
            names_for(&resolve_tz("Europe/London").unwrap()),
            Some(("GMT", "BST", "Greenwich Mean Time", "British Summer Time"))
        );
        // A valid but untabulated zone -> no verified names (callers fail loud).
        assert!(names_for(&resolve_tz("Antarctica/Troll").unwrap()).is_none());
    }
}
