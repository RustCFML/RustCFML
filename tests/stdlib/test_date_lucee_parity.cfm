<cfscript>
// GH #441: dates are values (an instant with millisecond precision), not
// strings. Every expected value here was captured from Lucee 7.1 running in
// Europe/London, so the file pins that zone and restores the previous one.
suiteBegin("Date values: Lucee parity (GH ##441)");

function dpExact(required string label, required actual, required string expected) {
	// The harness's assert() compares with ==, which is date-aware ({ts '…'}
	// equals the plain form). String forms need an exact compare.
	var a = isSimpleValue(arguments.actual) ? toString(arguments.actual) : "[complex]";
	if (compare(a, arguments.expected) == 0) {
		request._test_suitePassed++;
	} else {
		request._test_suiteFailed++;
		arrayAppend(request._test_suiteFailures, arguments.label & " | expected: [" & arguments.expected & "] | got: [" & a & "]");
	}
}

function dpThrows(required string label, required any fn, required string messageFragment) {
	var msg = "";
	try {
		fn();
		msg = "[no error]";
	} catch (any e) {
		msg = e.message;
	}
	if (findNoCase(arguments.messageFragment, msg)) {
		request._test_suitePassed++;
	} else {
		request._test_suiteFailed++;
		arrayAppend(request._test_suiteFailures, arguments.label & " | expected an error containing [" & arguments.messageFragment & "] | got: [" & msg & "]");
	}
}

function dpRun() {
	var fmt = "yyyy-mm-dd HH:nn:ss.lll";
	var d = createDateTime(2026, 1, 2, 10, 20, 30);
	var dms = dateAdd("l", 250, d);
	var d2 = createDateTime(2026, 3, 29, 23, 5, 9);

	// ---- string forms
	dpExact("createDateTime string form", d & "", "{ts '2026-01-02 10:20:30'}");
	dpExact("toString of a date", toString(d), "{ts '2026-01-02 10:20:30'}");
	dpExact("milliseconds do not print", dms & "", "{ts '2026-01-02 10:20:30'}");
	dpExact("createDate is midnight", createDate(2026, 1, 2) & "", "{ts '2026-01-02 00:00:00'}");
	dpExact("createTime prints {t}", createTime(10, 20, 30) & "", "{t '10:20:30'}");
	assert("createTime is on 1899-12-30", year(createTime(10, 20, 30)), 1899);
	dpExact("createODBCDate prints {d}", createODBCDate(d) & "", "{d '2026-01-02'}");
	dpExact("createODBCTime prints {t}", createODBCTime(d) & "", "{t '10:20:30'}");
	dpExact("createODBCDateTime prints {ts}", createODBCDateTime(d) & "", "{ts '2026-01-02 10:20:30'}");
	assert("len of a date is its string form", len(d), 26);
	assert("val of a date", val(d), 0);
	dpExact("concat", "x" & d, "x{ts '2026-01-02 10:20:30'}");
	dpExact("a date as a struct key", structKeyList({ "#d#": 1 }), "{ts '2026-01-02 10:20:30'}");

	// ---- milliseconds survive
	assert("millisecond after dateAdd l", millisecond(dms), 250);
	assert("createDateTime 7th argument", millisecond(createDateTime(2026, 1, 2, 10, 20, 30, 123)), 123);
	dpExact("createDateTime ms beyond 999 roll over", dateTimeFormat(createDateTime(2026, 1, 2, 10, 20, 30, 1500), fmt), "2026-01-02 10:20:31.500");
	assert("parseDateTime keeps ms", millisecond(parseDateTime("2026-01-02 10:20:30.250")), 250);
	assert("ISO with T", millisecond(parseDateTime("2026-01-02T10:20:30.250")), 250);
	assert("ISO with Z", millisecond(parseDateTime("2026-01-02T10:20:30.250Z")), 250);
	assert("microseconds round to ms", millisecond(parseDateTime("2026-01-02 10:20:30.123456")), 123);
	assert("one fraction digit is tenths", millisecond(parseDateTime("2026-01-02 10:20:30.5")), 500);
	assert("two fraction digits", millisecond(parseDateTime("2026-01-02 10:20:30.12")), 120);
	assert("fraction rounds half up", millisecond(parseDateTime("2026-01-02 10:20:30.1235")), 124);
	dpExact("fraction rounding carries into the second", dateTimeFormat(parseDateTime("2026-01-02 10:20:30.9995"), fmt), "2026-01-02 10:20:31.000");
	assert("lsParseDateTime keeps ms", millisecond(lsParseDateTime("2026-01-02 10:20:30.250")), 250);
	assert("millisecond of a date string", millisecond("2026-01-02 10:20:30.250"), 250);
	assert("ms survive duplicate", millisecond(duplicate(dms)), 250);
	assert("ms survive a struct", millisecond({ x: dms }.x), 250);
	assert("ms survive an array", millisecond([dms][1]), 250);
	var q = queryNew("d", "timestamp", [[dms]]);
	assert("ms survive a query", millisecond(q.d[1]), 250);
	var qq = queryExecute("select d from q", {}, { dbtype: "query" });
	assert("ms survive query of queries", millisecond(qq.d[1]), 250);
	assert("ms survive createODBCDateTime", millisecond(createODBCDateTime(dms)), 250);
	assert("ms survive createODBCTime", millisecond(createODBCTime(dateAdd("l", 5, d))), 5);

	// ---- dateAdd
	var adds = { yyyy: "2029-01-02 10:20:30.250", q: "2026-10-02 10:20:30.250", m: "2026-04-02 10:20:30.250",
		y: "2026-01-05 10:20:30.250", d: "2026-01-05 10:20:30.250", w: "2026-01-07 10:20:30.250",
		ww: "2026-01-23 10:20:30.250", h: "2026-01-02 13:20:30.250", n: "2026-01-02 10:23:30.250",
		s: "2026-01-02 10:20:33.250", l: "2026-01-02 10:20:30.253" };
	for (var part in adds) {
		dpExact("dateAdd " & part, dateTimeFormat(dateAdd(part, 3, dms), fmt), adds[part]);
	}
	dpExact("dateAdd l negative", dateTimeFormat(dateAdd("l", -251, d), fmt), "2026-01-02 10:20:29.749");
	dpExact("dateAdd l over a day", dateTimeFormat(dateAdd("l", 86400250, d), fmt), "2026-01-03 10:20:30.250");
	dpExact("dateAdd truncates a fractional number", dateTimeFormat(dateAdd("l", 2.7, d), fmt), "2026-01-02 10:20:30.002");
	dpExact("dateAdd m pins to month end", dateAdd("m", 1, createDate(2026, 1, 31)) & "", "{ts '2026-02-28 00:00:00'}");
	dpExact("dateAdd w from Saturday", dateFormat(dateAdd("w", 1, createDate(2026, 1, 3)), "yyyy-mm-dd ddd"), "2026-01-05 Mon");
	dpExact("dateAdd w backwards", dateFormat(dateAdd("w", -3, createDate(2026, 1, 7)), "yyyy-mm-dd ddd"), "2026-01-02 Fri");
	dpExact("dateAdd w five weekdays", dateFormat(dateAdd("w", 5, createDate(2026, 1, 2)), "yyyy-mm-dd ddd"), "2026-01-09 Fri");
	dpExact("dateAdd w zero from Saturday", dateFormat(dateAdd("w", 0, createDate(2026, 1, 3)), "yyyy-mm-dd ddd"), "2026-01-05 Mon");
	dpThrows("dateAdd bad datepart", function() { return dateAdd("x", 1, d); }, "invalid datepart identifier [x]");

	// ---- dateDiff
	var diffs = { yyyy: 0, q: 0, m: 2, y: 86, d: 86, w: 12, ww: 12, h: 2075, n: 124544, s: 7472678, l: 7472678750 };
	for (var part in diffs) {
		assert("dateDiff " & part & " across DST", dateDiff(part, dms, d2), diffs[part]);
	}
	assert("dateDiff l", dateDiff("l", d, dms), 250);
	assert("dateDiff s ignores ms", dateDiff("s", d, dateAdd("l", 999, d)), 0);
	assert("dateDiff d with 47 hours", dateDiff("d", d, dateAdd("h", 47, d)), 1);
	assert("dateDiff m whole months only", dateDiff("m", createDateTime(2026, 1, 15, 12, 0, 0), createDateTime(2026, 2, 15, 11, 0, 0)), 0);
	assert("dateDiff m to month end", dateDiff("m", createDate(2026, 1, 31), createDate(2026, 2, 28)), 1);
	assert("dateDiff m negative", dateDiff("m", createDate(2026, 3, 15), createDate(2026, 1, 20)), -1);
	assert("dateDiff q whole quarters", dateDiff("q", createDate(2026, 1, 15), createDate(2026, 4, 14)), 0);
	assert("dateDiff yyyy whole years", dateDiff("yyyy", createDate(2026, 6, 1), createDate(2027, 5, 31)), 0);
	assert("dateDiff h across spring forward", dateDiff("h", createDateTime(2026, 3, 28, 12, 0, 0), createDateTime(2026, 3, 29, 12, 0, 0)), 23);
	assert("dateDiff d across spring forward", dateDiff("d", createDateTime(2026, 3, 28, 12, 0, 0), createDateTime(2026, 3, 29, 12, 0, 0)), 1);
	assert("dateDiff wd", dateDiff("wd", d, d), 0);
	dpThrows("dateDiff bad datepart", function() { return dateDiff("x", d, d); }, "valid values has to be [l, q, s, n, h, d, m, y, yyyy, w, ww, wd]");

	// ---- comparison is to the second
	assertTrue("== ignores milliseconds", d == dms);
	assertFalse("< ignores milliseconds", d < dms);
	assert("compare() of two dates", compare(d, dms), 0);
	assert("dateCompare default is seconds", dateCompare(d, dms), 0);
	assert("dateCompare d", dateCompare(d, d2, "d"), -1);
	assert("dateCompare m", dateCompare(d, createDate(2026, 1, 28), "m"), 0);
	dpThrows("dateCompare bad datepart", function() { return dateCompare(d, dms, "x"); }, "valid values has to be [s,n,h,d,m,y,yyyy]");
	assertTrue("date equals its plain string", d == "2026-01-02 10:20:30");
	assertTrue("date equals its {ts} string", d == "{ts '2026-01-02 10:20:30'}");
	assertTrue("date equals a string with ms", d == "2026-01-02 10:20:30.999");
	assertFalse("date vs a non-date string", d == "{ts '2026-01-02 10:20:30'}x");
	assertTrue("date equals its numeric value", d == 46024.43090277778);
	assertTrue("date is after a smaller number", d > 46024);

	// ---- numbers
	assert("getNumericDate", getNumericDate(d), 46024.43090277778);
	assert("getNumericDate with ms", getNumericDate(dms), 46024.430905671295);
	assert("date + 1 is a number", d + 1, 46025.43090277778);
	assertFalse("date + 1 is not a date", isDate(d + 1));
	assert("date - date", dms - d, 0.000002893517);
	assertFalse("isNumeric of a date", isNumeric(d));
	assertTrue("isValid numeric of a date", isValid("numeric", d));
	assertTrue("isNumericDate number", isNumericDate(46023.5));
	assertTrue("isNumericDate numeric string", isNumericDate("12"));
	assertFalse("isNumericDate word", isNumericDate("abc"));
	dpExact("a number as a date", dateTimeFormat(46023.5, fmt), "2026-01-01 12:00:00.000");
	assert("ms of a numeric date", millisecond(46023.50000289352), 250);
	assert("getTime", d.getTime(), 1767349230000);
	assert("getTime with ms", dms.getTime(), 1767349230250);
	assert("getTime of the epoch in London", createDateTime(1970, 1, 1, 0, 0, 0).getTime(), -3600000);

	// ---- dateTimeFormat
	dpExact("epoch", dateTimeFormat(dms, "epoch"), "1767349230");
	dpExact("epochms", dateTimeFormat(dms, "epochms"), "1767349230250");
	dpExact("iso", dateTimeFormat(dms, "iso"), "2026-01-02T10:20:30Z");
	dpExact("iso8601", dateTimeFormat(dms, "iso8601"), "2026-01-02T10:20:30Z");
	dpExact("isoms", dateTimeFormat(dms, "isoms"), "2026-01-02T10:20:30.250Z");
	dpExact("javascript", dateTimeFormat(dms, "javascript"), "2026-01-02T10:20:30.250Z");
	dpExact("iso in summer", dateTimeFormat(createDate(2026, 7, 1), "iso"), "2026-07-01T00:00:00+01:00");
	dpExact("default mask", dateTimeFormat(dms), "02-Jan-2026 10:20:30");
	dpExact("short", dateTimeFormat(dms, "short"), "1/2/26, 10:20" & chr(8239) & "AM");
	dpExact("medium", dateTimeFormat(dms, "medium"), "Jan 2, 2026, 10:20:30" & chr(8239) & "AM");
	dpExact("long", dateTimeFormat(dms, "long"), "January 2, 2026, 10:20:30" & chr(8239) & "AM GMT");
	dpExact("full", dateTimeFormat(dms, "full"), "Friday, January 2, 2026, 10:20:30" & chr(8239) & "AM Greenwich Mean Time");
	dpExact("l in dateTimeFormat is a fraction", dateTimeFormat(dms, "yyyy-mm-dd HH:nn:ss.lll"), "2026-01-02 10:20:30.250");
	dpExact("Z", dateTimeFormat(dms, "yyyy-MM-dd'T'HH:nn:ss.lllZ"), "2026-01-02T10:20:30.250+0000");
	dpExact("XXX", dateTimeFormat(dms, "yyyy-mm-dd'T'HH:nn:ssXXX"), "2026-01-02T10:20:30Z");
	dpExact("z", dateTimeFormat(dms, "yyyy-mm-dd HH:nn:ss z"), "2026-01-02 10:20:30 GMT");
	dpExact("S is seconds", dateTimeFormat(dms, "SSS"), "30");
	dpExact("gg", dateTimeFormat(dms, "gg"), "AD");
	dpExact("E", dateTimeFormat(dms, "E"), "Fri");
	dpExact("EEEE", dateTimeFormat(dms, "EEEE"), "Friday");
	dpExact("w", dateTimeFormat(dms, "w"), "1");
	dpExact("ww", dateTimeFormat(dms, "ww"), "01");
	dpExact("a", dateTimeFormat(dms, "a"), "AM");
	dpExact("k", dateTimeFormat(dms, "k"), "10");
	dpExact("K", dateTimeFormat(dms, "K"), "10");
	dpExact("zzzz", dateTimeFormat(createDate(2026, 7, 1), "zzzz z"), "British Summer Time BST");
	dpExact("x and X families", dateTimeFormat(createDate(2026, 7, 1), "x xx xxx X XX XXX Z"), "+01 +0100 +01:00 +01 +0100 +01:00 +0100");
	dpExact("timezone argument", dateTimeFormat(d2, "yyyy-mm-dd HH:nn:ss z", "America/New_York"), "2026-03-29 18:05:09 EDT");
	dpExact("UTC zone argument", dateTimeFormat(dms, "X Z z", "UTC"), "Z +0000 UTC");
	dpExact("quoted text", dateTimeFormat(dms, "'yyyy' yyyy"), "yyyy 2026");
	dpExact("unknown letters are text", dateTimeFormat(dms, "b c f e A B C I J O P Q R T U V"), "b c f e A B C I J O P Q R A U V");
	dpExact("at becomes the marker", dateTimeFormat(dms, "yyyy-mm-dd at HH:nn"), "2026-01-02 AM 10:20");
	dpExact("single t", dateTimeFormat(dms, "h:nn t"), "10:20 A");
	dpExact("yyyyy", dateTimeFormat(dms, "yyyyy"), "02026");
	dpExact("mmmmm caps at the month name", dateTimeFormat(dms, "mmmmm"), "January");
	dpExact("week of year in December", dateTimeFormat(createDate(2025, 12, 30), "w"), "1");
	dpExact("F", dateTimeFormat(createDate(2026, 1, 9), "F"), "2");
	dpExact("W", dateTimeFormat(createDate(2026, 1, 9), "W"), "2");
	dpThrows("reserved character", function() { return dateTimeFormat(d, "[yyyy] {x}"); }, "reserved character");
	var d5 = dateAdd("l", 5, d);
	dpExact("dateTimeFormat l", dateTimeFormat(d5, "l"), "0");
	dpExact("dateTimeFormat lll", dateTimeFormat(d5, "lll"), "005");
	dpExact("dateTimeFormat llll", dateTimeFormat(d5, "llll"), "0050");

	// ---- timeFormat
	dpExact("timeFormat default", timeFormat(dms), "10:20 AM");
	dpExact("timeFormat l", timeFormat(d5, "l"), "5");
	dpExact("timeFormat ll", timeFormat(d5, "ll"), "05");
	dpExact("timeFormat lll", timeFormat(d5, "lll"), "005");
	dpExact("timeFormat llll", timeFormat(d5, "llll"), "0055");
	dpExact("timeFormat lllll", timeFormat(d5, "lllll"), "00505");
	dpExact("timeFormat L", timeFormat(dateAdd("l", 50, d), "L"), "50");
	dpExact("timeFormat HH:nn:ss.lll", timeFormat(dms, "HH:nn:ss.lll"), "10:20:30.250");
	dpExact("timeFormat styles", timeFormat(dms, "short") & "|" & timeFormat(dms, "medium") & "|" & timeFormat(createDate(2026, 7, 1), "long") & "|" & timeFormat(createDate(2026, 7, 1), "full"),
		"10:20 AM|10:20:30 AM|12:00:00 AM BST|12:00:00 AM British Summer Time");
	dpExact("timeFormat zone letters", timeFormat(createDate(2026, 7, 1), "X XX XXX Z z zzzz"), "+01 +0100 +01:00 +0100 BST British Summer Time");
	dpExact("timeFormat X in UTC", timeFormat(dms, "X Z z", "UTC"), "Z +0000 UTC");
	dpExact("timeFormat fields", timeFormat(createDateTime(2026, 1, 2, 0, 5, 7), "h hh H HH m mm n N s ss t tt"), "12 12 0 00 5 05 5 5 7 07 A AM");
	dpExact("timeFormat afternoon", timeFormat(createDateTime(2026, 1, 2, 13, 5, 7), "h hh H HH tt t"), "1 01 13 13 PM P");
	dpExact("timeFormat has no quoting", timeFormat(dms, "'HH'"), "'10'");

	// ---- dateFormat
	dpExact("dateFormat default", dateFormat(dms), "02-Jan-26");
	dpExact("dateFormat styles", dateFormat(dms, "short") & "|" & dateFormat(dms, "medium") & "|" & dateFormat(dms, "long") & "|" & dateFormat(dms, "full"),
		"1/2/26|Jan 2, 2026|January 2, 2026|Friday, January 2, 2026");
	dpExact("dateFormat iso", dateFormat(dms, "iso"), "2026-01-02");
	dpExact("dateFormat l is text", dateFormat(dms, "yyyy-mm-dd l"), "2026-01-02 l");
	dpExact("dateFormat letters", dateFormat(dms, "gg G W WW w ww y yyy Y d dd ddd dddd m mm mmm mmmm"), "AD AD 1 01 1 01 2026 2026 2026 2 02 Fri Friday 1 01 Jan January");
	dpExact("dateFormat w in December", dateFormat(createDate(2025, 12, 30), "w"), "1");
	assert("week() in December", week(createDate(2025, 12, 30)), 53);

	// ---- member functions
	dpExact("format member", dms.format(fmt), "2026-01-02 10:20:30.250");
	assert("add member", dms.add("l", 10).millisecond(), 260);
	assert("diff member", dms.diff("l", d), 250);
	assert("millisecond member", dms.millisecond(), 250);
	assert("part member", d.part("h"), 10);
	assert("lsDayOfWeek member", d.lsDayOfWeek(), 6);
	assertTrue("before", d.before(dateAdd("d", 1, d)));
	assert("compareTo", d.compareTo(dateAdd("d", 1, d)), -1);
	assertTrue("equals", d.equals(createDateTime(2026, 1, 2, 10, 20, 30)));
	dpExact("toString member", d.toString(), "{ts '2026-01-02 10:20:30'}");
	dpExact("toInstant", d.toInstant().toString(), "2026-01-02T10:20:30Z");
	assert("getTimezoneOffset", createDateTime(2026, 7, 2, 10, 20, 30).getTimezoneOffset(), -60);
	assert("getYear", createDateTime(2026, 7, 2, 10, 20, 30).getYear(), 126);
	var s1 = createDateTime(2026, 1, 2, 10, 20, 30);
	s1.setDay(40);
	dpExact("setDay rolls over", s1 & "", "{ts '2026-02-09 10:20:30'}");
	var s2 = createDateTime(2026, 1, 2, 10, 20, 30);
	s2.setMillisecond(1500);
	dpExact("setMillisecond rolls over", dateTimeFormat(s2, fmt), "2026-01-02 10:20:31.500");
	var s3 = createDateTime(2026, 1, 31, 10, 20, 30);
	s3.setMonth(2);
	dpExact("setMonth rolls over", s3 & "", "{ts '2026-03-03 10:20:30'}");
	var s4 = createDateTime(2028, 2, 29, 10, 20, 30);
	s4.setYear(2029);
	dpExact("setYear rolls over", s4 & "", "{ts '2029-03-01 10:20:30'}");
	dpThrows("string members are not date members", function() { return d.left(4); }, "does not exist in the Datetime");

	// ---- types
	assertTrue("isDate", isDate(dms));
	assertTrue("isSimpleValue", isSimpleValue(dms));
	assertFalse("isBoolean", isBoolean(d));
	assertTrue("isValid date", isValid("date", dms));
	assertTrue("isValid string", isValid("string", d));
	dpExact("getClass", dms.getClass().getName(), "lucee.runtime.type.dt.DateTimeImpl");
	assertTrue("isInstanceOf java.util.Date (GH ##471)", isInstanceOf(dms, "java.util.Date"));
	assertTrue("a timestamp from a query is a java.util.Date", isInstanceOf(queryNew("dt", "timestamp", [[now()]]).dt[1], "java.util.Date"));
	assertFalse("a date-like string is not a java.util.Date", isInstanceOf("March 2024", "java.util.Date"));
	var typed = function(date x) { return millisecond(x); };
	assert("date argument keeps ms", typed(dms), 250);
	assert("date argument from a string", typed("2026-01-02 10:20:30.250"), 250);
	dpThrows("a date is not a boolean", function() { return d ? 1 : 0; }, "Can't cast Date [Fri, 02-Jan-2026 10:20:30 GMT] to boolean value");

	// ---- construction errors and leniency
	dpThrows("month 13", function() { return createDateTime(2026, 13, 2, 10, 20, 30); }, "Month number [13] can not be greater than 12");
	dpThrows("second 75", function() { return createDateTime(2026, 1, 2, 10, 20, 75); }, "Second number [75] can not be greater than 59");
	dpThrows("month 0", function() { return createDateTime(2026, 0, 2); }, "Month number [0] must be at least 1");
	dpThrows("30 February", function() { return createDateTime(2026, 2, 30); }, "Day number [30] can not be greater than 28 when month is 2 and year 2026");
	dpExact("hour 24 is the next day", createDateTime(2026, 1, 2, 24, 0, 0) & "", "{ts '2026-01-03 00:00:00'}");
	dpExact("two-digit year", createDate(26, 1, 2) & "", "{ts '2026-01-02 00:00:00'}");
	dpExact("createTime hour 25", createTime(25, 0, 0) & "", "{t '01:00:00'}");
	dpExact("parseDateTime time only", parseDateTime("10:20:30") & "", "{ts '1899-12-30 10:20:30'}");
	dpExact("parseDateTime {d}", parseDateTime("{d '2026-01-02'}") & "", "{ts '2026-01-02 00:00:00'}");
	dpExact("parseDateTime {t}", parseDateTime("{t '10:20:30'}") & "", "{t '10:20:30'}");
	dpThrows("dayOfWeekAsString 8", function() { return dayOfWeekAsString(8); }, "must be between 1 and 7 now [8]");
	dpThrows("monthAsString 13", function() { return monthAsString(13); }, "must be between 1 and 12 now [13]");
	assert("datePart w ww l", datePart("w", dms) & datePart("ww", dms) & datePart("l", dms), "61250");
	dpThrows("datePart bad", function() { return datePart("x", d); }, "invalid datepart type [x]");

	// ---- daylight saving (Europe/London)
	dpExact("a time in the spring gap moves forward", dateTimeFormat(createDateTime(2026, 3, 29, 1, 30, 0), "yyyy-mm-dd HH:nn:ss"), "2026-03-29 02:30:00");
	dpExact("dateAdd h across the gap", dateTimeFormat(dateAdd("h", 1, createDateTime(2026, 3, 29, 0, 30, 0)), "yyyy-mm-dd HH:nn:ss"), "2026-03-29 02:30:00");
	dpExact("dateAdd d into the gap", dateTimeFormat(dateAdd("d", 1, createDateTime(2026, 3, 28, 1, 30, 0)), "yyyy-mm-dd HH:nn:ss"), "2026-03-29 00:30:00");
	assert("an ambiguous time is standard time", createDateTime(2026, 10, 25, 1, 30, 0).getTime(), 1792891800000);
	dpExact("offset string read as an instant", dateTimeFormat(parseDateTime("2026-01-02T10:20:30.250-05:00"), fmt), "2026-01-02 15:20:30.250");
	dpExact("local2utc", dateConvert("local2utc", createDateTime(2026, 7, 1, 12, 0, 0)) & "", "{ts '2026-07-01 11:00:00'}");
	dpExact("utc2local", dateConvert("utc2local", createDateTime(2026, 7, 1, 12, 0, 0)) & "", "{ts '2026-07-01 13:00:00'}");
	dpExact("getHttpTimeString", getHttpTimeString(createDateTime(2026, 7, 1, 12, 0, 0)), "Wed, 01 Jul 2026 11:00:00 GMT");
	var summer = createDateTime(2026, 7, 1, 12, 0, 0);
	setTimeZone("America/New_York");
	dpExact("the same date renders in the new zone", summer & " " & hour(summer), "{ts '2026-07-01 07:00:00'} 7");
	setTimeZone("Europe/London");

	// ---- serialization
	dpExact("serializeJSON", serializeJSON(d), '"January, 02 2026 10:20:30 +0000"');
	dpExact("serializeJSON in summer", serializeJSON(createDateTime(2026, 7, 1, 9, 5, 3)), '"July, 01 2026 09:05:03 +0100"');
	dpExact("serializeJSON of a query", serializeJSON(queryNew("d", "timestamp", [[createDateTime(2026, 7, 1, 9, 5, 3)]])), '{"COLUMNS":["d"],"DATA":[["July, 01 2026 09:05:03 +0100"]]}');
	dpExact("serializeJSON {d} and {t}", serializeJSON([createODBCDate(createDateTime(2026, 7, 1, 9, 5, 3)), createTime(1, 2, 3)]), '["July, 01 2026 00:00:00 +0100","December, 30 1899 01:02:03 +0000"]');
	assertTrue("the JSON form reads back as a date", isDate(deserializeJSON(serializeJSON(dms))));
	dpExact("serialize", serialize(createDateTime(2026, 7, 1, 9, 5, 3)), 'createDateTime(2026,7,1,9,5,3,0,"Europe/London")');
}

origTz = getTimeZone();
setTimeZone("Europe/London");
try {
	dpRun();
} finally {
	setTimeZone(origTz);
}

suiteEnd();
</cfscript>
