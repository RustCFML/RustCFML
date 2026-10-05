//! The CFML date parser: turns the many textual date forms CFML accepts into
//! a date value. Moved here from `cfml-stdlib` so the VM (date-aware `==`/`<`),
//! the stdlib date functions and the database layer share one parser.

use super::{CfmlDate, DateKind};
use chrono::{NaiveDate, NaiveDateTime, NaiveTime};

/// What a date string names.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Parsed {
    /// A wall-clock time, read in the request zone.
    Local(NaiveDateTime),
    /// A time-only ODBC literal (`{t '…'}`) on the 1899-12-30 base.
    Time(NaiveDateTime),
    /// An absolute instant (the string carried an offset), as UTC.
    Instant(NaiveDateTime),
}

impl Parsed {
    /// The date value, in the request zone, at the request's precision.
    pub fn to_date(self) -> CfmlDate {
        match self {
            Parsed::Local(l) => CfmlDate::from_local(&l),
            Parsed::Time(l) => CfmlDate::from_local(&l).with_kind(DateKind::Time),
            Parsed::Instant(u) => CfmlDate::from_utc(&u),
        }
    }

    /// The wall clock this names in the request zone.
    pub fn to_local(self) -> NaiveDateTime {
        match self {
            Parsed::Local(l) | Parsed::Time(l) => l,
            Parsed::Instant(_) => self.to_date().local(),
        }
    }
}

/// Parse a date string to a date value.
pub fn parse_date(s: &str) -> Option<CfmlDate> {
    parse(s).map(Parsed::to_date)
}

/// Parse a date string to a wall clock in the request zone.
pub fn parse_local(s: &str) -> Option<NaiveDateTime> {
    parse(s).map(Parsed::to_local)
}

/// Lucee's base date for times: 1899-12-30, CFML day zero.
fn time_base() -> Option<NaiveDate> {
    NaiveDate::from_ymd_opt(1899, 12, 30)
}

/// Parse an ODBC literal: `{d '...'}`, `{t '...'}`, `{ts '...'}`. A `{t}`
/// literal is a time on Lucee's 1899-12-30 base and keeps its kind.
fn parse_odbc_literal(s: &str) -> Option<Parsed> {
    let start = s.find('\'')?;
    let end = s.rfind('\'')?;
    if start >= end { return None; }
    // `{ts '…'}` and nothing else: a literal with trailing text is not a date.
    if s[end + 1..].trim() != "}" {
        return None;
    }
    let inner = &s[start+1..end];
    let lower = s.to_lowercase();
    if lower.starts_with("{ts ") {
        NaiveDateTime::parse_from_str(inner, "%Y-%m-%d %H:%M:%S%.f").ok().map(Parsed::Local)
    } else if lower.starts_with("{d ") {
        NaiveDate::parse_from_str(inner, "%Y-%m-%d").ok()
            .and_then(|d| d.and_hms_opt(0, 0, 0))
            .map(Parsed::Local)
    } else if lower.starts_with("{t ") {
        NaiveTime::parse_from_str(inner, "%H:%M:%S%.f").ok()
            .and_then(|t| time_base().map(|d| d.and_time(t)))
            .map(Parsed::Time)
    } else {
        None
    }
}


/// Expand a bare year the way Lucee's date parser does: a value below 100 is a
/// two-digit year in the fixed window **1930–2029**.
///
/// Probed on Lucee 7.1.0 — `1.1.29` → 2029 but `1.1.30` → 1930, `1.1.00` → 2000,
/// `1.1.99` → 1999. Note this is a FIXED window, not the JDK's default
/// "80 years before now" sliding one (which in 2026 would put the break at 1946
/// and read `30` as 2030). Three-or-more-digit values are literal years, so
/// `1.1.026` is 2026 and `1.1.2020` is 2020.
pub fn expand_two_digit_year(y: i32) -> i32 {
    match y {
        0..=29 => 2000 + y,
        30..=99 => 1900 + y,
        other => other,
    }
}

/// Parse Lucee's dot-separated date forms: `A.B.C`, optionally followed by a
/// time (`5.3.2 10:30:00`).
///
/// Which of A/B/C is the day, month and year is decided positionally, exactly as
/// Lucee decides it — verified against Lucee 7.1.0 across 35 probes:
///
/// | input | Lucee | rule |
/// |---|---|---|
/// | `2020.11.5` | 2020-11-05 | A is a full year → Y.M.D |
/// | `31.12.2020` | 2020-12-31 | A can't be a month, C is a full year → D.M.Y |
/// | `13.1.2` | 2013-01-02 | A can't be a month, C isn't a year → Y.M.D |
/// | `5.3.2` | 2002-05-03 | A could be a month → **M.D.Y** |
/// | `10.20.30` | 1930-10-20 | M.D.Y, and `30` is 1930 |
/// | `0.1.2` | rejected | month 0 — the M.D.Y reading is the ONLY one tried |
///
/// The last row is the subtle one and the reason this is not simply "try every
/// ordering": `0.1.2` would parse fine as Y.M.D (2000-01-02), and Lucee still
/// rejects it. A component that *could* be a month commits the string to M.D.Y,
/// pass or fail. Getting that wrong would make us accept dates Lucee refuses,
/// which for GH #411 means accepting a `numeric` argument Lucee rejects.
///
/// This is how `numeric` accepts `"5.3.2"`: Lucee's `numeric` cast falls back to
/// a date cast, and `5.3.2` is 3 May 2002. It is not a version-string rule — the
/// acceptance set tracks date validity exactly, leap years included (`2.29.2004`
/// passes, `2.29.2005` does not).
fn parse_dotted_date(s: &str) -> Option<NaiveDateTime> {
    let (date_part, time_part) = match s.split_once(char::is_whitespace) {
        Some((d, t)) => (d, t.trim()),
        None => (s, ""),
    };

    let mut parts = date_part.split('.');
    let (a, b, c) = (parts.next()?, parts.next()?, parts.next()?);
    if parts.next().is_some() {
        return None; // four or more components is not a date on either engine
    }
    // Digits only: no signs, no exponents, no empty components. This is what
    // keeps `1.2.3.4`, `-1.2.3`, `5..3` and `a.b.c` out.
    if [a, b, c].iter().any(|p| p.is_empty() || !p.bytes().all(|ch| ch.is_ascii_digit())) {
        return None;
    }
    let (a, b, c): (i32, i32, i32) = (a.parse().ok()?, b.parse().ok()?, c.parse().ok()?);

    let (year, month, day) = if a >= 100 {
        (a, b, c)
    } else if a > 12 {
        if c >= 100 { (c, b, a) } else { (a, b, c) }
    } else {
        (c, a, b)
    };

    let date = NaiveDate::from_ymd_opt(
        expand_two_digit_year(year),
        u32::try_from(month).ok()?,
        u32::try_from(day).ok()?,
    )?;

    if time_part.is_empty() {
        return date.and_hms_opt(0, 0, 0);
    }
    for fmt in &["%H:%M:%S", "%I:%M:%S %p", "%H:%M", "%I:%M %p"] {
        if let Ok(t) = NaiveTime::parse_from_str(time_part, fmt) {
            return Some(date.and_time(t));
        }
    }
    None
}

/// Central date parser: ODBC literals, ISO 8601, common US/EU forms, dotted
/// dates, time-only and a date serial number.
pub fn parse(s: &str) -> Option<Parsed> {
    let s = s.trim();
    if s.is_empty() { return None; }

    // ODBC literals
    if s.starts_with('{') {
        return parse_odbc_literal(s);
    }

    // DateTime formats (most specific first). `%.f` optionally consumes a
    // fractional-seconds component (".177"), so the fractional variants also
    // match values with no fraction.
    for fmt in &[
        "%Y-%m-%dT%H:%M:%S%.f",
        "%Y-%m-%d %H:%M:%S%.f",
        "%Y-%m-%d %H:%M:%S",
        "%Y-%m-%dT%H:%M:%S",
        "%Y-%m-%d %H:%M",
        // Slash-separated ISO order — `2020/1/2`, which Lucee parses (and
        // `isDate()`/`isValid("date",…)` accept) but we used to reject while
        // accepting the dashed form. Unambiguous against `%m/%d/%Y` below: a
        // leading 4-digit year can't be a month, and a leading month can't be
        // a year with a valid day left over.
        "%Y/%m/%d %H:%M:%S",
        "%Y/%m/%d %H:%M",
        "%m/%d/%Y %H:%M:%S",
        "%m/%d/%Y %I:%M:%S %p",
        "%m/%d/%Y %I:%M %p",
        "%m/%d/%Y %H:%M",
        "%d %b %Y %H:%M:%S",
        "%b %d, %Y %H:%M:%S",
        "%B %d, %Y %H:%M:%S",
        "%d-%b-%Y %H:%M:%S",
        // Month-name forms without a comma (Lucee/ACF accept these) — e.g.
        // "January 1 1970 00:00", used by cbsecurity's JwtService epoch base.
        "%B %d %Y %H:%M:%S",
        "%b %d %Y %H:%M:%S",
        "%B %d %Y %H:%M",
        "%b %d %Y %H:%M",
        // Lucee's own serializeJSON date form, offset-less variant:
        // "August, 25 2026 09:00:14". The comma sits after the MONTH here, not
        // after the day, so none of the "%B %d, %Y" patterns above match it.
        // See the offset-bearing variant just below (GH #365).
        "%B, %d %Y %H:%M:%S",
        "%b, %d %Y %H:%M:%S",
        "%B, %d %Y %H:%M",
        "%b, %d %Y %H:%M",
    ] {
        if let Ok(dt) = NaiveDateTime::parse_from_str(s, fmt) {
            return Some(Parsed::Local(dt));
        }
    }

    // Lucee's serializeJSON date form WITH the trailing UTC offset:
    // `serializeJSON({d: createDateTime(2026,8,25,9,0,14)})` emits
    // {"D":"August, 25 2026 09:00:14 +0000"} on Lucee 7.0.5, and Lucee's date
    // parser reads it straight back. We rejected it, so every date a Lucee
    // deployment had written into a JSON/jsonb column became unreadable after
    // switching engines — isDate() false, dateDiff "Invalid date2" (GH #365).
    // Our own serializeJSON writes "yyyy-mm-dd HH:mm:ss", which both engines
    // parse, so the gap is one-directional and this is the recovering half.
    // Wall-clock fields as written, matching the RFC 3339 branch below.
    for fmt in &["%B, %d %Y %H:%M:%S %z", "%b, %d %Y %H:%M:%S %z"] {
        if let Ok(dt) = chrono::DateTime::parse_from_str(s, fmt) {
            // An absolute instant: the request zone applies only when it is read.
            return Some(Parsed::Instant(dt.naive_utc()));
        }
    }

    // Date-only formats → midnight
    for fmt in &[
        "%Y-%m-%d",
        "%Y/%m/%d",
        "%m/%d/%Y",
        "%m-%d-%Y",
        "%d %b %Y",
        "%b %d, %Y",
        "%B %d, %Y",
        "%d-%b-%Y",
        "%B %d %Y",
        "%b %d %Y",
        // Month-comma order, date only (GH #365 — see the datetime list above).
        "%B, %d %Y",
        "%b, %d %Y",
    ] {
        if let Ok(d) = NaiveDate::parse_from_str(s, fmt) {
            return d.and_hms_opt(0, 0, 0).map(Parsed::Local);
        }
    }

    // Dot-separated dates — `5.3.2`, `31.12.2020`, `2020.11.5`, with an optional
    // trailing time. Lucee parses these; we rejected them outright (GH #411).
    if let Some(dt) = parse_dotted_date(s) {
        return Some(Parsed::Local(dt));
    }

    // Time-only → Lucee's base date 1899-12-30 (`parseDateTime("10:20:30")`
    // is `{ts '1899-12-30 10:20:30'}` on Lucee 7.1).
    for fmt in &["%H:%M:%S%.f", "%I:%M:%S %p", "%H:%M"] {
        if let Ok(t) = NaiveTime::parse_from_str(s, fmt) {
            return time_base().map(|d| Parsed::Local(d.and_time(t)));
        }
    }

    // RFC 3339 / ISO 8601 with a timezone offset or 'Z' suffix
    // ("2026-06-10T07:20:42.177+00:00", "...Z").
    //
    // The offset is HONOURED and the result expressed in the server's timezone,
    // which is what Lucee does for every offset-bearing form (probed on Lucee
    // 7.1.0.204 under Europe/London: "2026-08-25T09:00:14Z" -> 10:00:14,
    // "...-05:00" -> 15:00:14). We previously returned the wall-clock fields as
    // written, discarding the offset — so a stored UTC timestamp read back on a
    // non-UTC server was wrong by exactly that server's offset, and by six hours
    // for the -05:00 case. Invisible wherever the server runs in UTC (CI, most
    // containers), silently wrong everywhere else. Found alongside GH #365.
    //
    // `parse_cfml_datetime_utc` remains the accessor for the absolute instant.
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s) {
        return Some(Parsed::Instant(dt.naive_utc()));
    }

    // Date serial number (days since 1899-12-30, OLE Automation date)
    if let Ok(n) = s.parse::<f64>() {
        if n.is_finite() {
            let base = NaiveDate::from_ymd_opt(1899, 12, 30)?;
            let days = n.floor() as i64;
            let frac = n - n.floor();
            // Round to the nearest millisecond (NOT truncate) — the fraction is
            // a float, so 51 seconds can land at 50.9999s and truncate to 50.
            // Milliseconds also preserve sub-second precision through a
            // date -> serial -> date round-trip.
            let ms = (frac * 86_400_000.0).round() as i64;
            // try_days/try_milliseconds return None on overflow instead of
            // panicking ("TimeDelta::days out of bounds") — a bare epoch-millis
            // value like 1.7e12 parsed as a date serial would otherwise crash.
            return base.and_hms_opt(0, 0, 0)
                .and_then(|dt| chrono::Duration::try_days(days).and_then(|d| dt.checked_add_signed(d)))
                .and_then(|dt| chrono::Duration::try_milliseconds(ms).and_then(|d| dt.checked_add_signed(d)))
                .map(Parsed::Local);
        }
    }

    None
}

