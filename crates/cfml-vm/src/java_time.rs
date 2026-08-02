//! `java.time.*` shim — enough of the JSR-310 surface for ColdBox's scheduler
//! date library (`coldbox/system/async/time/*.cfc`) to construct and compute
//! schedules without a JVM. RustCFML's async kernel spawns detached threads and
//! does not consume these schedule instants with millisecond fidelity, so the
//! shim aims to be *non-crashing and reasonable* rather than bit-exact: real
//! arithmetic where it's cheap (chrono-backed), benign identity where it's exotic
//! (`with(adjuster)`, custom `format(pattern)`).
//!
//! Value model: an instant-bearing object (LocalDateTime / Instant /
//! ZonedDateTime) carries `__dt_millis` (epoch millis, UTC). A Duration carries
//! `__dur_millis`. A ZoneId/ZoneOffset carries `__zone`. Period carries
//! `__p_days/__p_months/__p_years`. Enum-like holders (ChronoUnit, ChronoField,
//! DayOfWeek, Month) expose their constants as string-token keys for field reads.

use cfml_common::dynamic::{CfmlValue, ValueMap};
use cfml_common::vm::{CfmlError, CfmlResult};
use chrono::{Datelike, Months, NaiveDateTime, TimeZone, Timelike, Utc};

pub const LOCALDATETIME_CLASS: &str = "java.time.localdatetime";
pub const LOCALDATE_CLASS: &str = "java.time.localdate";
pub const INSTANT_CLASS: &str = "java.time.instant";
pub const ZONEDDATETIME_CLASS: &str = "java.time.zoneddatetime";
pub const DURATION_CLASS: &str = "java.time.duration";
pub const PERIOD_CLASS: &str = "java.time.period";
pub const ZONEID_CLASS: &str = "java.time.zoneid";
pub const ZONEOFFSET_CLASS: &str = "java.time.zoneoffset";
pub const CHRONOUNIT_CLASS: &str = "java.time.temporal.chronounit";
pub const CHRONOFIELD_CLASS: &str = "java.time.temporal.chronofield";
pub const DAYOFWEEK_CLASS: &str = "java.time.dayofweek";
pub const MONTH_CLASS: &str = "java.time.month";
pub const TEMPORALADJUSTERS_CLASS: &str = "java.time.temporal.temporaladjusters";

fn shim_map(class: &str) -> ValueMap {
    let mut m = ValueMap::default();
    m.insert("__java_shim".to_string(), CfmlValue::Bool(true));
    m.insert("__java_class".to_string(), CfmlValue::string(class.to_string()));
    m
}

fn now_millis() -> i64 {
    Utc::now().timestamp_millis()
}

fn millis_to_ndt(ms: i64) -> NaiveDateTime {
    chrono::DateTime::from_timestamp_millis(ms)
        .map(|dt| dt.naive_utc())
        .unwrap_or_else(|| chrono::DateTime::from_timestamp_millis(0).unwrap().naive_utc())
}

fn ndt_to_millis(ndt: NaiveDateTime) -> i64 {
    Utc.from_utc_datetime(&ndt).timestamp_millis()
}

/// An instant-bearing shim (LocalDateTime / Instant / ZonedDateTime) at `ms`.
fn make_instant_like(class: &str, ms: i64, zone: Option<&str>) -> CfmlValue {
    let mut m = shim_map(class);
    m.insert("__dt_millis".to_string(), CfmlValue::Int(ms));
    if let Some(z) = zone {
        m.insert("__zone".to_string(), CfmlValue::string(z.to_string()));
    }
    CfmlValue::strukt(m)
}

fn make_duration(ms: i64) -> CfmlValue {
    let mut m = shim_map(DURATION_CLASS);
    m.insert("__dur_millis".to_string(), CfmlValue::Int(ms));
    CfmlValue::strukt(m)
}

fn make_zone(class: &str, id: &str) -> CfmlValue {
    let mut m = shim_map(class);
    m.insert("__zone".to_string(), CfmlValue::string(id.to_string()));
    CfmlValue::strukt(m)
}

/// Read `__dt_millis` off an instant-bearing shim arg.
fn arg_millis(v: &CfmlValue) -> Option<i64> {
    match v {
        CfmlValue::Struct(s) => match s.get("__dt_millis") {
            Some(CfmlValue::Int(n)) => Some(n),
            Some(other) => other.as_string().trim().parse::<i64>().ok(),
            None => None,
        },
        _ => None,
    }
}

fn arg_dur_millis(v: &CfmlValue) -> Option<i64> {
    match v {
        CfmlValue::Struct(s) => match s.get("__dur_millis") {
            Some(CfmlValue::Int(n)) => Some(n),
            Some(other) => other.as_string().trim().parse::<i64>().ok(),
            None => None,
        },
        _ => None,
    }
}

fn self_millis(object: &CfmlValue) -> i64 {
    arg_millis(object).unwrap_or_else(now_millis)
}

fn arg_i64(args: &[CfmlValue], i: usize) -> i64 {
    args.get(i)
        .map(|v| v.as_string().trim().parse::<i64>().unwrap_or(0))
        .unwrap_or(0)
}

/// Build the ChronoUnit enum holder with all its constant tokens as keys.
fn make_chronounit() -> CfmlValue {
    let mut m = shim_map(CHRONOUNIT_CLASS);
    for u in [
        "NANOS", "MICROS", "MILLIS", "SECONDS", "MINUTES", "HOURS", "HALF_DAYS", "DAYS", "WEEKS",
        "MONTHS", "YEARS", "DECADES", "CENTURIES", "MILLENNIA", "ERAS", "FOREVER",
    ] {
        m.insert(u.to_string(), CfmlValue::string(u.to_string()));
    }
    CfmlValue::strukt(m)
}

fn make_chronofield() -> CfmlValue {
    let mut m = shim_map(CHRONOFIELD_CLASS);
    for f in [
        "NANO_OF_SECOND",
        "MILLI_OF_SECOND",
        "SECOND_OF_MINUTE",
        "MINUTE_OF_HOUR",
        "HOUR_OF_DAY",
        "DAY_OF_WEEK",
        "DAY_OF_MONTH",
        "DAY_OF_YEAR",
        "MONTH_OF_YEAR",
        "YEAR",
    ] {
        m.insert(f.to_string(), CfmlValue::string(f.to_string()));
    }
    CfmlValue::strukt(m)
}

fn make_dayofweek_holder() -> CfmlValue {
    let mut m = shim_map(DAYOFWEEK_CLASS);
    for (i, d) in [
        "MONDAY",
        "TUESDAY",
        "WEDNESDAY",
        "THURSDAY",
        "FRIDAY",
        "SATURDAY",
        "SUNDAY",
    ]
    .iter()
    .enumerate()
    {
        // Each constant is a DayOfWeek instance carrying its 1-7 value.
        let mut dm = shim_map(DAYOFWEEK_CLASS);
        dm.insert("__dow".to_string(), CfmlValue::Int((i + 1) as i64));
        m.insert(d.to_string(), CfmlValue::strukt(dm));
    }
    CfmlValue::strukt(m)
}

fn make_month_holder() -> CfmlValue {
    let mut m = shim_map(MONTH_CLASS);
    for (i, mo) in [
        "JANUARY",
        "FEBRUARY",
        "MARCH",
        "APRIL",
        "MAY",
        "JUNE",
        "JULY",
        "AUGUST",
        "SEPTEMBER",
        "OCTOBER",
        "NOVEMBER",
        "DECEMBER",
    ]
    .iter()
    .enumerate()
    {
        let mut mm = shim_map(MONTH_CLASS);
        mm.insert("__month".to_string(), CfmlValue::Int((i + 1) as i64));
        m.insert(mo.to_string(), CfmlValue::strukt(mm));
    }
    CfmlValue::strukt(m)
}

/// Build a java.time.Instant shim at the given epoch millis. Used by the VM to
/// bridge a native CFML date value's `.toInstant()` into the java.time world
/// (ColdBox's ChronoUnit does `cfDate.toInstant().atZone(zone)`).
pub fn instant_from_millis(ms: i64) -> CfmlValue {
    make_instant_like(INSTANT_CLASS, ms, None)
}

/// Construct a java.time class shim for `createObject("java", class)`.
/// Returns `None` for a class this module doesn't handle.
pub fn construct(class_lower: &str) -> Option<CfmlValue> {
    let v = match class_lower {
        LOCALDATETIME_CLASS => make_instant_like(LOCALDATETIME_CLASS, now_millis(), None),
        LOCALDATE_CLASS => make_instant_like(LOCALDATE_CLASS, now_millis(), None),
        INSTANT_CLASS => make_instant_like(INSTANT_CLASS, now_millis(), None),
        ZONEDDATETIME_CLASS => make_instant_like(ZONEDDATETIME_CLASS, now_millis(), Some("Z")),
        DURATION_CLASS => make_duration(0),
        PERIOD_CLASS => {
            let mut m = shim_map(PERIOD_CLASS);
            m.insert("__p_days".to_string(), CfmlValue::Int(0));
            m.insert("__p_months".to_string(), CfmlValue::Int(0));
            m.insert("__p_years".to_string(), CfmlValue::Int(0));
            CfmlValue::strukt(m)
        }
        ZONEID_CLASS => make_zone(ZONEID_CLASS, "UTC"),
        ZONEOFFSET_CLASS => {
            // ZoneOffset also exposes the static UTC constant as a field.
            let mut m = shim_map(ZONEOFFSET_CLASS);
            m.insert("__zone".to_string(), CfmlValue::string("Z".to_string()));
            m.insert("UTC".to_string(), make_zone(ZONEOFFSET_CLASS, "Z"));
            CfmlValue::strukt(m)
        }
        CHRONOUNIT_CLASS => make_chronounit(),
        CHRONOFIELD_CLASS => make_chronofield(),
        DAYOFWEEK_CLASS => make_dayofweek_holder(),
        MONTH_CLASS => make_month_holder(),
        TEMPORALADJUSTERS_CLASS => CfmlValue::strukt(shim_map(TEMPORALADJUSTERS_CLASS)),
        _ => return None,
    };
    Some(v)
}

/// Dispatch a method call on a java.time shim. `class_lower` is the shim's
/// `__java_class`. Returns `Ok(Null)` for an unrecognised method so the caller
/// can fall through to generic struct dispatch.
pub fn dispatch(class_lower: &str, method: &str, args: Vec<CfmlValue>, object: &CfmlValue) -> CfmlResult {
    let m = method.to_ascii_lowercase();
    match class_lower {
        LOCALDATETIME_CLASS | LOCALDATE_CLASS | ZONEDDATETIME_CLASS => {
            dispatch_datetime(class_lower, &m, args, object)
        }
        INSTANT_CLASS => dispatch_instant(&m, args, object),
        DURATION_CLASS => dispatch_duration(&m, args, object),
        PERIOD_CLASS => dispatch_period(&m, args, object),
        ZONEID_CLASS | ZONEOFFSET_CLASS => dispatch_zone(class_lower, &m, args, object),
        CHRONOUNIT_CLASS => dispatch_chronounit(&m, args),
        DAYOFWEEK_CLASS => dispatch_dayofweek(&m, args, object),
        MONTH_CLASS => dispatch_month(&m, object),
        _ => Err(CfmlError::shim_unhandled(method)),
    }
}

fn dispatch_datetime(class: &str, m: &str, args: Vec<CfmlValue>, object: &CfmlValue) -> CfmlResult {
    // Static-ish constructors (called on the class object).
    match m {
        "now" => return Ok(make_instant_like(class, now_millis(), Some("Z"))),
        "parse" => {
            let s = args.first().map(|v| v.as_string()).unwrap_or_default();
            let ms = parse_datetime(&s).unwrap_or_else(now_millis);
            return Ok(make_instant_like(class, ms, Some("Z")));
        }
        "ofepochsecond" => {
            return Ok(make_instant_like(class, arg_i64(&args, 0) * 1000, Some("Z")));
        }
        _ => {}
    }

    let ms = self_millis(object);
    let ndt = millis_to_ndt(ms);
    let out = |n: i64| Ok(make_instant_like(class, n, Some("Z")));
    match m {
        "plusdays" => out(ms + arg_i64(&args, 0) * 86_400_000),
        "minusdays" => out(ms - arg_i64(&args, 0) * 86_400_000),
        "plushours" => out(ms + arg_i64(&args, 0) * 3_600_000),
        "minushours" => out(ms - arg_i64(&args, 0) * 3_600_000),
        "plusminutes" => out(ms + arg_i64(&args, 0) * 60_000),
        "minusminutes" => out(ms - arg_i64(&args, 0) * 60_000),
        "plusseconds" => out(ms + arg_i64(&args, 0) * 1000),
        "minusseconds" => out(ms - arg_i64(&args, 0) * 1000),
        "plusweeks" => out(ms + arg_i64(&args, 0) * 7 * 86_400_000),
        "minusweeks" => out(ms - arg_i64(&args, 0) * 7 * 86_400_000),
        "plusmonths" => out(ndt_to_millis(add_months(ndt, arg_i64(&args, 0)))),
        "minusmonths" => out(ndt_to_millis(add_months(ndt, -arg_i64(&args, 0)))),
        "plusyears" => out(ndt_to_millis(add_months(ndt, arg_i64(&args, 0) * 12))),
        "minusyears" => out(ndt_to_millis(add_months(ndt, -arg_i64(&args, 0) * 12))),
        // plus(amount, unit) or plus(duration); minus(...) — approximate.
        "plus" => out(ms + amount_millis(&args)),
        "minus" => out(ms - amount_millis(&args)),
        "isbefore" => Ok(CfmlValue::Bool(arg_millis(args.first().unwrap_or(&CfmlValue::Null)).map(|o| ms < o).unwrap_or(false))),
        "isafter" => Ok(CfmlValue::Bool(arg_millis(args.first().unwrap_or(&CfmlValue::Null)).map(|o| ms > o).unwrap_or(false))),
        "isequal" | "equals" => Ok(CfmlValue::Bool(arg_millis(args.first().unwrap_or(&CfmlValue::Null)).map(|o| ms == o).unwrap_or(false))),
        "atzone" | "atstartofday" => {
            let zone = args.first().and_then(zone_of).unwrap_or_else(|| "Z".to_string());
            Ok(make_instant_like(ZONEDDATETIME_CLASS, ms, Some(&zone)))
        }
        "toinstant" => Ok(make_instant_like(INSTANT_CLASS, ms, None)),
        "tolocaldatetime" => Ok(make_instant_like(LOCALDATETIME_CLASS, ms, None)),
        "tolocaldate" => Ok(make_instant_like(LOCALDATE_CLASS, ms, None)),
        "withhour" => out(ndt_to_millis(ndt.with_hour(arg_i64(&args, 0) as u32).unwrap_or(ndt))),
        "withminute" => out(ndt_to_millis(ndt.with_minute(arg_i64(&args, 0) as u32).unwrap_or(ndt))),
        "withsecond" => out(ndt_to_millis(ndt.with_second(arg_i64(&args, 0) as u32).unwrap_or(ndt))),
        "withnano" | "withnanos" => {
            out(ndt_to_millis(ndt.with_nanosecond(arg_i64(&args, 0) as u32).unwrap_or(ndt)))
        }
        "withyear" => out(ndt_to_millis(ndt.with_year(arg_i64(&args, 0) as i32).unwrap_or(ndt))),
        "withmonth" => out(ndt_to_millis(ndt.with_month(arg_i64(&args, 0) as u32).unwrap_or(ndt))),
        "withdayofmonth" => out(ndt_to_millis(ndt.with_day(arg_i64(&args, 0) as u32).unwrap_or(ndt))),
        "truncatedto" | "with" => Ok(object.clone()),
        "toepochsecond" | "getepochsecond" => Ok(CfmlValue::Int(ms / 1000)),
        "toepochmilli" | "tomillis" => Ok(CfmlValue::Int(ms)),
        "getyear" => Ok(CfmlValue::Int(ndt.year() as i64)),
        "getmonthvalue" => Ok(CfmlValue::Int(ndt.month() as i64)),
        "getdayofmonth" => Ok(CfmlValue::Int(ndt.day() as i64)),
        "gethour" => Ok(CfmlValue::Int(ndt.hour() as i64)),
        "getminute" => Ok(CfmlValue::Int(ndt.minute() as i64)),
        "getsecond" => Ok(CfmlValue::Int(ndt.second() as i64)),
        "getdayofweek" => {
            let mut dm = shim_map(DAYOFWEEK_CLASS);
            dm.insert("__dow".to_string(), CfmlValue::Int(ndt.weekday().number_from_monday() as i64));
            Ok(CfmlValue::strukt(dm))
        }
        "format" | "tostring" => Ok(CfmlValue::string(ndt.format("%Y-%m-%dT%H:%M:%S").to_string())),
        _ => Err(CfmlError::shim_unhandled(m)),
    }
}

fn dispatch_instant(m: &str, args: Vec<CfmlValue>, object: &CfmlValue) -> CfmlResult {
    match m {
        "now" => Ok(make_instant_like(INSTANT_CLASS, now_millis(), None)),
        "ofepochmilli" => Ok(make_instant_like(INSTANT_CLASS, arg_i64(&args, 0), None)),
        "ofepochsecond" => Ok(make_instant_like(INSTANT_CLASS, arg_i64(&args, 0) * 1000, None)),
        _ => {
            let ms = self_millis(object);
            match m {
                "toepochmilli" => Ok(CfmlValue::Int(ms)),
                "getepochsecond" => Ok(CfmlValue::Int(ms / 1000)),
                "toinstant" => Ok(object.clone()),
                "atzone" => {
                    let zone = args.first().and_then(zone_of).unwrap_or_else(|| "Z".to_string());
                    Ok(make_instant_like(ZONEDDATETIME_CLASS, ms, Some(&zone)))
                }
                "plusmillis" => Ok(make_instant_like(INSTANT_CLASS, ms + arg_i64(&args, 0), None)),
                "plusseconds" => Ok(make_instant_like(INSTANT_CLASS, ms + arg_i64(&args, 0) * 1000, None)),
                "isbefore" => Ok(CfmlValue::Bool(arg_millis(args.first().unwrap_or(&CfmlValue::Null)).map(|o| ms < o).unwrap_or(false))),
                "isafter" => Ok(CfmlValue::Bool(arg_millis(args.first().unwrap_or(&CfmlValue::Null)).map(|o| ms > o).unwrap_or(false))),
                "tostring" => Ok(CfmlValue::string(millis_to_ndt(ms).format("%Y-%m-%dT%H:%M:%SZ").to_string())),
                _ => Ok(CfmlValue::Null),
            }
        }
    }
}

fn dispatch_duration(m: &str, args: Vec<CfmlValue>, object: &CfmlValue) -> CfmlResult {
    // Static factories.
    match m {
        "ofdays" => return Ok(make_duration(arg_i64(&args, 0) * 86_400_000)),
        "ofhours" => return Ok(make_duration(arg_i64(&args, 0) * 3_600_000)),
        "ofminutes" => return Ok(make_duration(arg_i64(&args, 0) * 60_000)),
        "ofseconds" => return Ok(make_duration(arg_i64(&args, 0) * 1000)),
        "ofmillis" => return Ok(make_duration(arg_i64(&args, 0))),
        "ofnanos" => return Ok(make_duration(arg_i64(&args, 0) / 1_000_000)),
        "of" => {
            // of(amount, unit) — unit is a ChronoUnit token string.
            let amt = arg_i64(&args, 0);
            let unit = args.get(1).map(|v| v.as_string().to_ascii_uppercase()).unwrap_or_default();
            return Ok(make_duration(amt * unit_millis(&unit)));
        }
        "between" => {
            let a = args.first().and_then(arg_millis).unwrap_or(0);
            let b = args.get(1).and_then(arg_millis).unwrap_or(0);
            return Ok(make_duration(b - a));
        }
        _ => {}
    }
    let ms = arg_dur_millis(object).unwrap_or(0);
    match m {
        "tomillis" => Ok(CfmlValue::Int(ms)),
        "getseconds" | "toseconds" => Ok(CfmlValue::Int(ms / 1000)),
        "tominutes" => Ok(CfmlValue::Int(ms / 60_000)),
        "tohours" => Ok(CfmlValue::Int(ms / 3_600_000)),
        "todays" => Ok(CfmlValue::Int(ms / 86_400_000)),
        "getnano" => Ok(CfmlValue::Int((ms % 1000) * 1_000_000)),
        "plus" => Ok(make_duration(ms + args.first().and_then(arg_dur_millis).unwrap_or(0))),
        "minus" => Ok(make_duration(ms - args.first().and_then(arg_dur_millis).unwrap_or(0))),
        "withseconds" => Ok(make_duration(arg_i64(&args, 0) * 1000 + (ms % 1000))),
        "withnanos" => Ok(make_duration((ms / 1000) * 1000 + arg_i64(&args, 0) / 1_000_000)),
        "plusseconds" => Ok(make_duration(ms + arg_i64(&args, 0) * 1000)),
        "plusmillis" => Ok(make_duration(ms + arg_i64(&args, 0))),
        "plusminutes" => Ok(make_duration(ms + arg_i64(&args, 0) * 60_000)),
        "plushours" => Ok(make_duration(ms + arg_i64(&args, 0) * 3_600_000)),
        "plusdays" => Ok(make_duration(ms + arg_i64(&args, 0) * 86_400_000)),
        "isnegative" => Ok(CfmlValue::Bool(ms < 0)),
        "iszero" => Ok(CfmlValue::Bool(ms == 0)),
        "tostring" => Ok(CfmlValue::string(format!("PT{}S", ms / 1000))),
        _ => Err(CfmlError::shim_unhandled(m)),
    }
}

fn dispatch_period(m: &str, args: Vec<CfmlValue>, object: &CfmlValue) -> CfmlResult {
    let mk = |days: i64, months: i64, years: i64| {
        let mut mm = shim_map(PERIOD_CLASS);
        mm.insert("__p_days".to_string(), CfmlValue::Int(days));
        mm.insert("__p_months".to_string(), CfmlValue::Int(months));
        mm.insert("__p_years".to_string(), CfmlValue::Int(years));
        CfmlValue::strukt(mm)
    };
    match m {
        "ofdays" => Ok(mk(arg_i64(&args, 0), 0, 0)),
        "ofweeks" => Ok(mk(arg_i64(&args, 0) * 7, 0, 0)),
        "ofmonths" => Ok(mk(0, arg_i64(&args, 0), 0)),
        "ofyears" => Ok(mk(0, 0, arg_i64(&args, 0))),
        "of" => Ok(mk(arg_i64(&args, 2), arg_i64(&args, 1), arg_i64(&args, 0))),
        _ => {
            let get = |k: &str| match object {
                CfmlValue::Struct(s) => match s.get(k) {
                    Some(CfmlValue::Int(n)) => n,
                    _ => 0,
                },
                _ => 0,
            };
            match m {
                "getdays" => Ok(CfmlValue::Int(get("__p_days"))),
                "getmonths" => Ok(CfmlValue::Int(get("__p_months"))),
                "getyears" => Ok(CfmlValue::Int(get("__p_years"))),
                _ => Ok(CfmlValue::Null),
            }
        }
    }
}

fn dispatch_zone(class: &str, m: &str, args: Vec<CfmlValue>, object: &CfmlValue) -> CfmlResult {
    match m {
        "of" => Ok(make_zone(ZONEID_CLASS, &args.first().map(|v| v.as_string()).unwrap_or_else(|| "UTC".to_string()))),
        "systemdefault" => Ok(make_zone(ZONEID_CLASS, &iana_local_zone())),
        "getid" | "tostring" | "getdisplayname" | "normalized" => {
            let z = zone_of(object).unwrap_or_else(|| "UTC".to_string());
            if m == "normalized" {
                Ok(make_zone(class, &z))
            } else {
                Ok(CfmlValue::string(z))
            }
        }
        _ => Err(CfmlError::shim_unhandled(m)),
    }
}

fn dispatch_chronounit(m: &str, args: Vec<CfmlValue>) -> CfmlResult {
    // The only instance method used is unit.between(a, b) — return the elapsed
    // count in this unit. `between` here is dispatched on the class object with
    // no per-unit context, so callers using `ChronoUnit.SECONDS.between(...)`
    // reach this via the token string, not here; kept for `valueOf`/`values`.
    match m {
        "valueof" => Ok(CfmlValue::string(args.first().map(|v| v.as_string().to_ascii_uppercase()).unwrap_or_default())),
        _ => Err(CfmlError::shim_unhandled(m)),
    }
}

fn dispatch_dayofweek(m: &str, args: Vec<CfmlValue>, object: &CfmlValue) -> CfmlResult {
    match m {
        "of" => {
            let mut dm = shim_map(DAYOFWEEK_CLASS);
            dm.insert("__dow".to_string(), CfmlValue::Int(arg_i64(&args, 0)));
            Ok(CfmlValue::strukt(dm))
        }
        "getvalue" => {
            let v = match object {
                CfmlValue::Struct(s) => match s.get("__dow") {
                    Some(CfmlValue::Int(n)) => n,
                    _ => 1,
                },
                _ => 1,
            };
            Ok(CfmlValue::Int(v))
        }
        _ => Err(CfmlError::shim_unhandled(m)),
    }
}

fn dispatch_month(m: &str, object: &CfmlValue) -> CfmlResult {
    if m == "getvalue" {
        let v = match object {
            CfmlValue::Struct(s) => match s.get("__month") {
                Some(CfmlValue::Int(n)) => n,
                _ => 1,
            },
            _ => 1,
        };
        return Ok(CfmlValue::Int(v));
    }
    Ok(CfmlValue::Null)
}

// ---- helpers ----

fn add_months(ndt: NaiveDateTime, months: i64) -> NaiveDateTime {
    if months >= 0 {
        ndt.checked_add_months(Months::new(months as u32)).unwrap_or(ndt)
    } else {
        ndt.checked_sub_months(Months::new((-months) as u32)).unwrap_or(ndt)
    }
}

/// Milliseconds for `plus(amount, unit)` / `plus(duration)` argument shapes.
fn amount_millis(args: &[CfmlValue]) -> i64 {
    // plus(duration)
    if let Some(d) = args.first().and_then(arg_dur_millis) {
        return d;
    }
    // plus(amount, unit)
    if args.len() >= 2 {
        let amt = args[0].as_string().trim().parse::<i64>().unwrap_or(0);
        let unit = args[1].as_string().to_ascii_uppercase();
        return amt * unit_millis(&unit);
    }
    0
}

fn unit_millis(unit: &str) -> i64 {
    match unit {
        "NANOS" => 0,
        "MICROS" => 0,
        "MILLIS" => 1,
        "SECONDS" => 1000,
        "MINUTES" => 60_000,
        "HOURS" => 3_600_000,
        "HALF_DAYS" => 43_200_000,
        "DAYS" => 86_400_000,
        "WEEKS" => 7 * 86_400_000,
        "MONTHS" => 30 * 86_400_000,
        "YEARS" => 365 * 86_400_000,
        _ => 1000,
    }
}

fn zone_of(v: &CfmlValue) -> Option<String> {
    match v {
        CfmlValue::Struct(s) => match s.get("__zone") {
            Some(z) => Some(z.as_string()),
            None => None,
        },
        CfmlValue::String(s) => Some(s.to_string()),
        _ => None,
    }
}

fn iana_local_zone() -> String {
    // Best-effort local zone id; falls back to UTC.
    std::env::var("TZ").ok().filter(|s| !s.is_empty()).unwrap_or_else(|| "UTC".to_string())
}

fn parse_datetime(s: &str) -> Option<i64> {
    let t = s.trim();
    // Try ISO-8601 with time, then date-only.
    for fmt in ["%Y-%m-%dT%H:%M:%S", "%Y-%m-%dT%H:%M", "%Y-%m-%d %H:%M:%S"] {
        if let Ok(ndt) = NaiveDateTime::parse_from_str(t, fmt) {
            return Some(ndt_to_millis(ndt));
        }
    }
    if let Ok(d) = chrono::NaiveDate::parse_from_str(t, "%Y-%m-%d") {
        return Some(ndt_to_millis(d.and_hms_opt(0, 0, 0)?));
    }
    None
}
