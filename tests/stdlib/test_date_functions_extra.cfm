<cfscript>
suiteBegin("Date Functions Extra");

// --- millisecond ---
assert("millisecond of midnight", millisecond(createDateTime(2024, 6, 15, 0, 0, 0)), 0);
assert("millisecond of a date string", millisecond("2024-06-15 10:30:00"), 0);

// --- dateConvert ---
// We can't assert exact values because offsets depend on the server timezone,
// but we can verify it returns a valid date string and round-trips correctly.
utcDate = dateConvert("local2utc", "2024-06-15 12:00:00");
assertTrue("dateConvert local2utc returns a date", isDate(utcDate));

localDate = dateConvert("utc2local", "2024-06-15 12:00:00");
assertTrue("dateConvert utc2local returns a date", isDate(localDate));

// Round-trip: local->utc->local should give back original
original = "2024-06-15 12:00:00";
roundTripped = dateConvert("utc2local", dateConvert("local2utc", original));
assert("dateConvert round-trip year", year(roundTripped), 2024);
assert("dateConvert round-trip month", month(roundTripped), 6);
assert("dateConvert round-trip day", day(roundTripped), 15);
assert("dateConvert round-trip hour", hour(roundTripped), 12);
assert("dateConvert round-trip minute", minute(roundTripped), 0);

// --- getNumericDate ---
// Dec 30, 1899 is day 0 in CFML date serial numbering
numDate = getNumericDate("1899-12-30");
assert("getNumericDate epoch is 0", int(numDate), 0);

// Jan 1, 2000 is 36526 days after Dec 30, 1899
numDate2000 = getNumericDate("2000-01-01");
assert("getNumericDate 2000-01-01", int(numDate2000), 36526);

// A date with a time component should have a fractional part
numDateNoon = getNumericDate("2000-01-01 12:00:00");
assertTrue("getNumericDate noon has fractional .5", numDateNoon > 36526 && numDateNoon < 36527);

// --- getHTTPTimeString ---
httpStr = getHTTPTimeString("2024-06-15 10:30:00");
assertTrue("getHTTPTimeString contains GMT", find("GMT", httpStr) > 0);
assertTrue("getHTTPTimeString contains year", find("2024", httpStr) > 0);
assertTrue("getHTTPTimeString contains Jun", find("Jun", httpStr) > 0);
assertTrue("getHTTPTimeString contains 15", find("15", httpStr) > 0);
// Time may be converted to GMT, so just check it has a time component
assertTrue("getHTTPTimeString contains time", reFind("\d{2}:\d{2}:\d{2}", httpStr) > 0);

// Verify the day-of-week abbreviation for Saturday June 15, 2024
assertTrue("getHTTPTimeString correct DOW", find("Sat", httpStr) > 0);

// --- blank input to FORMAT functions returns "" (Lucee parity) ---
// dateFormat/timeFormat/dateTimeFormat return an empty string for a blank date,
// UNLIKE dateAdd/year/parseDateTime which throw a cast error. Masa's admin
// content-edit + Staging views pass empty date columns straight to
// dateTimeFormat(); throwing 500'd those pages. Cross-checked on Lucee 7.
assert("dateFormat('') returns empty", dateFormat(""), "");
assert("dateFormat(' ') returns empty", dateFormat(" "), "");
assert("timeFormat('') returns empty", timeFormat(""), "");
assert("dateTimeFormat('') returns empty", dateTimeFormat(""), "");
assert("dateTimeFormat('', mask) returns empty", dateTimeFormat("", "yyyy-mm-dd"), "");
// A real date still formats normally (guard against over-broad blanking).
assert("dateFormat still formats a real date",
	dateFormat("2024-06-15", "yyyy-mm-dd"), "2024-06-15");
// The strict date functions still THROW for blank (Lucee parity — not blanked).
assertThrows("year('') still throws", function(){ year(""); });
assertThrows("parseDateTime('') still throws", function(){ parseDateTime(""); });

// --- nowServer ---
assertTrue("nowServer returns a date", isDate(nowServer()));
// nowServer should return same time as now (approximately)
assert("nowServer year matches now", year(nowServer()), year(now()));
assert("nowServer month matches now", month(nowServer()), month(now()));
assert("nowServer day matches now", day(nowServer()), day(now()));

suiteEnd();
</cfscript>
