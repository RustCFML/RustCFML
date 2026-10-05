//! Date and time functions (GH #441).
//!
//! Each function is a port of Lucee 7.1's implementation, working on the date
//! value in [`cfml_common::datetime`]. Calendar arithmetic follows
//! `java.util.GregorianCalendar`, which is what Lucee uses, including its
//! daylight-saving adjustments; the three formatters follow Lucee's
//! `DateFormat`, `TimeFormat` and `DateTimeFormat` (the last converts the CFML
//! mask to a `java.time` pattern, implemented here for the letters it can
//! produce).

use cfml_common::datetime::{self, parse, CfmlDate, DateKind};
use cfml_common::dynamic::CfmlValue;
use cfml_common::vm::{CfmlError, CfmlResult};
use chrono::{Datelike, Duration, NaiveDate, NaiveDateTime, Timelike};
use chrono_tz::{OffsetName, Tz};

// ─────────────────────────────────────────────
// Argument conversion
// ─────────────────────────────────────────────

/// A value as a date, the way Lucee's `Caster.toDate` reads it: a date as-is,
/// a number as a CFML numeric date, a string through the date parser.
pub fn to_date(v: &CfmlValue) -> Option<CfmlDate> {
    datetime::value_to_date(v)
}

/// Lucee's message for a value that is not a date.
pub fn cast_error(v: &CfmlValue) -> CfmlError {
    match v {
        CfmlValue::Array(_) => CfmlError::expression("can't cast Complex Object Type [Array] to a Date".into()),
        CfmlValue::Struct(_) => CfmlError::expression("can't cast Complex Object Type [Struct] to a Date".into()),
        CfmlValue::Query(_) => CfmlError::expression("can't cast Complex Object Type [Query] to a Date".into()),
        CfmlValue::Component(_) => CfmlError::expression("can't cast Complex Object Type [Component] to a Date".into()),
        other => CfmlError::expression(format!("can't cast [{}] to date value", other.as_string())),
    }
}

/// Argument `i` as a date, or Lucee's cast error.
pub fn arg_date(args: &[CfmlValue], i: usize) -> Result<CfmlDate, CfmlError> {
    let v = args.get(i).unwrap_or(&CfmlValue::Null);
    to_date(v).ok_or_else(|| cast_error(v))
}

/// Optional time-zone argument `i` (absent or empty: the request zone).
pub fn arg_tz(args: &[CfmlValue], i: usize) -> Result<Tz, CfmlError> {
    match args.get(i) {
        None | Some(CfmlValue::Null) => Ok(datetime::current_zone()),
        Some(v) => {
            let id = v.as_string();
            if id.trim().is_empty() {
                return Ok(datetime::current_zone());
            }
            datetime::zone::resolve_tz(&id)
                .ok_or_else(|| CfmlError::expression(format!("can't cast value [{}] to a TimeZone", id)))
        }
    }
}

/// A number argument, truncated to a whole number (`Caster.toLongValue`).
pub fn arg_long(args: &[CfmlValue], i: usize) -> Result<i64, CfmlError> {
    let v = args.get(i).unwrap_or(&CfmlValue::Null);
    to_long(v)
}

fn to_long(v: &CfmlValue) -> Result<i64, CfmlError> {
    match v.query_column_scalar() {
        CfmlValue::Int(i) => Ok(*i),
        CfmlValue::Double(d) | CfmlValue::TimeSpan(d) => Ok(d.trunc() as i64),
        CfmlValue::Bool(b) => Ok(*b as i64),
        CfmlValue::DateTime(d) => Ok(d.to_numeric().trunc() as i64),
        other => {
            let s = other.as_string();
            let t = s.trim();
            if let Ok(n) = t.parse::<i64>() {
                return Ok(n);
            }
            match cfml_common::numeric::numeric_string_value(t) {
                Some(f) => Ok(f.trunc() as i64),
                None => match t.to_ascii_lowercase().as_str() {
                    "true" | "yes" => Ok(1),
                    "false" | "no" => Ok(0),
                    _ if t.is_empty() => Err(CfmlError::expression("can't cast empty string to a number value".into())),
                    _ => Err(CfmlError::expression(format!("can't cast [{}] string to a number value", s))),
                },
            }
        }
    }
}

/// Lucee's `FunctionException` wording.
fn arg_error(func: &str, pos: usize, arg: &str, detail: &str) -> CfmlError {
    const ORD: [&str; 8] = ["first", "second", "third", "fourth", "fifth", "sixth", "seventh", "eighth"];
    CfmlError::expression(format!(
        "Invalid call of the function [{}], {} Argument [{}] is invalid, {}",
        func,
        ORD.get(pos.saturating_sub(1)).copied().unwrap_or("an"),
        arg,
        detail
    ))
}

fn date_value(d: CfmlDate) -> CfmlValue {
    CfmlValue::DateTime(d)
}

// ─────────────────────────────────────────────
// Calendar arithmetic (java.util.GregorianCalendar)
// ─────────────────────────────────────────────

/// A wall clock in `tz` as a date, keeping its sub-second part as given.
fn from_wall(tz: &Tz, wall: &NaiveDateTime) -> CfmlDate {
    CfmlDate::from_epoch(datetime::local_to_epoch_secs(tz, wall), wall.nanosecond() as i64)
}

/// `Calendar.add(DATE, days)`: same wall-clock time on the new day, read with
/// the old offset, then corrected for an offset change unless the correction
/// would move it off that day.
pub fn add_days(d: CfmlDate, tz: &Tz, days: i64) -> CfmlDate {
    let wall = d.local_in(tz);
    let Some(new_date) = Duration::try_days(days).and_then(|x| wall.date().checked_add_signed(x)) else {
        return d;
    };
    let new_wall = new_date.and_time(wall.time());
    let off0 = d.offset_secs_in(tz) as i64;
    let t1 = new_wall.and_utc().timestamp() - off0;
    let c1 = CfmlDate::from_epoch(t1, d.subsec_nanos() as i64);
    let diff = off0 - c1.offset_secs_in(tz) as i64;
    if diff != 0 {
        let c2 = CfmlDate::from_epoch(t1 + diff, d.subsec_nanos() as i64);
        if c2.local_in(tz).date() == new_date {
            return c2;
        }
    }
    c1
}

/// `Calendar.add(MONTH, months)`: the day is pinned to the new month's length.
pub fn add_months(d: CfmlDate, tz: &Tz, months: i64) -> CfmlDate {
    let wall = d.local_in(tz);
    let total = wall.year() as i64 * 12 + wall.month0() as i64 + months;
    let (y, m) = (total.div_euclid(12) as i32, total.rem_euclid(12) as u32 + 1);
    let day = wall.day().min(datetime::days_in_month(y, m));
    match NaiveDate::from_ymd_opt(y, m, day) {
        Some(nd) => from_wall(tz, &nd.and_time(wall.time())),
        None => d,
    }
}

/// `Calendar.set(YEAR, year)`: lenient, so 29 February in a common year rolls
/// to 1 March (Lucee's `dateAdd("yyyy")` does not pin the day).
fn set_year(d: CfmlDate, tz: &Tz, year: i64) -> CfmlDate {
    let wall = d.local_in(tz);
    let Ok(y) = i32::try_from(year) else { return d };
    match NaiveDate::from_ymd_opt(y, wall.month(), 1)
        .and_then(|first| first.checked_add_signed(Duration::days(wall.day() as i64 - 1)))
    {
        Some(nd) => from_wall(tz, &nd.and_time(wall.time())),
        None => d,
    }
}

/// Day of week, Sunday = 1 (`Calendar.DAY_OF_WEEK`).
fn dow(wall: &NaiveDateTime) -> i64 {
    wall.weekday().number_from_sunday() as i64
}

/// Week of the year, US rules (weeks start on Sunday; week 1 holds 1 January),
/// as `Calendar.WEEK_OF_YEAR` reports it: the last days of December fall in
/// week 1 when their week holds the next 1 January.
fn calendar_week_of_year(date: NaiveDate) -> i64 {
    let start_of_week = date - Duration::days(date.weekday().num_days_from_sunday() as i64);
    if let Some(next_jan1) = NaiveDate::from_ymd_opt(date.year() + 1, 1, 1) {
        if next_jan1 <= start_of_week + Duration::days(6) {
            return 1;
        }
    }
    continuing_week_of_year(date)
}

/// Lucee's `week()`: the US week count without the wrap into next year
/// (`DateTimeUtil.getWeekOfYear` counts those December days as week 53).
fn continuing_week_of_year(date: NaiveDate) -> i64 {
    let jan1 = NaiveDate::from_ymd_opt(date.year(), 1, 1).unwrap_or(date);
    (date.ordinal0() as i64 + jan1.weekday().num_days_from_sunday() as i64) / 7 + 1
}

/// Week of the month, US rules (`Calendar.WEEK_OF_MONTH`).
fn week_of_month(date: NaiveDate) -> i64 {
    let first = date.with_day(1).unwrap_or(date);
    (date.day0() as i64 + first.weekday().num_days_from_sunday() as i64) / 7 + 1
}

// ─────────────────────────────────────────────
// dateAdd / dateDiff / dateCompare / datePart
// ─────────────────────────────────────────────

/// Lucee `DateAdd._call`.
pub fn date_add(datepart: &str, number: i64, d: CfmlDate, tz: &Tz) -> Result<CfmlDate, CfmlError> {
    let part = datepart.to_lowercase();
    let n = number as i32 as i64; // Java narrows to int for the calendar parts
    let first = if part.chars().count() == 1 { part.chars().next().unwrap_or('\0') } else { '\0' };
    let r = match first {
        'l' => return Ok(d.plus_millis(number).with_kind(DateKind::DateTime)),
        's' => return Ok(d.plus_millis(number.saturating_mul(1000)).with_kind(DateKind::DateTime)),
        'n' => return Ok(d.plus_millis(number.saturating_mul(60_000)).with_kind(DateKind::DateTime)),
        'h' => return Ok(d.plus_millis(number.saturating_mul(3_600_000)).with_kind(DateKind::DateTime)),
        _ => {
            if part == "yyyy" {
                let y = d.local_in(tz).year() as i64;
                set_year(d, tz, y + n)
            } else if part == "ww" {
                add_days(d, tz, n * 7)
            } else if first == 'q' {
                add_months(d, tz, n * 3)
            } else if first == 'm' {
                add_months(d, tz, n)
            } else if first == 'y' || first == 'd' {
                add_days(d, tz, n)
            } else if first == 'w' {
                add_weekdays(d, tz, n)
            } else {
                return Err(CfmlError::expression(format!(
                    "invalid datepart identifier [{}] for function dateAdd",
                    datepart
                )));
            }
        }
    };
    Ok(r.with_kind(DateKind::DateTime))
}

/// Lucee's weekday addition (`dateAdd("w")`): Saturday and Sunday are skipped.
fn add_weekdays(d: CfmlDate, tz: &Tz, number: i64) -> CfmlDate {
    let mut n = number;
    let dw = dow(&d.local_in(tz));
    let offset = if n < 0 {
        if dw == 1 { 2 } else { -(6 - dw) }
    } else if dw == 7 {
        -2
    } else {
        dw - 2
    };
    let d1 = add_days(d, tz, -offset);
    if dw == 7 || dw == 1 {
        if n > 0 {
            n -= 1;
        } else if n < 0 {
            n += 1;
        }
    } else {
        n += offset;
    }
    add_days(d1, tz, (n / 5) * 7 + n % 5)
}

#[derive(Clone, Copy, PartialEq)]
enum DiffPart {
    D,
    Y,
    Yyyy,
    M,
    Ww,
    Wd,
    Q,
}

/// Lucee `DateDiff.call`.
pub fn date_diff(datepart: &str, left: CfmlDate, right: CfmlDate, tz: &Tz) -> Result<i64, CfmlError> {
    let ms_left = left.epoch_millis();
    let ms_right = right.epoch_millis();
    let part = datepart.trim().to_lowercase();
    let diff_seconds = || {
        if ms_left > ms_right {
            -((ms_left - ms_right) / 1000)
        } else {
            (ms_right - ms_left) / 1000
        }
    };
    let dp = match part.as_str() {
        "l" => return Ok(ms_right - ms_left),
        "s" => return Ok(diff_seconds()),
        "n" => return Ok(diff_seconds() / 60),
        "h" => return Ok(diff_seconds() / 3600),
        "d" => DiffPart::D,
        "y" => DiffPart::Y,
        "yyyy" => DiffPart::Yyyy,
        "m" => DiffPart::M,
        // Lucee keeps "w" as weeks (ACF changed it to weekdays; Lucee did not).
        "w" | "ww" => DiffPart::Ww,
        "wd" => DiffPart::Wd,
        "q" => DiffPart::Q,
        _ => {
            return Err(arg_error(
                "dateDiff",
                3,
                "datePart",
                &format!(
                    "invalid value [{}], valid values has to be [l, q, s, n, h, d, m, y, yyyy, w, ww, wd]",
                    part
                ),
            ))
        }
    };
    if ms_left > ms_right {
        Ok(-diff_call(dp, right, left, tz))
    } else {
        Ok(diff_call(dp, left, right, tz))
    }
}

/// Sign of the first non-zero difference among `diffs`: -1 if it is negative.
fn borrow(diffs: &[i64]) -> i64 {
    for &d in diffs {
        if d < 0 {
            return -1;
        }
        if d > 0 {
            return 0;
        }
    }
    0
}

fn diff_call(dp: DiffPart, left: CfmlDate, right: CfmlDate, tz: &Tz) -> i64 {
    let cl = left.local_in(tz);
    let cr = right.local_in(tz);
    let mut d_diff = cr.day() as i64 - cl.day() as i64;
    let h_diff = cr.hour() as i64 - cl.hour() as i64;
    let n_diff = cr.minute() as i64 - cl.minute() as i64;
    let s_diff = cr.second() as i64 - cl.second() as i64;
    match dp {
        DiffPart::Wd => working_days_diff(left, right, tz),
        DiffPart::D | DiffPart::Y | DiffPart::Ww => {
            let rst = day_diff(&cl, &cr) + borrow(&[h_diff, n_diff, s_diff]);
            if dp == DiffPart::Ww {
                rst / 7
            } else {
                rst
            }
        }
        DiffPart::Yyyy => {
            let y_diff = cr.year() as i64 - cl.year() as i64;
            let m_diff = cr.month() as i64 - cl.month() as i64;
            y_diff + borrow(&[m_diff, d_diff, h_diff, n_diff, s_diff])
        }
        DiffPart::M | DiffPart::Q => {
            let y_diff = cr.year() as i64 - cl.year() as i64;
            let m_diff = cr.month() as i64 - cl.month() as i64;
            if d_diff < 0 && cr.day() == datetime::days_in_month(cr.year(), cr.month()) {
                d_diff = 0;
            }
            let rst = m_diff + y_diff * 12 + borrow(&[d_diff, h_diff, n_diff, s_diff]);
            if dp == DiffPart::Q {
                rst / 3
            } else {
                rst
            }
        }
    }
}

fn day_diff(l: &NaiveDateTime, r: &NaiveDateTime) -> i64 {
    let (ly, ry) = (l.year(), r.year());
    let mut diff = r.ordinal() as i64 - l.ordinal() as i64;
    if ly == ry {
        return diff;
    }
    for y in ly..ry {
        diff += if datetime::is_leap_year(y) { 366 } else { 365 };
    }
    diff
}

/// Lucee `DateDiff.getWorkingDaysDiff`.
fn working_days_diff(left: CfmlDate, right: CfmlDate, tz: &Tz) -> i64 {
    let mut l = left;
    let mut r = right;
    let mut ldw = dow(&l.local_in(tz));
    let mut rdw = dow(&r.local_in(tz));
    if ldw == 1 {
        ldw = 6;
        l = add_days(l, tz, -2);
    } else if ldw == 7 {
        ldw = 6;
        l = add_days(l, tz, -1);
    }
    if rdw == 1 {
        rdw = 6;
        r = add_days(r, tz, -2);
    } else if rdw == 7 {
        rdw = 6;
        r = add_days(r, tz, -1);
    }
    let loff = ldw - 2;
    let roff = rdw - 2;
    l = add_days(l, tz, -loff);
    r = add_days(r, tz, -roff);
    let days = diff_call(DiffPart::D, l, r, tz);
    let weeks = diff_call(DiffPart::Ww, l, r, tz);
    (days - 2 * weeks) + roff - loff
}

/// Lucee `DateCompare.call`.
pub fn date_compare(left: CfmlDate, right: CfmlDate, datepart: &str, tz: &Tz) -> Result<i64, CfmlError> {
    let part = datepart.trim().to_lowercase();
    // 0 year, 1 month, 2 day, 3 hour, 4 minute, 5 second
    let depth = match part.as_str() {
        "s" => 5,
        "n" => 4,
        "h" => 3,
        "d" | "y" => 2,
        "m" => 1,
        "yyyy" => 0,
        _ => {
            return Err(arg_error(
                "dateCompare",
                3,
                "datePart",
                &format!("invalid value [{}], valid values has to be [s,n,h,d,m,y,yyyy]", part),
            ))
        }
    };
    let a = left.local_in(tz);
    let b = right.local_in(tz);
    let fields = [
        (a.year() as i64, b.year() as i64),
        (a.month() as i64, b.month() as i64),
        (a.day() as i64, b.day() as i64),
        (a.hour() as i64, b.hour() as i64),
        (a.minute() as i64, b.minute() as i64),
        (a.second() as i64, b.second() as i64),
    ];
    for (i, (x, y)) in fields.iter().enumerate() {
        if x != y {
            return Ok(if x > y { 1 } else { -1 });
        }
        if i == depth {
            return Ok(0);
        }
    }
    Ok(0)
}

/// Lucee `DatePart.call`.
pub fn date_part(datepart: &str, d: CfmlDate, tz: &Tz) -> Result<i64, CfmlError> {
    let part = datepart.to_lowercase();
    let l = d.local_in(tz);
    let first = if part.chars().count() == 1 { part.chars().next().unwrap_or('\0') } else { '\0' };
    Ok(if part == "yyyy" {
        l.year() as i64
    } else if part == "ww" {
        continuing_week_of_year(l.date())
    } else {
        match first {
            'w' => dow(&l),
            'q' => (l.month() as i64 - 1) / 3 + 1,
            'm' => l.month() as i64,
            'y' => l.ordinal() as i64,
            'd' => l.day() as i64,
            'h' => l.hour() as i64,
            'n' => l.minute() as i64,
            's' => l.second() as i64,
            'l' => (d.subsec_nanos() / 1_000_000) as i64,
            _ => {
                return Err(CfmlError::expression(format!(
                    "invalid datepart type [{}] for function datePart",
                    part
                )))
            }
        }
    })
}

// ─────────────────────────────────────────────
// Construction
// ─────────────────────────────────────────────

/// Lucee `DateTimeUtil.toYear`: years below 100 are two-digit (≤ 29 → 20xx).
fn to_year(y: i64) -> i64 {
    if y < 100 {
        if y < 30 {
            y + 2000
        } else {
            y + 1900
        }
    } else {
        y
    }
}

/// Lucee `DateTimeUtil.toTime` (the throwing variant) + `_toTime`: validate the
/// fields, then let the lenient calendar resolve hour 24 and milliseconds
/// beyond 999.
#[allow(clippy::too_many_arguments)]
pub fn make_date(
    tz: &Tz,
    year: i64,
    month: i64,
    day: i64,
    hour: i64,
    minute: i64,
    second: i64,
    milli: i64,
) -> Result<CfmlDate, CfmlError> {
    let year = to_year(year);
    let err = |m: String| CfmlError::expression(m);
    if month < 1 {
        return Err(err(format!("Month number [{}] must be at least 1", month)));
    }
    if month > 12 {
        return Err(err(format!("Month number [{}] can not be greater than 12", month)));
    }
    if day < 1 {
        return Err(err(format!("Day number [{}] must be at least 1", day)));
    }
    if hour < 0 {
        return Err(err(format!("Hour number [{}] must be at least 0", hour)));
    }
    if minute < 0 {
        return Err(err(format!("Minute number [{}] must be at least 0", minute)));
    }
    if second < 0 {
        return Err(err(format!("Second number [{}] must be at least 0", second)));
    }
    if milli < 0 {
        return Err(err(format!("Milli second number [{}] must be at least 0", milli)));
    }
    if hour > 24 {
        return Err(err(format!("Hour number [{}] can not be greater than 24", hour)));
    }
    if minute > 59 {
        return Err(err(format!("Minute number [{}] can not be greater than 59", minute)));
    }
    if second > 59 {
        return Err(err(format!("Second number [{}] can not be greater than 59", second)));
    }
    let y = i32::try_from(year).map_err(|_| err(format!("Year number [{}] is out of range", year)))?;
    let dim = datetime::days_in_month(y, month as u32) as i64;
    if day > dim {
        return Err(err(format!(
            "Day number [{}] can not be greater than {} when month is {} and year {}",
            day, dim, month, year
        )));
    }
    Ok(lenient_wall(tz, y, month, day, hour, minute, second, milli))
}

/// A wall clock from fields that may overflow (hour 24, 1500 ms, hour 25 for
/// `createTime`), rolled forward as a lenient `Calendar` does.
#[allow(clippy::too_many_arguments)]
fn lenient_wall(tz: &Tz, y: i32, month: i64, day: i64, hour: i64, minute: i64, second: i64, milli: i64) -> CfmlDate {
    let base = NaiveDate::from_ymd_opt(y, month.clamp(1, 12) as u32, 1)
        .and_then(|d| d.and_hms_opt(0, 0, 0))
        .unwrap_or_default();
    let offset = Duration::days(day - 1)
        + Duration::hours(hour)
        + Duration::minutes(minute)
        + Duration::seconds(second)
        + Duration::milliseconds(milli);
    let wall = base.checked_add_signed(offset).unwrap_or(base);
    from_wall(tz, &wall)
}

/// `createTime(h, m, s, ms)`: a time on Lucee's 1899-12-30 base. Lucee uses
/// the non-throwing validation here; an invalid field gives epoch 0.
pub fn make_time(tz: &Tz, hour: i64, minute: i64, second: i64, milli: i64) -> CfmlDate {
    let valid = (0..=24).contains(&hour) && (0..=59).contains(&minute) && (0..=59).contains(&second) && milli >= 0;
    let d = if valid || hour > 24 {
        // Lucee's toTime(…, defaultValue) rejects hour > 24, but the observed
        // createTime(25,0,0) is {t '01:00:00'}: the lenient calendar wins.
        lenient_wall(tz, 1899, 12, 30, hour, minute, second, milli)
    } else {
        CfmlDate::from_epoch_millis(0)
    };
    d.with_kind(DateKind::Time)
}

// ─────────────────────────────────────────────
// Names
// ─────────────────────────────────────────────

pub const MONTHS: [&str; 12] = [
    "January", "February", "March", "April", "May", "June", "July", "August", "September", "October",
    "November", "December",
];
pub const MONTHS_SHORT: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
/// Sunday first (`Calendar.DAY_OF_WEEK` − 1).
pub const DAYS: [&str; 7] = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];
pub const DAYS_SHORT: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];

fn month_name(m: u32) -> &'static str {
    MONTHS.get(m as usize - 1).copied().unwrap_or("")
}
fn month_short(m: u32) -> &'static str {
    MONTHS_SHORT.get(m as usize - 1).copied().unwrap_or("")
}
fn day_name(wall: &NaiveDateTime) -> &'static str {
    DAYS[wall.weekday().num_days_from_sunday() as usize]
}
fn day_short(wall: &NaiveDateTime) -> &'static str {
    DAYS_SHORT[wall.weekday().num_days_from_sunday() as usize]
}

/// Zone display name, as `TimeZone.getDisplayName(daylight, style, US)`.
fn zone_name(tz: &Tz, d: &CfmlDate, long: bool) -> String {
    let ms = d.epoch_millis();
    let daylight = datetime::dst_secs_at(tz, ms) != 0;
    if let Some((s_std, s_dst, l_std, l_dst)) = datetime::zone::names_for(tz) {
        return match (long, daylight) {
            (false, false) => s_std,
            (false, true) => s_dst,
            (true, false) => l_std,
            (true, true) => l_dst,
        }
        .to_string();
    }
    // Untabulated zone: the IANA abbreviation, else the GMT offset.
    let utc = d.utc();
    use chrono::TimeZone;
    let off = tz.offset_from_utc_datetime(&utc);
    match off.abbreviation() {
        Some(a) if !a.starts_with('+') && !a.starts_with('-') => a.to_string(),
        _ => {
            let secs = d.offset_secs_in(tz);
            let sign = if secs < 0 { '-' } else { '+' };
            let a = secs.abs();
            format!("GMT{}{:02}:{:02}", sign, a / 3600, (a % 3600) / 60)
        }
    }
}

/// RFC 822 offset, `+0100` (Lucee `DateFormat.Z`).
fn rfc822(tz: &Tz, d: &CfmlDate) -> String {
    let secs = d.offset_secs_in(tz);
    let sign = if secs < 0 { '-' } else { '+' };
    let mins = secs.abs() / 60;
    format!("{}{:02}{:02}", sign, mins / 60, mins % 60)
}

fn is_utc_zone(tz: &Tz) -> bool {
    matches!(tz.name(), "UTC" | "Etc/UTC")
}

/// Lucee `DateFormat.X` (used by dateFormat/timeFormat): `Z` only for the UTC
/// zone itself.
fn iso_offset_lucee(tz: &Tz, d: &CfmlDate, count: usize) -> String {
    if is_utc_zone(tz) {
        return "Z".into();
    }
    let res = rfc822(tz, d);
    match count {
        1 => res[..3].to_string(),
        2 => res,
        _ => format!("{}:{}", &res[..3], &res[3..]),
    }
}

// ─────────────────────────────────────────────
// dateFormat (Lucee runtime.format.DateFormat)
// ─────────────────────────────────────────────

pub fn format_date(d: &CfmlDate, mask: &str, tz: &Tz) -> String {
    let lc = mask.to_lowercase();
    match lc.as_str() {
        "short" => return java_format("M/d/yy", d, tz).unwrap_or_default(),
        "medium" => return java_format("MMM d, y", d, tz).unwrap_or_default(),
        "long" => return java_format("MMMM d, y", d, tz).unwrap_or_default(),
        "full" => return java_format("EEEE, MMMM d, y", d, tz).unwrap_or_default(),
        "iso8601" | "iso" => return java_format("yyyy-MM-dd", d, tz).unwrap_or_default(),
        _ => {}
    }
    let w = d.local_in(tz);
    let m: Vec<char> = mask.chars().collect();
    let len = m.len();
    let at = |i: usize| -> char { m.get(i).copied().unwrap_or('\0') };
    let mut out = String::new();
    let mut pos = 0;
    while pos < len {
        let c = m[pos];
        let next = at(pos + 1);
        match c {
            'z' => {
                let mut count = 1;
                while at(pos + 1) == 'z' {
                    pos += 1;
                    count += 1;
                }
                out.push_str(&zone_name(tz, d, count >= 4));
            }
            'Z' => {
                while at(pos + 1) == 'Z' {
                    pos += 1;
                }
                out.push_str(&rfc822(tz, d));
            }
            'X' => {
                let mut count = 1;
                while at(pos + 1) == 'X' {
                    pos += 1;
                    count += 1;
                }
                out.push_str(&iso_offset_lucee(tz, d, count));
            }
            'g' | 'G' => {
                while at(pos + 1).eq_ignore_ascii_case(&'g') {
                    pos += 1;
                }
                out.push_str(if w.year() > 0 { "AD" } else { "BC" });
            }
            'd' | 'D' => {
                let is_d = |ch: char| ch == 'd' || ch == 'D';
                if is_d(next) {
                    if is_d(at(pos + 2)) {
                        if is_d(at(pos + 3)) {
                            out.push_str(day_name(&w));
                            pos += 3;
                        } else {
                            out.push_str(day_short(&w));
                            pos += 2;
                        }
                    } else {
                        out.push_str(&format!("{:02}", w.day()));
                        pos += 1;
                    }
                } else {
                    out.push_str(&w.day().to_string());
                }
            }
            'm' | 'M' => {
                let is_m = |ch: char| ch == 'm' || ch == 'M';
                if is_m(next) {
                    if is_m(at(pos + 2)) {
                        if is_m(at(pos + 3)) {
                            out.push_str(month_name(w.month()));
                            pos += 3;
                        } else {
                            out.push_str(month_short(w.month()));
                            pos += 2;
                        }
                    } else {
                        out.push_str(&format!("{:02}", w.month()));
                        pos += 1;
                    }
                } else {
                    out.push_str(&w.month().to_string());
                }
            }
            'w' | 'W' => {
                let mut week = 0;
                if c == 'W' || next == 'W' {
                    week = week_of_month(w.date());
                }
                if c == 'w' || next == 'w' {
                    week = calendar_week_of_year(w.date());
                }
                // Lucee's condition, kept as written:
                // next == 'w' || next == 'W' && next_1 == 'w' || next_1 == 'W'
                // where next_1 is the same character as next.
                if next == 'w' || (next == 'W' && next == 'w') || next == 'W' {
                    out.push_str(&format!("{:02}", week));
                    pos += 1;
                } else {
                    out.push_str(&week.to_string());
                }
            }
            'y' | 'Y' => {
                let is_y = |ch: char| ch == 'y' || ch == 'Y';
                let y4 = w.year();
                let y2 = y4 % 100;
                if is_y(next) {
                    if is_y(at(pos + 2)) && is_y(at(pos + 3)) {
                        out.push_str(&y4.to_string());
                        pos += 3;
                    } else if is_y(at(pos + 2)) {
                        out.push_str(&y4.to_string());
                        pos += 2;
                    } else {
                        out.push_str(&format!("{:02}", y2));
                        pos += 1;
                    }
                } else {
                    out.push_str(&y4.to_string());
                }
            }
            other => out.push(other),
        }
        pos += 1;
    }
    out
}

// ─────────────────────────────────────────────
// timeFormat (Lucee runtime.format.TimeFormat)
// ─────────────────────────────────────────────

pub fn format_time(d: &CfmlDate, mask: &str, tz: &Tz) -> String {
    // java.text styles; Lucee replaces the narrow no-break spaces with spaces.
    let lc = mask.to_lowercase();
    match lc.as_str() {
        "short" => return java_format("h:mm a", d, tz).unwrap_or_default(),
        "medium" => return java_format("h:mm:ss a", d, tz).unwrap_or_default(),
        "long" => return java_format("h:mm:ss a z", d, tz).unwrap_or_default(),
        "full" => return java_format("h:mm:ss a zzzz", d, tz).unwrap_or_default(),
        "beat" => {
            // Swatch Internet Time: thousandths of a day in UTC+1.
            let ms = (d.epoch_millis() + 3_600_000).rem_euclid(86_400_000);
            return cfml_common::dynamic::format_double(ms as f64 / 86_400.0);
        }
        _ => {}
    }
    let w = d.local_in(tz);
    let m: Vec<char> = mask.chars().collect();
    let len = m.len();
    let at = |i: usize| -> char { m.get(i).copied().unwrap_or('\0') };
    let mut out = String::new();
    let mut pos = 0;
    while pos < len {
        let c = m[pos];
        let next = at(pos + 1);
        match c {
            'z' => {
                let mut count = 1;
                while at(pos + 1) == 'z' {
                    pos += 1;
                    count += 1;
                }
                out.push_str(&zone_name(tz, d, count >= 4));
            }
            'Z' => {
                while at(pos + 1) == 'Z' {
                    pos += 1;
                }
                out.push_str(&rfc822(tz, d));
            }
            'X' => {
                let mut count = 1;
                while at(pos + 1) == 'X' {
                    pos += 1;
                    count += 1;
                }
                out.push_str(&iso_offset_lucee(tz, d, count));
            }
            'h' => {
                let mut h = w.hour();
                if h == 0 {
                    h = 12;
                }
                if h > 12 {
                    h -= 12;
                }
                if next == 'h' {
                    out.push_str(&format!("{:02}", h));
                    pos += 1;
                } else {
                    out.push_str(&h.to_string());
                }
            }
            'H' => {
                if next == 'H' {
                    out.push_str(&format!("{:02}", w.hour()));
                    pos += 1;
                } else {
                    out.push_str(&w.hour().to_string());
                }
            }
            'N' | 'n' | 'M' | 'm' => {
                if matches!(next, 'M' | 'm' | 'N' | 'n') {
                    out.push_str(&format!("{:02}", w.minute()));
                    pos += 1;
                } else {
                    out.push_str(&w.minute().to_string());
                }
            }
            's' | 'S' => {
                if next == 'S' || next == 's' {
                    out.push_str(&format!("{:02}", w.second()));
                    pos += 1;
                } else {
                    out.push_str(&w.second().to_string());
                }
            }
            'l' | 'L' => {
                let nextnext = at(pos + 2);
                let mut millis = (d.subsec_nanos() / 1_000_000).to_string();
                if next == 'L' || next == 'l' {
                    if millis.len() == 1 {
                        millis.insert(0, '0');
                    }
                    pos += 1;
                }
                if nextnext == 'L' || nextnext == 'l' {
                    if millis.len() == 2 {
                        millis.insert(0, '0');
                    }
                    pos += 1;
                }
                out.push_str(&millis);
            }
            't' | 'T' => {
                let am = w.hour() < 12;
                if next == 'T' || next == 't' {
                    out.push_str(if am { "AM" } else { "PM" });
                    pos += 1;
                } else {
                    out.push_str(if am { "A" } else { "P" });
                }
            }
            other => out.push(other),
        }
        pos += 1;
    }
    out
}

// ─────────────────────────────────────────────
// dateTimeFormat (Lucee DateTimeFormat + java.time)
// ─────────────────────────────────────────────

pub const DATETIME_DEFAULT_MASK: &str = "dd-MMM-yyyy HH:mm:ss";

pub fn format_datetime(d: &CfmlDate, mask: Option<&str>, tz: &Tz) -> Result<String, CfmlError> {
    if let Some(m) = mask {
        let lc = m.to_lowercase();
        match lc.as_str() {
            "epoch" => return Ok((d.epoch_millis() / 1000).to_string()),
            "epochms" => return Ok(d.epoch_millis().to_string()),
            // java.time localized styles (JDK 21 CLDR): a comma between date and
            // time, and a NARROW NO-BREAK SPACE before the AM/PM marker.
            "short" => return java_format("M/d/yy, h:mm\u{202F}a", d, tz),
            "medium" => return java_format("MMM d, y, h:mm:ss\u{202F}a", d, tz),
            "long" => return java_format("MMMM d, y, h:mm:ss\u{202F}a z", d, tz),
            "full" => return java_format("EEEE, MMMM d, y, h:mm:ss\u{202F}a zzzz", d, tz),
            "iso" | "iso8601" => return java_format("yyyy-MM-dd'T'HH:mm:ssXXX", d, tz),
            "isoms" | "isomillis" | "javascript" => {
                return java_format("yyyy-MM-dd'T'HH:mm:ss.SSSXXX", d, tz)
            }
            _ => {}
        }
    }
    let pattern = convert_mask(mask);
    let mut result = java_format(&pattern, d, tz)?;
    // A single `t` became `>>>a<<<`: keep the first letter of the marker.
    while let Some(start) = result.find(">>>") {
        let Some(rel) = result[start + 3..].find("<<<") else { break };
        let end = start + 3 + rel;
        let mut content = result[start + 3..end].to_string();
        if content.chars().count() == 2 {
            content = content.chars().take(1).collect();
        }
        result = format!("{}{}{}", &result[..start], content, &result[end + 3..]);
    }
    Ok(result)
}

/// Lucee `DateTimeFormat.convertMask`: a CFML mask to a java.time pattern.
fn convert_mask(mask: Option<&str>) -> String {
    const ZERO: char = '\u{0}';
    const ONE: char = '\u{1}';
    let Some(mask) = mask else { return DATETIME_DEFAULT_MASK.to_string() };
    let lc = mask.to_lowercase();
    if lc == "iso8601" || lc == "iso" {
        return "yyyy-MM-dd'T'HH:mm:ssXXX".into();
    }
    if lc == "isoms" || lc == "isomillis" || lc == "javascript" {
        return "yyyy-MM-dd'T'HH:mm:ss.SSSXXX".into();
    }
    let zz: String = [ZERO, ZERO].iter().collect();
    let mask = mask.replace("''", &zz);
    let carr: Vec<char> = mask.chars().collect();
    let mut sb: Vec<char> = Vec::with_capacity(carr.len() + 8);
    let mut inside = false;

    fn has_already(sb: &[char], c: char, count: usize) -> bool {
        let l = sb.len();
        if l < count {
            return false;
        }
        sb[l - count..].iter().all(|&x| x == c)
    }
    fn push_capped(sb: &mut Vec<char>, inside: bool, c: char, emit: char, cap: usize) {
        if inside {
            sb.push(c);
        } else if !has_already(sb, emit, cap) {
            sb.push(emit);
        }
    }
    fn push_str(sb: &mut Vec<char>, s: &str) {
        sb.extend(s.chars());
    }

    let mut i = 0;
    while i < carr.len() {
        let c = carr[i];
        match c {
            'W' | 'a' | 'F' => push_capped(&mut sb, inside, c, c, 1),
            's' | 'H' | 'K' | 'k' | 'h' | 'w' => push_capped(&mut sb, inside, c, c, 2),
            'x' | 'Z' => push_capped(&mut sb, inside, c, c, 3),
            'G' | 'E' | 'M' | 'z' => push_capped(&mut sb, inside, c, c, 4),
            'X' => push_capped(&mut sb, inside, c, c, 5),
            'y' => push_capped(&mut sb, inside, c, c, 10),
            'm' => push_capped(&mut sb, inside, c, 'M', 4),
            'D' | 'd' => {
                if inside {
                    sb.push(c);
                } else if has_already(&sb, 'E', 4) {
                    // already the full day name
                } else if has_already(&sb, 'E', 3) {
                    sb.push('E');
                } else if has_already(&sb, c, 2) {
                    // Lucee writes over the two previous letters by MASK index.
                    if i >= 2 && i - 1 < sb.len() {
                        sb[i - 2] = 'E';
                        sb[i - 1] = 'E';
                    }
                    sb.push('E');
                } else {
                    sb.push(c);
                }
            }
            'S' => push_capped(&mut sb, inside, c, 's', 2),
            't' | 'T' => {
                if inside {
                    sb.push(c);
                } else if i + 1 < carr.len() && (carr[i + 1] == 't' || carr[i + 1] == 'T') {
                    if !has_already(&sb, 'a', 1) {
                        sb.push('a');
                    }
                    i += 1;
                } else if !has_already(&sb, 'a', 1) {
                    push_str(&mut sb, ">>>a<<<");
                }
            }
            'n' | 'N' => push_capped(&mut sb, inside, c, 'm', 2),
            'l' | 'L' => push_capped(&mut sb, inside, c, 'S', 9),
            'Y' => push_capped(&mut sb, inside, c, 'y', 10),
            'g' => push_capped(&mut sb, inside, c, 'G', 4),
            'f' | 'e' | 'A' => {
                if inside {
                    sb.push(c);
                } else {
                    sb.push('\'');
                    sb.push(c);
                    sb.push('\'');
                }
            }
            '\'' => {
                if i + 1 < carr.len() && carr[i + 1] == '\'' {
                    i += 1;
                    push_str(&mut sb, "''");
                } else {
                    inside = !inside;
                    sb.push('\'');
                }
            }
            _ => {
                if !inside && c.is_ascii_alphabetic() {
                    sb.push('\'');
                    sb.push(c);
                    sb.push('\'');
                } else {
                    sb.push(c);
                }
            }
        }
        i += 1;
    }
    let s: String = sb.into_iter().collect();
    let s = s.replace("''", "").replace(&zz, "''").replace(ONE, "E");
    y_to_yyyy(&s)
}

/// Lucee `DateTimeFormat.y2yyyy`: a lone `y` means the full year.
fn y_to_yyyy(s: &str) -> String {
    let c: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len() + 4);
    let mut inside = false;
    for i in 0..c.len() {
        let ch = c[i];
        if ch == '\'' {
            inside = !inside;
        } else if !inside && ch == 'y' {
            let prev_y = i > 0 && c[i - 1] == 'y';
            let next_y = i + 1 < c.len() && c[i + 1] == 'y';
            if !prev_y && !next_y {
                out.push_str("yyyy");
                continue;
            }
        }
        out.push(ch);
    }
    out
}

/// Format with a `java.time.format.DateTimeFormatter` pattern, `Locale.US`.
/// Covers every letter Lucee's mask conversion can produce, plus the ones the
/// named styles use.
pub fn java_format(pattern: &str, d: &CfmlDate, tz: &Tz) -> Result<String, CfmlError> {
    let w = d.local_in(tz);
    let p: Vec<char> = pattern.chars().collect();
    let mut out = String::with_capacity(pattern.len() + 8);
    let mut i = 0;
    while i < p.len() {
        let c = p[i];
        if c == '\'' {
            // Quoted literal; '' is an apostrophe.
            if i + 1 < p.len() && p[i + 1] == '\'' {
                out.push('\'');
                i += 2;
                continue;
            }
            i += 1;
            while i < p.len() {
                if p[i] == '\'' {
                    if i + 1 < p.len() && p[i + 1] == '\'' {
                        out.push('\'');
                        i += 2;
                        continue;
                    }
                    break;
                }
                out.push(p[i]);
                i += 1;
            }
            i += 1;
            continue;
        }
        if matches!(c, '{' | '}' | '#') {
            return Err(CfmlError::expression(format!("Pattern includes reserved character: '{}'", c)));
        }
        if c == '[' || c == ']' {
            i += 1;
            continue;
        }
        if !c.is_ascii_alphabetic() {
            out.push(c);
            i += 1;
            continue;
        }
        let mut n = 1;
        while i + n < p.len() && p[i + n] == c {
            n += 1;
        }
        i += n;
        let too_many = || CfmlError::expression(format!("Too many pattern letters: {}", c));
        let num = |v: i64, min: usize| -> String {
            if v < 0 {
                format!("-{:0width$}", -v, width = min)
            } else {
                format!("{:0width$}", v, width = min)
            }
        };
        match c {
            'G' => out.push_str(match n {
                1..=3 => if w.year() > 0 { "AD" } else { "BC" },
                4 => if w.year() > 0 { "Anno Domini" } else { "Before Christ" },
                _ => if w.year() > 0 { "A" } else { "B" },
            }),
            'y' | 'u' => {
                let y = if c == 'y' && w.year() <= 0 { 1 - w.year() as i64 } else { w.year() as i64 };
                if n == 2 {
                    out.push_str(&format!("{:02}", y.rem_euclid(100)));
                } else {
                    out.push_str(&num(y, n));
                }
            }
            'M' | 'L' => match n {
                1 => out.push_str(&w.month().to_string()),
                2 => out.push_str(&format!("{:02}", w.month())),
                3 => out.push_str(month_short(w.month())),
                4 => out.push_str(month_name(w.month())),
                5 => out.push_str(&month_name(w.month())[..1]),
                _ => return Err(too_many()),
            },
            'd' => match n {
                1 => out.push_str(&w.day().to_string()),
                2 => out.push_str(&format!("{:02}", w.day())),
                _ => return Err(too_many()),
            },
            'D' => match n {
                1..=3 => out.push_str(&num(w.ordinal() as i64, n)),
                _ => return Err(too_many()),
            },
            'E' => match n {
                1..=3 => out.push_str(day_short(&w)),
                4 => out.push_str(day_name(&w)),
                5 => out.push_str(&day_name(&w)[..1]),
                _ => return Err(too_many()),
            },
            'a' => {
                if n > 1 {
                    return Err(too_many());
                }
                out.push_str(if w.hour() < 12 { "AM" } else { "PM" })
            }
            'h' | 'K' | 'k' | 'H' | 'm' | 's' | 'w' | 'W' | 'F' => {
                let v: i64 = match c {
                    'h' => {
                        let h = w.hour() % 12;
                        if h == 0 { 12 } else { h as i64 }
                    }
                    'K' => (w.hour() % 12) as i64,
                    'k' => {
                        if w.hour() == 0 { 24 } else { w.hour() as i64 }
                    }
                    'H' => w.hour() as i64,
                    'm' => w.minute() as i64,
                    's' => w.second() as i64,
                    'w' => calendar_week_of_year(w.date()),
                    'W' => week_of_month(w.date()),
                    _ => (w.day() as i64 - 1) / 7 + 1, // F: aligned week of month
                };
                let max = if matches!(c, 'W' | 'F') { 1 } else { 2 };
                if n > max {
                    return Err(too_many());
                }
                out.push_str(&num(v, n));
            }
            'S' => {
                if n > 9 {
                    return Err(too_many());
                }
                let digits = format!("{:09}", d.subsec_nanos());
                out.push_str(&digits[..n]);
            }
            'z' => match n {
                1..=3 => out.push_str(&zone_name(tz, d, false)),
                4 => out.push_str(&zone_name(tz, d, true)),
                _ => return Err(too_many()),
            },
            'Z' => {
                let secs = d.offset_secs_in(tz);
                match n {
                    1..=3 => out.push_str(&rfc822(tz, d)),
                    4 => {
                        if secs == 0 {
                            out.push_str("GMT");
                        } else {
                            let a = secs.abs();
                            out.push_str(&format!(
                                "GMT{}{:02}:{:02}",
                                if secs < 0 { '-' } else { '+' },
                                a / 3600,
                                (a % 3600) / 60
                            ));
                        }
                    }
                    5 => out.push_str(&iso_offset_java(d.offset_secs_in(tz), 3, true)),
                    _ => return Err(too_many()),
                }
            }
            'X' | 'x' => {
                if n > 5 {
                    return Err(too_many());
                }
                out.push_str(&iso_offset_java(d.offset_secs_in(tz), n, c == 'X'));
            }
            other => {
                return Err(CfmlError::expression(format!("Unknown pattern letter: {}", other)));
            }
        }
    }
    Ok(out)
}

/// java.time `X`/`x` offsets. `X` prints `Z` for a zero offset.
fn iso_offset_java(secs: i32, n: usize, z_for_zero: bool) -> String {
    if secs == 0 && z_for_zero {
        return "Z".into();
    }
    let sign = if secs < 0 { '-' } else { '+' };
    let a = secs.abs();
    let (h, m, s) = (a / 3600, (a % 3600) / 60, a % 60);
    match n {
        1 => {
            if m == 0 {
                format!("{}{:02}", sign, h)
            } else {
                format!("{}{:02}{:02}", sign, h, m)
            }
        }
        2 => format!("{}{:02}{:02}", sign, h, m),
        3 => format!("{}{:02}:{:02}", sign, h, m),
        4 => {
            if s == 0 {
                format!("{}{:02}{:02}", sign, h, m)
            } else {
                format!("{}{:02}{:02}{:02}", sign, h, m, s)
            }
        }
        _ => {
            if s == 0 {
                format!("{}{:02}:{:02}", sign, h, m)
            } else {
                format!("{}{:02}:{:02}:{:02}", sign, h, m, s)
            }
        }
    }
}

// ─────────────────────────────────────────────
// The functions
// ─────────────────────────────────────────────

pub fn fn_now(_args: Vec<CfmlValue>) -> CfmlResult {
    Ok(date_value(CfmlDate::now()))
}

pub fn fn_create_date_time(args: Vec<CfmlValue>) -> CfmlResult {
    let n = |i: usize, default: i64| -> Result<i64, CfmlError> {
        match args.get(i) {
            None | Some(CfmlValue::Null) => Ok(default),
            Some(_) => arg_long(&args, i),
        }
    };
    let tz = arg_tz(&args, 7)?;
    Ok(date_value(make_date(&tz, n(0, 0)?, n(1, 1)?, n(2, 1)?, n(3, 0)?, n(4, 0)?, n(5, 0)?, n(6, 0)?)?))
}

pub fn fn_create_date(args: Vec<CfmlValue>) -> CfmlResult {
    let n = |i: usize| -> Result<i64, CfmlError> {
        match args.get(i) {
            None | Some(CfmlValue::Null) => Ok(1),
            Some(_) => arg_long(&args, i),
        }
    };
    let tz = arg_tz(&args, 3)?;
    Ok(date_value(make_date(&tz, arg_long(&args, 0)?, n(1)?, n(2)?, 0, 0, 0, 0)?))
}

pub fn fn_create_time(args: Vec<CfmlValue>) -> CfmlResult {
    let n = |i: usize| -> Result<i64, CfmlError> {
        match args.get(i) {
            None | Some(CfmlValue::Null) => Ok(0),
            Some(_) => arg_long(&args, i),
        }
    };
    let tz = arg_tz(&args, 4)?;
    Ok(date_value(make_time(&tz, n(0)?, n(1)?, n(2)?, n(3)?)))
}

pub fn fn_create_odbc_date(args: Vec<CfmlValue>) -> CfmlResult {
    let d = arg_date(&args, 0)?;
    let tz = arg_tz(&args, 1)?;
    let w = d.local_in(&tz);
    let midnight = w.date().and_hms_opt(0, 0, 0).unwrap_or(w);
    Ok(date_value(from_wall(&tz, &midnight).with_kind(DateKind::Date)))
}

pub fn fn_create_odbc_date_time(args: Vec<CfmlValue>) -> CfmlResult {
    Ok(date_value(arg_date(&args, 0)?.with_kind(DateKind::DateTime)))
}

pub fn fn_create_odbc_time(args: Vec<CfmlValue>) -> CfmlResult {
    Ok(date_value(arg_date(&args, 0)?.with_kind(DateKind::Time)))
}

pub fn fn_date_add(args: Vec<CfmlValue>) -> CfmlResult {
    let part = args.first().map(|v| v.as_string()).unwrap_or_default();
    let number = arg_long(&args, 1)?;
    let d = arg_date(&args, 2)?;
    let tz = datetime::current_zone();
    Ok(date_value(date_add(&part, number, d, &tz)?))
}

pub fn fn_date_diff(args: Vec<CfmlValue>) -> CfmlResult {
    let part = args.first().map(|v| v.as_string()).unwrap_or_default();
    let left = arg_date(&args, 1)?;
    let right = arg_date(&args, 2)?;
    let tz = datetime::current_zone();
    Ok(CfmlValue::Int(date_diff(&part, left, right, &tz)?))
}

pub fn fn_date_compare(args: Vec<CfmlValue>) -> CfmlResult {
    let left = arg_date(&args, 0)?;
    let right = arg_date(&args, 1)?;
    let part = match args.get(2) {
        Some(v) if !matches!(v, CfmlValue::Null) => v.as_string(),
        _ => "s".into(),
    };
    let tz = datetime::current_zone();
    Ok(CfmlValue::Int(date_compare(left, right, &part, &tz)?))
}

pub fn fn_date_part(args: Vec<CfmlValue>) -> CfmlResult {
    let part = args.first().map(|v| v.as_string()).unwrap_or_default();
    let d = arg_date(&args, 1)?;
    let tz = arg_tz(&args, 2)?;
    Ok(CfmlValue::Int(date_part(&part, d, &tz)?))
}

/// The common shape of `year(date [, tz])` and friends.
fn field(args: &[CfmlValue], f: impl Fn(&CfmlDate, &NaiveDateTime) -> i64) -> CfmlResult {
    let d = arg_date(args, 0)?;
    let tz = arg_tz(args, 1)?;
    let w = d.local_in(&tz);
    Ok(CfmlValue::Int(f(&d, &w)))
}

pub fn fn_year(args: Vec<CfmlValue>) -> CfmlResult {
    field(&args, |_, w| w.year() as i64)
}
pub fn fn_month(args: Vec<CfmlValue>) -> CfmlResult {
    field(&args, |_, w| w.month() as i64)
}
pub fn fn_day(args: Vec<CfmlValue>) -> CfmlResult {
    field(&args, |_, w| w.day() as i64)
}
pub fn fn_hour(args: Vec<CfmlValue>) -> CfmlResult {
    field(&args, |_, w| w.hour() as i64)
}
pub fn fn_minute(args: Vec<CfmlValue>) -> CfmlResult {
    field(&args, |_, w| w.minute() as i64)
}
pub fn fn_second(args: Vec<CfmlValue>) -> CfmlResult {
    field(&args, |_, w| w.second() as i64)
}
pub fn fn_millisecond(args: Vec<CfmlValue>) -> CfmlResult {
    field(&args, |d, _| (d.subsec_nanos() / 1_000_000) as i64)
}
pub fn fn_day_of_week(args: Vec<CfmlValue>) -> CfmlResult {
    field(&args, |_, w| dow(w))
}
pub fn fn_day_of_year(args: Vec<CfmlValue>) -> CfmlResult {
    field(&args, |_, w| w.ordinal() as i64)
}
pub fn fn_days_in_month(args: Vec<CfmlValue>) -> CfmlResult {
    field(&args, |_, w| datetime::days_in_month(w.year(), w.month()) as i64)
}
pub fn fn_days_in_year(args: Vec<CfmlValue>) -> CfmlResult {
    field(&args, |_, w| if datetime::is_leap_year(w.year()) { 366 } else { 365 })
}
/// Day of the year of the first day of the date's month (Lucee).
pub fn fn_first_day_of_month(args: Vec<CfmlValue>) -> CfmlResult {
    field(&args, |_, w| w.date().with_day(1).map(|f| f.ordinal() as i64).unwrap_or(1))
}
pub fn fn_quarter(args: Vec<CfmlValue>) -> CfmlResult {
    field(&args, |_, w| (w.month() as i64 - 1) / 3 + 1)
}
pub fn fn_week(args: Vec<CfmlValue>) -> CfmlResult {
    field(&args, |_, w| continuing_week_of_year(w.date()))
}

pub fn fn_get_numeric_date(args: Vec<CfmlValue>) -> CfmlResult {
    Ok(CfmlValue::Double(arg_date(&args, 0)?.to_numeric()))
}

/// Lucee `Decision.isDateAdvanced(value, alsoNumbers=true)`.
pub fn fn_is_numeric_date(args: Vec<CfmlValue>) -> CfmlResult {
    let v = args.first().map(|v| v.query_column_scalar()).unwrap_or(&CfmlValue::Null);
    Ok(CfmlValue::Bool(match v {
        CfmlValue::DateTime(_) | CfmlValue::Int(_) | CfmlValue::Double(_) | CfmlValue::TimeSpan(_) => true,
        CfmlValue::String(s) => {
            let t = s.trim();
            !t.is_empty() && (cfml_common::numeric::numeric_string_value(t).is_some() || parse::parse(t).is_some())
        }
        _ => false,
    }))
}

pub fn fn_date_convert(args: Vec<CfmlValue>) -> CfmlResult {
    let kind = args.first().map(|v| v.as_string()).unwrap_or_default();
    let d = arg_date(&args, 1)?;
    let tz = datetime::current_zone();
    let offset_ms = d.offset_secs_in(&tz) as i64 * 1000;
    match kind.to_lowercase().as_str() {
        "local2utc" => Ok(date_value(d.plus_millis(-offset_ms).with_kind(DateKind::DateTime))),
        "utc2local" => Ok(date_value(d.plus_millis(offset_ms).with_kind(DateKind::DateTime))),
        _ => Err(arg_error(
            "DateConvert",
            1,
            "conversionType",
            &format!("invalid conversion-type [{}] for function dateConvert", kind),
        )),
    }
}

pub fn fn_get_http_time_string(args: Vec<CfmlValue>) -> CfmlResult {
    let d = match args.first() {
        None | Some(CfmlValue::Null) => CfmlDate::now(),
        Some(_) => arg_date(&args, 0)?,
    };
    Ok(CfmlValue::string(datetime::http_string(&d, false)))
}

pub fn fn_parse_date_time(args: Vec<CfmlValue>) -> CfmlResult {
    let v = args.first().unwrap_or(&CfmlValue::Null);
    match v {
        CfmlValue::DateTime(d) => Ok(date_value(*d)),
        CfmlValue::String(s) => {
            let parsed = parse::parse(s.trim()).ok_or_else(|| {
                CfmlError::expression(format!("can't cast [{}] to date value", s))
            })?;
            let d = parsed.to_date();
            // parseDateTime returns a date-time, except that a `{t '…'}`
            // literal stays a time (Lucee 7.1).
            Ok(date_value(if matches!(parsed, parse::Parsed::Time(_)) {
                d
            } else {
                d.with_kind(DateKind::DateTime)
            }))
        }
        CfmlValue::QueryColumn(..) => fn_parse_date_time(vec![v.query_column_scalar().clone()]),
        _ => Err(CfmlError::expression("can't cast value to a Date Object".into())),
    }
}

/// dateFormat / lsDateFormat. A blank value gives "" (Lucee).
pub fn fn_date_format(args: Vec<CfmlValue>) -> CfmlResult {
    let v = args.first().unwrap_or(&CfmlValue::Null);
    if is_blank(v) {
        return Ok(CfmlValue::string(String::new()));
    }
    let d = to_date(v).ok_or_else(|| {
        CfmlError::expression(format!("Can't cast String [{}] to a value of type [datetime]", v.as_string()))
    })?;
    let mask = mask_arg(&args, 1).unwrap_or_else(|| "dd-mmm-yy".into());
    let tz = arg_tz(&args, 2)?;
    Ok(CfmlValue::string(format_date(&d, &mask, &tz)))
}

pub fn fn_time_format(args: Vec<CfmlValue>) -> CfmlResult {
    let v = args.first().unwrap_or(&CfmlValue::Null);
    if is_blank(v) {
        return Ok(CfmlValue::string(String::new()));
    }
    let d = to_date(v).ok_or_else(|| {
        CfmlError::expression(format!("Can't convert value [{}] to a datetime value", v.as_string()))
    })?;
    let mask = mask_arg(&args, 1).unwrap_or_else(|| "hh:mm tt".into());
    let tz = arg_tz(&args, 2)?;
    Ok(CfmlValue::string(format_time(&d, &mask, &tz)))
}

pub fn fn_date_time_format(args: Vec<CfmlValue>) -> CfmlResult {
    let v = args.first().unwrap_or(&CfmlValue::Null);
    if is_blank(v) {
        return Ok(CfmlValue::string(String::new()));
    }
    let d = to_date(v).ok_or_else(|| {
        CfmlError::expression(format!("Can't convert value [{}] to a datetime value", v.as_string()))
    })?;
    let mask = mask_arg(&args, 1);
    let tz = arg_tz(&args, 2)?;
    Ok(CfmlValue::string(format_datetime(&d, mask.as_deref(), &tz)?))
}

fn is_blank(v: &CfmlValue) -> bool {
    match v.query_column_scalar() {
        CfmlValue::Null => true,
        CfmlValue::String(s) => s.trim().is_empty(),
        _ => false,
    }
}

fn mask_arg(args: &[CfmlValue], i: usize) -> Option<String> {
    match args.get(i) {
        None | Some(CfmlValue::Null) => None,
        Some(v) => Some(v.as_string()),
    }
}

pub fn fn_get_tick_count(args: Vec<CfmlValue>) -> CfmlResult {
    let unit = match args.first() {
        None | Some(CfmlValue::Null) => return Ok(CfmlValue::Int(cfml_common::clock::now_unix_millis() as i64)),
        Some(CfmlValue::Int(_)) | Some(CfmlValue::Double(_)) => {
            // Numeric units: 1 nano, 2 milli, 4 micro, else seconds.
            let u = args[0].as_string().parse::<f64>().unwrap_or(8.0);
            return Ok(CfmlValue::Int(match u as i64 {
                1 => cfml_common::clock::now_unix_nanos() as i64,
                4 => (cfml_common::clock::now_unix_nanos() / 1000) as i64,
                2 => cfml_common::clock::now_unix_millis() as i64,
                _ => cfml_common::clock::now_unix_secs() as i64,
            }));
        }
        Some(v) => v.as_string(),
    };
    let u = unit.trim();
    let first = u.chars().next().map(|c| c.to_ascii_lowercase());
    let val = match first {
        Some('n') => cfml_common::clock::now_unix_nanos() as i64,
        Some('m') if u.eq_ignore_ascii_case("micro") => (cfml_common::clock::now_unix_nanos() / 1000) as i64,
        Some('m') => cfml_common::clock::now_unix_millis() as i64,
        Some('s') => cfml_common::clock::now_unix_secs() as i64,
        _ => {
            return Err(arg_error(
                "GetTickCount",
                1,
                "unit",
                &format!("invalid value [{}], valid values are (nano, micro, milli, second)", u),
            ))
        }
    };
    Ok(CfmlValue::Int(val))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn london() -> Tz {
        datetime::zone::resolve_tz("Europe/London").unwrap()
    }

    fn d(y: i64, m: i64, day: i64, h: i64, mi: i64, s: i64, ms: i64) -> CfmlDate {
        make_date(&london(), y, m, day, h, mi, s, ms).unwrap()
    }

    fn fmt(d: &CfmlDate) -> String {
        format_datetime(d, Some("yyyy-mm-dd HH:nn:ss.lll"), &london()).unwrap()
    }

    #[test]
    fn date_add_matches_lucee() {
        let tz = london();
        let dms = d(2026, 1, 2, 10, 20, 30, 250);
        let cases = [
            ("yyyy", "2029-01-02 10:20:30.250"),
            ("q", "2026-10-02 10:20:30.250"),
            ("m", "2026-04-02 10:20:30.250"),
            ("y", "2026-01-05 10:20:30.250"),
            ("d", "2026-01-05 10:20:30.250"),
            ("w", "2026-01-07 10:20:30.250"),
            ("ww", "2026-01-23 10:20:30.250"),
            ("h", "2026-01-02 13:20:30.250"),
            ("n", "2026-01-02 10:23:30.250"),
            ("s", "2026-01-02 10:20:33.250"),
            ("l", "2026-01-02 10:20:30.253"),
        ];
        for (part, want) in cases {
            assert_eq!(fmt(&date_add(part, 3, dms, &tz).unwrap()), want, "{}", part);
        }
        // DST: Calendar.add(DATE) across the spring change lands on 00:30.
        let before = d(2026, 3, 28, 1, 30, 0, 0);
        assert_eq!(fmt(&date_add("d", 1, before, &tz).unwrap()), "2026-03-29 00:30:00.000");
        let mid = d(2026, 3, 29, 0, 30, 0, 0);
        assert_eq!(fmt(&date_add("h", 1, mid, &tz).unwrap()), "2026-03-29 02:30:00.000");
    }

    #[test]
    fn weekdays_match_lucee() {
        let tz = london();
        let f = |y, m, day, n| {
            let r = date_add("w", n, d(y, m, day, 0, 0, 0, 0), &tz).unwrap();
            format_date(&r, "yyyy-mm-dd ddd", &tz)
        };
        assert_eq!(f(2026, 1, 3, 1), "2026-01-05 Mon");
        assert_eq!(f(2026, 1, 7, -3), "2026-01-02 Fri");
        assert_eq!(f(2026, 1, 2, 5), "2026-01-09 Fri");
        assert_eq!(f(2026, 1, 4, 1), "2026-01-05 Mon");
        assert_eq!(f(2026, 1, 3, 0), "2026-01-05 Mon");
    }

    #[test]
    fn date_diff_matches_lucee() {
        let tz = london();
        let dms = d(2026, 1, 2, 10, 20, 30, 250);
        let d2 = d(2026, 3, 29, 23, 5, 9, 0);
        let want = [
            ("yyyy", 0),
            ("q", 0),
            ("m", 2),
            ("y", 86),
            ("d", 86),
            ("w", 12),
            ("ww", 12),
            ("h", 2075),
            ("n", 124544),
            ("s", 7472678),
            ("l", 7472678750),
        ];
        for (part, v) in want {
            assert_eq!(date_diff(part, dms, d2, &tz).unwrap(), v, "{}", part);
        }
        let m = |a: CfmlDate, b: CfmlDate| date_diff("m", a, b, &tz).unwrap();
        assert_eq!(m(d(2026, 1, 31, 0, 0, 0, 0), d(2026, 2, 28, 0, 0, 0, 0)), 1);
        assert_eq!(m(d(2026, 1, 15, 12, 0, 0, 0), d(2026, 2, 15, 11, 0, 0, 0)), 0);
        assert_eq!(m(d(2026, 3, 15, 0, 0, 0, 0), d(2026, 1, 20, 0, 0, 0, 0)), -1);
        assert_eq!(date_diff("q", d(2026, 1, 15, 0, 0, 0, 0), d(2026, 4, 14, 0, 0, 0, 0), &tz).unwrap(), 0);
        assert_eq!(date_diff("yyyy", d(2027, 5, 31, 0, 0, 0, 0), d(2026, 6, 1, 0, 0, 0, 0), &tz).unwrap(), 0);
        assert_eq!(date_diff("h", d(2026, 3, 28, 12, 0, 0, 0), d(2026, 3, 29, 12, 0, 0, 0), &tz).unwrap(), 23);
        assert_eq!(date_diff("d", d(2026, 3, 28, 12, 0, 0, 0), d(2026, 3, 29, 12, 0, 0, 0), &tz).unwrap(), 1);
    }

    #[test]
    fn masks_match_lucee() {
        let tz = london();
        let dms = d(2026, 1, 2, 10, 20, 30, 250);
        let dt = |m: &str| format_datetime(&dms, Some(m), &tz).unwrap();
        assert_eq!(dt("yyyy-MM-dd'T'HH:nn:ss.lllZ"), "2026-01-02T10:20:30.250+0000");
        assert_eq!(dt("yyyy-mm-dd'T'HH:nn:ssXXX"), "2026-01-02T10:20:30Z");
        assert_eq!(dt("SSS"), "30");
        assert_eq!(dt("gg"), "AD");
        assert_eq!(dt("EEEE"), "Friday");
        assert_eq!(dt("ww"), "01");
        assert_eq!(dt("yyyy-mm-dd at HH:nn"), "2026-01-02 AM 10:20");
        assert_eq!(dt("h:nn t"), "10:20 A");
        assert_eq!(dt("'yyyy' yyyy"), "yyyy 2026");
        assert_eq!(dt("b c f e A B C I J O P Q R T U V"), "b c f e A B C I J O P Q R A U V");
        assert_eq!(dt("short"), "1/2/26, 10:20\u{202F}AM");
        assert_eq!(dt("full"), "Friday, January 2, 2026, 10:20:30\u{202F}AM Greenwich Mean Time");
        assert_eq!(dt("iso"), "2026-01-02T10:20:30Z");
        assert_eq!(dt("epochms"), "1767349230250");
        let d5 = d(2026, 1, 2, 10, 20, 30, 5);
        let t = |m: &str| format_time(&d5, m, &tz);
        assert_eq!([t("l"), t("ll"), t("lll"), t("llll"), t("lllll")], ["5", "05", "005", "0055", "00505"]);
        let x = |m: &str| format_datetime(&d5, Some(m), &tz).unwrap();
        assert_eq!([x("l"), x("lll"), x("llll"), x("lllll")], ["0", "005", "0050", "00500"]);
        assert_eq!(format_date(&dms, "'yyyy'", &tz), "'2026'");
        assert_eq!(
            format_date(&dms, "gg G W WW w ww y yyy Y d dd ddd dddd m mm mmm mmmm", &tz),
            "AD AD 1 01 1 01 2026 2026 2026 2 02 Fri Friday 1 01 Jan January"
        );
    }
}

// ─────────────────────────────────────────────
// Database values
// ─────────────────────────────────────────────
//
// JDBC hands Lucee a `java.sql.Timestamp` for a zone-less column, read as a
// wall clock in the JVM zone, and Lucee keeps its milliseconds (truncated). A
// `TIME` column is a `java.sql.Time`, a time on 1970-01-01. These helpers give
// every driver the same rules.

/// A zone-less database date/time (`DATETIME`, `timestamp`, `DATE`) as a date:
/// the wall clock in the request zone, truncated to the request's precision.
pub fn from_db_wall(wall: &NaiveDateTime) -> CfmlDate {
    let tz = datetime::current_zone();
    CfmlDate::from_epoch(datetime::local_to_epoch_secs(&tz, wall), wall.nanosecond() as i64)
        .truncated_to(datetime::precision())
}

/// A database instant (`timestamptz`, `DATETIMEOFFSET`) as a date.
pub fn from_db_utc(utc: &NaiveDateTime) -> CfmlDate {
    let u = utc.and_utc();
    CfmlDate::from_epoch(u.timestamp(), u.timestamp_subsec_nanos() as i64).truncated_to(datetime::precision())
}

/// A database `TIME` as a date on 1970-01-01 (`java.sql.Time`). `nanos_of_day`
/// may be negative or exceed a day (MySQL `TIME` spans ±838 hours).
pub fn from_db_time(nanos_of_day: i128) -> CfmlDate {
    let base = NaiveDate::from_ymd_opt(1970, 1, 1).and_then(|d| d.and_hms_opt(0, 0, 0)).unwrap_or_default();
    let secs = nanos_of_day.div_euclid(1_000_000_000) as i64;
    let nanos = nanos_of_day.rem_euclid(1_000_000_000) as u32;
    let wall = base
        .checked_add_signed(Duration::seconds(secs))
        .and_then(|w| w.with_nanosecond(nanos))
        .unwrap_or(base);
    from_db_wall(&wall)
}

/// The wall clock a date binds as, in the request zone.
pub fn db_wall(d: &CfmlDate) -> NaiveDateTime {
    d.local()
}
