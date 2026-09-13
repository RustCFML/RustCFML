<cfscript>
// An offset-bearing date string names an absolute instant, and CFML reports
// instants in the REQUEST timezone (setTimeZone / this.timezone). We resolved
// the offset against the SYSTEM zone instead, so the two disagreed by the whole
// zone offset whenever they differed — `dateConvert("utc2Local", d)` used the
// request zone while parsing the same instant used the system zone.
//
// It hid on developer machines because the system zone usually IS the request
// zone; it only surfaced on a UTC CI box. The assertions below use
// Asia/Kolkata, which is UTC+5:30 with NO daylight saving, so they hold on any
// host whatever its own zone — that is what makes this test portable rather
// than a re-run of the bug that hid the bug.
//
// Measured on Lucee 7.1.0.204: with setTimeZone("Asia/Kolkata"),
// parseDateTime("August, 25 2026 09:00:14 +0000") is 2026-08-25 14:30:14 and
// the dateDiff against dateConvert("utc2Local", ...) is 0. We returned the
// instant in the system zone.
suiteBegin("Offset-bearing dates parse into the request timezone");

originalTZ = getTimeZone();
setTimeZone("Asia/Kolkata");

// 09:00:14 UTC == 14:30:14 in Kolkata (+05:30).
offsetForm = "August, 25 2026 09:00:14 +0000";
utcInstant = createDateTime(2026, 8, 25, 9, 0, 14);

assert("dateConvert reports the instant in the request zone",
	dateTimeFormat(dateConvert("utc2Local", utcInstant), "yyyy-mm-dd HH:nn:ss"),
	"2026-08-25 14:30:14");

assert("parsing an offset-bearing form agrees with dateConvert",
	dateTimeFormat(parseDateTime(offsetForm), "yyyy-mm-dd HH:nn:ss"),
	"2026-08-25 14:30:14");

// The differential the original failure tripped on: the same instant reached
// two ways must compare equal.
assert("parsed offset form and converted instant are the same moment",
	dateDiff("s", dateConvert("utc2Local", utcInstant), offsetForm), 0);

// A non-zero offset too, so this is not just "+0000 is special".
assert("a non-zero offset is applied, not discarded",
	dateTimeFormat(parseDateTime("August, 25 2026 09:00:14 +0200"), "yyyy-mm-dd HH:nn:ss"),
	"2026-08-25 12:30:14");

setTimeZone(originalTZ);

suiteEnd();
</cfscript>
