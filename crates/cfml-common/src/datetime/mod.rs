//! The CFML date value (GH #441).
//!
//! A date is an **instant**, as on Lucee (where it is a `java.util.Date`): a
//! point on the time line with no zone of its own. Its calendar fields, its
//! string form and its numeric value are all read in the *request* time zone
//! (`setTimeZone` / `this.timezone`, else the system zone) at the moment they
//! are asked for, so the same date prints differently after `setTimeZone`,
//! exactly as Lucee does.
//!
//! Dates used to be strings (`2026-01-02 10:20:30`): everything below a second
//! was lost, a date printed differently from Lucee (`{ts '…'}`), date member
//! functions did not exist, and wall-clock arithmetic was wrong across a DST
//! change. Where the value still differs from Lucee: `docs/known-issues.md` §117.
//!
//! The value keeps nanoseconds. Values are *made* at the configured precision
//! ([`Precision`]); the default is milliseconds, which is Lucee's, so every
//! result matches Lucee unless an application asks for more (GH #479).

pub mod parse;
pub mod zone;

use chrono::{Datelike, NaiveDate, NaiveDateTime, Offset, TimeZone, Timelike};
use chrono_tz::{OffsetComponents, Tz};
use std::cell::Cell;
use std::sync::OnceLock;

/// What a date prints as. Lucee has three date classes that differ only in
/// their string form: `DateTimeImpl` (`{ts '…'}`), `DateImpl` (`{d '…'}`, from
/// `createODBCDate`) and `TimeImpl` (`{t '…'}`, from `createTime` /
/// `createODBCTime`). Any arithmetic produces a plain date-time again.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DateKind {
    DateTime,
    Date,
    Time,
}

/// Sub-second precision a date keeps when it is made (parsed, read from a
/// database, `now()`, `createDateTime`). Milliseconds is Lucee's precision.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Precision {
    Millisecond,
    Microsecond,
    Nanosecond,
}

impl Precision {
    /// Size of one unit of this precision, in nanoseconds.
    #[inline]
    pub fn quantum_nanos(self) -> u32 {
        match self {
            Precision::Millisecond => 1_000_000,
            Precision::Microsecond => 1_000,
            Precision::Nanosecond => 1,
        }
    }

    pub fn parse(s: &str) -> Option<Precision> {
        match s.trim().to_ascii_lowercase().as_str() {
            "millisecond" | "milliseconds" | "milli" | "ms" | "l" => Some(Precision::Millisecond),
            "microsecond" | "microseconds" | "micro" | "us" => Some(Precision::Microsecond),
            "nanosecond" | "nanoseconds" | "nano" | "ns" => Some(Precision::Nanosecond),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Precision::Millisecond => "millisecond",
            Precision::Microsecond => "microsecond",
            Precision::Nanosecond => "nanosecond",
        }
    }
}

thread_local! {
    static PRECISION: Cell<Precision> = const { Cell::new(Precision::Millisecond) };
}

/// The precision new dates are made at on this thread (the request's).
#[inline]
pub fn precision() -> Precision {
    PRECISION.with(|p| p.get())
}

/// Set the precision new dates are made at (per request; the VM sets it from
/// `this.datePrecision`).
pub fn set_precision(p: Precision) {
    PRECISION.with(|c| c.set(p));
}

const NANOS_PER_SEC: i64 = 1_000_000_000;
/// Milliseconds between 1899-12-30 (CFML day 0) and 1970-01-01.
const CF_UNIX_OFFSET_MS: i64 = 2_209_161_600_000;
const DAY_MS: f64 = 86_400_000.0;
/// The last moment it was 1899-12-30 anywhere on Earth. Lucee renders dates up
/// to here (the synthetic `createTime` base) with the zone's present standard
/// offset rather than its historical local-mean-time rules.
const LAST_1899_MOMENT_MS: i64 = -2_208_859_201_000;

/// A CFML date: an instant with nanosecond resolution, plus the class it
/// prints as.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct CfmlDate {
    secs: i64,
    nanos: u32,
    kind: DateKind,
}

impl std::fmt::Debug for CfmlDate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "CfmlDate({}, {:?})", self.utc_iso(), self.kind)
    }
}

// ─────────────────────────────────────────────
// Zones
// ─────────────────────────────────────────────

/// The host's zone, resolved once (the JVM caches its default zone too).
pub fn system_zone() -> Tz {
    static SYSTEM: OnceLock<Tz> = OnceLock::new();
    *SYSTEM.get_or_init(|| zone::resolve_tz(&zone::system_tz_id()).unwrap_or(Tz::UTC))
}

/// The zone dates are read and written in for this request.
#[inline]
pub fn current_zone() -> Tz {
    crate::clock::request_tz().unwrap_or_else(system_zone)
}

/// UTC offset (seconds, east positive) of `tz` at the instant `epoch_ms`.
pub fn offset_secs_at(tz: &Tz, epoch_ms: i64) -> i32 {
    if epoch_ms <= LAST_1899_MOMENT_MS {
        return standard_offset_secs(tz);
    }
    let secs = epoch_ms.div_euclid(1000);
    match chrono::DateTime::from_timestamp(secs, 0) {
        Some(dt) => tz.offset_from_utc_datetime(&dt.naive_utc()).fix().local_minus_utc(),
        None => 0,
    }
}

/// DST saving (seconds) in force in `tz` at the instant `epoch_ms`.
pub fn dst_secs_at(tz: &Tz, epoch_ms: i64) -> i32 {
    if epoch_ms <= LAST_1899_MOMENT_MS {
        return 0;
    }
    let secs = epoch_ms.div_euclid(1000);
    match chrono::DateTime::from_timestamp(secs, 0) {
        Some(dt) => tz.offset_from_utc_datetime(&dt.naive_utc()).dst_offset().num_seconds() as i32,
        None => 0,
    }
}

/// The zone's present-day standard (non-DST) offset.
fn standard_offset_secs(tz: &Tz) -> i32 {
    let now = chrono::DateTime::from_timestamp_millis(crate::clock::now_unix_millis() as i64)
        .map(|d| d.naive_utc())
        .unwrap_or_default();
    tz.offset_from_utc_datetime(&now).base_utc_offset().num_seconds() as i32
}

/// Resolve a wall-clock time in `tz` to an instant (epoch seconds), the way
/// `java.util.GregorianCalendar` does — which is what Lucee uses:
///
/// - a time inside a spring-forward gap is read with the offset in force
///   *before* the gap, so it lands after it (London 01:30 on the change day is
///   02:30 BST);
/// - a time that occurs twice at a fall-back is read as *standard* time, the
///   later of the two.
pub fn local_to_epoch_secs(tz: &Tz, local: &NaiveDateTime) -> i64 {
    let naive_secs = local.and_utc().timestamp();
    // Synthetic pre-1900 dates (createTime's 1899-12-30 base) use the fixed
    // standard offset, mirroring how they are rendered.
    if naive_secs.saturating_mul(1000) <= LAST_1899_MOMENT_MS + 86_400_000 {
        return naive_secs - standard_offset_secs(tz) as i64;
    }
    match tz.offset_from_local_datetime(local) {
        chrono::LocalResult::Single(off) => naive_secs - off.fix().local_minus_utc() as i64,
        chrono::LocalResult::Ambiguous(a, b) => {
            let ia = naive_secs - a.fix().local_minus_utc() as i64;
            let ib = naive_secs - b.fix().local_minus_utc() as i64;
            ia.max(ib)
        }
        chrono::LocalResult::None => {
            // Offset before the transition: read a day earlier.
            let before = local
                .checked_sub_signed(chrono::Duration::days(1))
                .map(|b| tz.offset_from_utc_datetime(&b).fix().local_minus_utc())
                .unwrap_or(0);
            naive_secs - before as i64
        }
    }
}

// ─────────────────────────────────────────────
// The value
// ─────────────────────────────────────────────

impl CfmlDate {
    /// An instant from epoch seconds + nanoseconds (normalised).
    pub fn from_epoch(secs: i64, nanos: i64) -> CfmlDate {
        let total_secs = secs + nanos.div_euclid(NANOS_PER_SEC);
        let n = nanos.rem_euclid(NANOS_PER_SEC) as u32;
        CfmlDate { secs: total_secs, nanos: n, kind: DateKind::DateTime }
    }

    pub fn from_epoch_millis(ms: i64) -> CfmlDate {
        CfmlDate::from_epoch(ms.div_euclid(1000), ms.rem_euclid(1000) * 1_000_000)
    }

    /// The current instant, at the request's precision (truncated, as
    /// `System.currentTimeMillis()` is).
    pub fn now() -> CfmlDate {
        let nanos = crate::clock::now_unix_nanos() as i128;
        let q = precision().quantum_nanos() as i128;
        let n = nanos - nanos.rem_euclid(q);
        CfmlDate::from_epoch((n / 1_000_000_000) as i64, (n % 1_000_000_000) as i64)
    }

    /// A wall-clock time in `tz`, rounded to the request's precision.
    pub fn from_local_in(tz: &Tz, local: &NaiveDateTime) -> CfmlDate {
        let secs = local_to_epoch_secs(tz, local);
        CfmlDate::from_epoch(secs, local.and_utc().timestamp_subsec_nanos() as i64).rounded()
    }

    /// A wall-clock time in the request zone.
    pub fn from_local(local: &NaiveDateTime) -> CfmlDate {
        CfmlDate::from_local_in(&current_zone(), local)
    }

    /// A UTC wall-clock time (an absolute instant).
    pub fn from_utc(utc: &NaiveDateTime) -> CfmlDate {
        let u = utc.and_utc();
        CfmlDate::from_epoch(u.timestamp(), u.timestamp_subsec_nanos() as i64).rounded()
    }

    /// Round to the request's precision (half up), as Lucee rounds a parsed
    /// fraction to the nearest millisecond (`.9995` → the next second).
    pub fn rounded(self) -> CfmlDate {
        self.rounded_to(precision())
    }

    pub fn rounded_to(self, p: Precision) -> CfmlDate {
        let q = p.quantum_nanos();
        if q == 1 {
            return self;
        }
        let rem = self.nanos % q;
        if rem == 0 {
            return self;
        }
        let down = (self.nanos - rem) as i64;
        let n = if rem >= q / 2 { down + q as i64 } else { down };
        CfmlDate { kind: self.kind, ..CfmlDate::from_epoch(self.secs, n) }
    }

    /// Truncate to the given precision.
    pub fn truncated_to(self, p: Precision) -> CfmlDate {
        let q = p.quantum_nanos();
        CfmlDate { nanos: self.nanos - self.nanos % q, ..self }
    }

    #[inline]
    pub fn kind(&self) -> DateKind {
        self.kind
    }

    #[inline]
    pub fn with_kind(self, kind: DateKind) -> CfmlDate {
        CfmlDate { kind, ..self }
    }

    #[inline]
    pub fn epoch_secs(&self) -> i64 {
        self.secs
    }

    /// Nanoseconds within the second (0..1e9).
    #[inline]
    pub fn subsec_nanos(&self) -> u32 {
        self.nanos
    }

    /// Epoch milliseconds, as `Date.getTime()` (floored).
    #[inline]
    pub fn epoch_millis(&self) -> i64 {
        self.secs * 1000 + (self.nanos / 1_000_000) as i64
    }

    /// Epoch nanoseconds (i128: an i64 only spans 1677–2262).
    pub fn epoch_nanos(&self) -> i128 {
        self.secs as i128 * 1_000_000_000 + self.nanos as i128
    }

    pub fn from_epoch_nanos(n: i128) -> CfmlDate {
        let secs = n.div_euclid(1_000_000_000) as i64;
        let nanos = n.rem_euclid(1_000_000_000) as i64;
        CfmlDate::from_epoch(secs, nanos)
    }

    /// Add a signed number of nanoseconds.
    pub fn plus_nanos(self, n: i128) -> CfmlDate {
        CfmlDate::from_epoch_nanos(self.epoch_nanos() + n)
    }

    pub fn plus_millis(self, ms: i64) -> CfmlDate {
        self.plus_nanos(ms as i128 * 1_000_000)
    }

    /// The instant as a UTC wall clock.
    pub fn utc(&self) -> NaiveDateTime {
        chrono::DateTime::from_timestamp(self.secs, self.nanos)
            .map(|d| d.naive_utc())
            .unwrap_or_default()
    }

    fn utc_iso(&self) -> String {
        self.utc().format("%Y-%m-%dT%H:%M:%S%.9fZ").to_string()
    }

    /// UTC offset (seconds) of `tz` at this instant.
    pub fn offset_secs_in(&self, tz: &Tz) -> i32 {
        offset_secs_at(tz, self.epoch_millis())
    }

    /// The wall clock in `tz`.
    pub fn local_in(&self, tz: &Tz) -> NaiveDateTime {
        let off = self.offset_secs_in(tz) as i64;
        chrono::DateTime::from_timestamp(self.secs + off, self.nanos)
            .map(|d| d.naive_utc())
            .unwrap_or_default()
    }

    /// The wall clock in the request zone.
    pub fn local(&self) -> NaiveDateTime {
        self.local_in(&current_zone())
    }

    /// Lucee's numeric date: days since 1899-12-30 in the request zone
    /// (`DateTimeUtil.toDoubleValue`). Sub-millisecond parts do not count.
    pub fn to_numeric(&self) -> f64 {
        let tz = current_zone();
        let t = self.epoch_millis();
        let off = offset_secs_at(&tz, t) as i64 * 1000;
        (t + off + CF_UNIX_OFFSET_MS) as f64 / DAY_MS
    }

    /// The inverse, `DateTimeUtil.toDateTime(double)`. Lucee takes the zone
    /// offset at the approximate instant, so this follows it exactly.
    pub fn from_numeric(days: f64) -> Option<CfmlDate> {
        if !days.is_finite() {
            return None;
        }
        let ms = days * DAY_MS;
        if ms.abs() > 8.0e18 {
            return None;
        }
        // Java Math.round: floor(x + 0.5).
        let mut utc = (ms + 0.5).floor() as i64;
        utc -= CF_UNIX_OFFSET_MS;
        utc -= offset_secs_at(&current_zone(), utc) as i64 * 1000;
        Some(CfmlDate::from_epoch_millis(utc))
    }

    /// The string a date converts to: `{ts 'yyyy-MM-dd HH:mm:ss'}`,
    /// `{d 'yyyy-MM-dd'}` or `{t 'HH:mm:ss'}`, in the request zone.
    pub fn to_cfml_string(&self) -> String {
        let l = self.local();
        match self.kind {
            DateKind::DateTime => format!(
                "{{ts '{}-{:02}-{:02} {:02}:{:02}:{:02}'}}",
                fmt_year(l.year()),
                l.month(),
                l.day(),
                l.hour(),
                l.minute(),
                l.second()
            ),
            DateKind::Date => format!("{{d '{}-{:02}-{:02}'}}", fmt_year(l.year()), l.month(), l.day()),
            DateKind::Time => format!("{{t '{:02}:{:02}:{:02}'}}", l.hour(), l.minute(), l.second()),
        }
    }

    /// `yyyy-MM-dd HH:mm:ss[.fff]` in the request zone: the plain form a
    /// database or another engine reads back.
    pub fn to_plain_string(&self) -> String {
        let l = self.local();
        let mut s = format!(
            "{}-{:02}-{:02} {:02}:{:02}:{:02}",
            fmt_year(l.year()),
            l.month(),
            l.day(),
            l.hour(),
            l.minute(),
            l.second()
        );
        if self.nanos != 0 {
            let frac = format!("{:09}", self.nanos);
            s.push('.');
            s.push_str(frac.trim_end_matches('0'));
        }
        s
    }

    /// The text a database without a date type stores (SQLite, the worker's
    /// wire protocol): `yyyy-MM-dd HH:mm:ss[.fff]`, or just the date / time
    /// part for a `{d}` / `{t}` value.
    pub fn to_db_text(&self) -> String {
        match self.kind {
            DateKind::DateTime => self.to_plain_string(),
            DateKind::Date => {
                let l = self.local();
                format!("{}-{:02}-{:02}", fmt_year(l.year()), l.month(), l.day())
            }
            DateKind::Time => {
                let plain = self.to_plain_string();
                plain.split_once(' ').map(|(_, t)| t.to_string()).unwrap_or(plain)
            }
        }
    }

    /// Compare at Lucee's operator resolution: whole seconds of `getTime()`,
    /// truncated toward zero as Java's `long / 1000L` is.
    pub fn cmp_seconds(&self, other: &CfmlDate) -> std::cmp::Ordering {
        (self.epoch_millis() / 1000).cmp(&(other.epoch_millis() / 1000))
    }
}

/// The HTTP date form, `Fri, 02 Jan 2026 10:20:30 GMT` (always in GMT).
/// `with_dash` gives the cookie variant `Fri, 02-Jan-2026 10:20:30 GMT`, which
/// Lucee also uses in its "Can't cast Date [...]" messages.
pub fn http_string(d: &CfmlDate, with_dash: bool) -> String {
    let u = d.utc();
    const DAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
    const MONTHS: [&str; 12] =
        ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    let sep = if with_dash { '-' } else { ' ' };
    format!(
        "{}, {:02}{}{}{}{} {:02}:{:02}:{:02} GMT",
        DAYS[u.weekday().num_days_from_monday() as usize],
        u.day(),
        sep,
        MONTHS[u.month0() as usize],
        sep,
        fmt_year(u.year()),
        u.hour(),
        u.minute(),
        u.second()
    )
}

/// A value as a date, the way Lucee's `Caster.toDate` reads it: a date as-is,
/// a number as a CFML numeric date, a string through the date parser, a
/// `java.util.Date` shim by its epoch milliseconds.
pub fn value_to_date(v: &crate::dynamic::CfmlValue) -> Option<CfmlDate> {
    use crate::dynamic::CfmlValue;
    match v {
        CfmlValue::DateTime(d) => Some(*d),
        CfmlValue::Int(i) => CfmlDate::from_numeric(*i as f64),
        CfmlValue::Double(f) | CfmlValue::TimeSpan(f) => CfmlDate::from_numeric(*f),
        CfmlValue::Bool(b) => CfmlDate::from_numeric(if *b { 1.0 } else { 0.0 }),
        CfmlValue::String(s) => {
            let t = s.trim();
            if t.is_empty() {
                None
            } else {
                parse::parse_date(t)
            }
        }
        CfmlValue::QueryColumn(..) => {
            let scalar = v.query_column_scalar();
            if matches!(scalar, CfmlValue::QueryColumn(..)) {
                None
            } else {
                value_to_date(scalar)
            }
        }
        CfmlValue::Struct(s) => s.data_get("__millis").and_then(|ms| match ms {
            CfmlValue::Int(n) => Some(CfmlDate::from_epoch_millis(n)),
            CfmlValue::Double(d) => Some(CfmlDate::from_epoch_millis(d as i64)),
            other => other.as_string().trim().parse::<i64>().ok().map(CfmlDate::from_epoch_millis),
        }),
        _ => None,
    }
}

/// A calendar field a date member setter writes (`d.setDay(5)`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DateField {
    Year,
    Month,
    Day,
    Hour,
    Minute,
    Second,
    Millisecond,
}

/// `Calendar.set(field, value)` on a lenient calendar: an out-of-range value
/// rolls into the neighbouring field (`setDay(40)` in January is 9 February,
/// `setMonth(2)` on 31 January is 3 March), as Lucee's date setters do.
pub fn set_field(d: CfmlDate, tz: &Tz, field: DateField, value: i64) -> CfmlDate {
    let w = d.local_in(tz);
    let (mut y, mut mo, mut day) = (w.year() as i64, w.month() as i64, w.day() as i64);
    let (mut h, mut mi, mut s) = (w.hour() as i64, w.minute() as i64, w.second() as i64);
    let mut nanos = d.subsec_nanos() as i64;
    match field {
        DateField::Year => y = value,
        DateField::Month => mo = value,
        DateField::Day => day = value,
        DateField::Hour => h = value,
        DateField::Minute => mi = value,
        DateField::Second => s = value,
        DateField::Millisecond => nanos = value.saturating_mul(1_000_000),
    }
    let total_months = y * 12 + (mo - 1);
    let (ny, nm) = (total_months.div_euclid(12), total_months.rem_euclid(12) + 1);
    let Some(base) = i32::try_from(ny)
        .ok()
        .and_then(|yy| NaiveDate::from_ymd_opt(yy, nm as u32, 1))
        .and_then(|nd| nd.and_hms_opt(0, 0, 0))
    else {
        return d;
    };
    let offset = chrono::Duration::try_days(day - 1)
        .and_then(|x| x.checked_add(&chrono::Duration::try_hours(h)?))
        .and_then(|x| x.checked_add(&chrono::Duration::try_minutes(mi)?))
        .and_then(|x| x.checked_add(&chrono::Duration::try_seconds(s)?))
        .and_then(|x| x.checked_add(&chrono::Duration::nanoseconds(nanos)));
    let Some(wall) = offset.and_then(|o| base.checked_add_signed(o)) else {
        return d;
    };
    CfmlDate::from_epoch(local_to_epoch_secs(tz, &wall), wall.and_utc().timestamp_subsec_nanos() as i64)
        .with_kind(d.kind())
}

/// A filesystem / system time as a date, at millisecond precision (Java's
/// `File.lastModified()`), or the current precision if that is coarser.
pub fn from_system_time(t: std::time::SystemTime) -> CfmlDate {
    let (secs, nanos) = match t.duration_since(std::time::UNIX_EPOCH) {
        Ok(d) => (d.as_secs() as i64, d.subsec_nanos() as i64),
        Err(e) => {
            let d = e.duration();
            (-(d.as_secs() as i64), -(d.subsec_nanos() as i64))
        }
    };
    CfmlDate::from_epoch(secs, nanos).truncated_to(Precision::Millisecond)
}

/// Epoch seconds as a date.
pub fn from_epoch_secs(secs: i64) -> CfmlDate {
    CfmlDate::from_epoch(secs, 0)
}

/// A year the way Java's `yyyy` prints it: at least four digits.
fn fmt_year(y: i32) -> String {
    if y < 0 {
        format!("-{:04}", -(y as i64))
    } else {
        format!("{:04}", y)
    }
}

/// Days in a month (Gregorian).
pub fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if is_leap_year(year) {
                29
            } else {
                28
            }
        }
        _ => 30,
    }
}

pub fn is_leap_year(year: i32) -> bool {
    (year % 4 == 0) && ((year % 100 != 0) || (year % 400 == 0))
}

/// A calendar wall clock from its fields (no validation beyond chrono's).
pub fn naive(year: i32, month: u32, day: u32, h: u32, mi: u32, s: u32, nanos: u32) -> Option<NaiveDateTime> {
    NaiveDate::from_ymd_opt(year, month, day)?.and_hms_nano_opt(h, mi, s, nanos)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ndt(y: i32, m: u32, d: u32, h: u32, mi: u32) -> NaiveDateTime {
        naive(y, m, d, h, mi, 0, 0).unwrap()
    }

    #[test]
    fn gap_moves_forward_overlap_takes_standard_time() {
        let london = zone::resolve_tz("Europe/London").unwrap();
        // 2026-03-29 01:30 does not exist in London: Java reads it with GMT,
        // giving 01:30Z = 02:30 BST.
        let gap = CfmlDate::from_local_in(&london, &ndt(2026, 3, 29, 1, 30));
        assert_eq!(gap.epoch_millis(), 1_774_747_800_000);
        assert_eq!(gap.local_in(&london), ndt(2026, 3, 29, 2, 30));
        // 2026-10-25 01:30 happens twice: Java picks GMT (the later), the
        // value Lucee 7.1 returned for createDateTime(...).getTime().
        let overlap = CfmlDate::from_local_in(&london, &ndt(2026, 10, 25, 1, 30));
        assert_eq!(overlap.epoch_millis(), 1_792_891_800_000);
    }

    #[test]
    fn numeric_round_trip_matches_lucee() {
        // Lucee 7.1 (Europe/London): createDateTime(2026,1,2,10,20,30) is
        // 46024.43090277778; +250ms is 46024.430905671295.
        let london = zone::resolve_tz("Europe/London").unwrap();
        crate::clock::set_request_timezone(Some("Europe/London".into()));
        let d = CfmlDate::from_local_in(&london, &ndt(2026, 1, 2, 10, 20).with_second(30).unwrap());
        assert_eq!(d.to_numeric(), 46024.43090277778);
        assert_eq!(d.plus_millis(250).to_numeric(), 46024.430905671295);
        let back = CfmlDate::from_numeric(46024.43090277778).unwrap();
        assert_eq!(back.epoch_millis(), d.epoch_millis());
        crate::clock::set_request_timezone(None);
    }

    #[test]
    fn rounding_is_half_up_and_carries() {
        let d = CfmlDate::from_epoch(10, 999_500_000).rounded_to(Precision::Millisecond);
        assert_eq!((d.epoch_secs(), d.subsec_nanos()), (11, 0));
        let d = CfmlDate::from_epoch(10, 123_499_999).rounded_to(Precision::Millisecond);
        assert_eq!(d.subsec_nanos(), 123_000_000);
        let d = CfmlDate::from_epoch(-1, 500_000).rounded_to(Precision::Millisecond);
        assert_eq!((d.epoch_secs(), d.subsec_nanos()), (-1, 1_000_000));
    }

    #[test]
    fn compare_truncates_toward_zero_like_java() {
        let a = CfmlDate::from_epoch_millis(-1500);
        let b = CfmlDate::from_epoch_millis(-1001);
        // -1500/1000 = -1 and -1001/1000 = -1 in Java: equal.
        assert_eq!(a.cmp_seconds(&b), std::cmp::Ordering::Equal);
    }
}
