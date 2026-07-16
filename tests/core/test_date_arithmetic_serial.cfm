<cfscript>
suiteBegin("Core: date arithmetic (numeric serial)");

// ============================================================
// Background
// ============================================================
// In CFML, `+`/`-`/`*` are ARITHMETIC (only `&` concatenates). A date/datetime
// participates as its numeric "serial" value: days since 1899-12-30, with the
// time-of-day in the fraction. So `now() + 1` is tomorrow, `dateB - dateA` is the
// number of days between, and `date + createTimeSpan(...)` shifts the date. The
// RESULT is a numeric serial (isNumeric=true, isDate=false), which date functions
// (dateAdd/dateTimeFormat/...) re-parse via the serial branch of the date parser.
//
// RustCFML stores dates as strings and used to fall back to STRING CONCATENATION
// for `+` when an operand was a date (e.g. `now() + 1` -> "2026-07-16 00:00:001").
// Masa CMS's sessionUserFacade.generateCSRFTokens does
// `dateAdd('l', ms, (currentDateTime + timespan))` — the broken `+` produced a
// malformed "...:100.125" string that dateAdd could not parse ("Invalid date").
// Verified against Lucee 7 (values below match exactly).
// ============================================================

base = createDateTime(2026, 7, 16, 10, 30, 45);

// serial of 2026-07-16 10:30:45 (verified on Lucee 7: 46219.43802083333)
assert("date coerces to its serial via +0", (base + 0), 46219.43802083333);

// +1 day, -1 day, +0.5 day
assert("date + 1 = serial + 1 (a day later)", (base + 1), 46220.43802083333);
assert("date - 1 = serial - 1 (a day earlier)", (base - 1), 46218.43802083333);
assert("date + 0.5 = serial + half a day", (base + 0.5), 46219.93802083333);

// date + timespan (createTimeSpan(0,3,0,0) == 0.125 days == 3 hours)
ts = createTimeSpan(0, 3, 0, 0);
assert("date + timespan(3h) = serial + 0.125", (base + ts), 46219.56302083333);

// date - date = whole days between
d2 = createDateTime(2026, 7, 20, 10, 30, 45);
assert("dateB - dateA = days between", (d2 - base), 4);

// result type: numeric, not a date
assert("date + n is numeric", isNumeric(base + 1), true);
assert("date + n is NOT a date value", isDate(base + 1), false);

// the round-trip that Masa relies on: arithmetic result re-parses as a date
assert("dateAdd re-parses a serial produced by + (adds 3h)",
       dateTimeFormat(dateAdd('l', 0, (base + ts)), "yyyy-mm-dd HH:nn:ss"),
       "2026-07-16 13:30:45");

// a genuinely non-numeric, non-date string still concatenates (leniency preserved)
assert("non-date string + number still concatenates", ("abc" & (1 + 2)), "abc3");

suiteEnd();
</cfscript>
