<cfscript>
// GH #479: opt-in date precision beyond Lucee's milliseconds. RustCFML-only:
// Lucee has no datePrecision setting, microsecond()/nanosecond(), or the
// us/ns date parts.
suiteBegin("Date precision beyond milliseconds (GH ##479)");

function dpPrecisionRun() {
	var s = "2026-01-02 10:20:30.123456789";

	// Default: milliseconds, exactly as Lucee.
	var d = parseDateTime(s);
	assert("default keeps milliseconds", millisecond(d), 123);
	assert("default microsecond", microsecond(d), 123000);
	assert("default nanosecond", nanosecond(d), 123000000);
	assert("default now() is whole milliseconds", nanosecond(now()) mod 1000000, 0);

	application action="update" datePrecision="microsecond";
	d = parseDateTime(s);
	assert("microsecond precision rounds half up", microsecond(d), 123457);
	assert("microsecond precision nanosecond()", nanosecond(d), 123457000);
	assert("millisecond() is unchanged", millisecond(d), 123);
	assert("now() is whole microseconds", nanosecond(now()) mod 1000, 0);

	application action="update" datePrecision="nanosecond";
	d = parseDateTime(s);
	assert("nanosecond precision", nanosecond(d), 123456789);
	assert("member microsecond", d.microsecond(), 123456);
	assert("member nanosecond", d.nanosecond(), 123456789);
	assert("datePart us", datePart("us", d), 123456);
	assert("datePart ns", datePart("ns", d), 123456789);
	assert("ISO string with nanoseconds", nanosecond(parseDateTime("2026-01-02T10:20:30.987654321Z")), 987654321);

	assert("dateAdd us", nanosecond(dateAdd("us", 1, d)), 123457789);
	assert("dateAdd ns", nanosecond(dateAdd("ns", 211, d)), 123457000);
	assert("dateAdd ns carries into the second", second(dateAdd("ns", 900000000, d)), 31);
	assert("dateDiff us", dateDiff("us", d, dateAdd("us", 7, d)), 7);
	assert("dateDiff ns", dateDiff("ns", d, dateAdd("ns", 5, d)), 5);
	assert("dateDiff ns negative", dateDiff("ns", dateAdd("ns", 5, d), d), -5);
	assert("dateDiff l still milliseconds", dateDiff("l", d, dateAdd("us", 1500, d)), 1);

	var later = dateAdd("ns", 1, d);
	assert("dateCompare ns", dateCompare(d, later, "ns"), -1);
	assert("dateCompare us", dateCompare(d, later, "us"), 0);
	assert("dateCompare l", dateCompare(d, dateAdd("l", 1, d), "l"), -1);
	assertTrue("== still compares to the second (Lucee)", d == later);
	assertFalse("=== is exact", d === later);

	assert("dateTimeFormat nine l digits", dateTimeFormat(d, "HH:nn:ss.lllllllll"), "10:20:30.123456789");
	assert("dateTimeFormat six l digits", dateTimeFormat(d, "HH:nn:ss.llllll"), "10:20:30.123456");
	assert("the string form is unchanged", d & "", "{ts '2026-01-02 10:20:30'}");

	assert("duplicate keeps nanoseconds", nanosecond(duplicate(d)), 123456789);
	assert("a struct keeps nanoseconds", nanosecond({ x: d }.x), 123456789);
	var q = queryNew("id,dt", "integer,timestamp", [[1, dateAdd("ns", 2, d)], [2, d], [3, dateAdd("ns", 1, d)]]);
	var sorted = queryExecute("select id from q order by dt", {}, { dbtype: "query" });
	assert("query of queries orders by nanoseconds", valueList(sorted.id), "2,3,1");

	// Values already made keep their precision when it is lowered again.
	application action="update" datePrecision="millisecond";
	assert("an existing value keeps its nanoseconds", nanosecond(d), 123456789);
	assert("new values are milliseconds again", nanosecond(parseDateTime(s)), 123000000);
}

try {
	dpPrecisionRun();
} finally {
	application action="update" datePrecision="millisecond";
}

suiteEnd();
</cfscript>
