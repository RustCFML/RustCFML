<cfscript>
suiteBegin("createTimeSpan: distinct timespan type (Lucee parity)");

// RustCFML models createTimeSpan() as a distinct TimeSpan value (numerically the
// fractional-day Double) so JVM-style type introspection works without a JVM.
// This fixes Preside's AdHocTaskManagerService._isTimespan() (a getClass() sniff)
// and `timespan`-typed params. All expectations verified against Lucee 7.0.4.

oneDay = createTimeSpan(1, 0, 0, 0);
mixed  = createTimeSpan(1, 2, 3, 4);   // 1.0854629629629... days

// --- numeric value / arithmetic: behaves exactly like a Double ---
assert("timespan is fractional days (1 day == 1)", oneDay, 1);
assert("timespan arithmetic (oneDay + 1)", oneDay + 1, 2);
assert("timespan * 86400 == seconds in a day", round(oneDay * 86400), 86400);
assert("val(timespan)", val(oneDay), 1);
assertTrue("timespan compares numerically (oneDay lt mixed)", oneDay lt mixed);
assertTrue("timespan equals its numeric value", oneDay == 1);

// --- type introspection (the whole point of the distinct type) ---
assertTrue("getClass().getName() contains 'timespan'",
    findNoCase("timespan", oneDay.getClass().getName()) gt 0);
assertTrue("isSimpleValue(timespan) is true", isSimpleValue(oneDay));
assertFalse("isNumeric(timespan) is false (Lucee)", isNumeric(oneDay));
assertFalse("isBoolean(timespan) is false (Lucee)", isBoolean(oneDay));
assertTrue("isDate(timespan) is true (Lucee)", isDate(oneDay));

// --- Lucee TimeSpan accessor methods: getSeconds() = TOTAL secs, getSecond() = component ---
span = createTimeSpan(3, 5, 25, 35);   // 3d 5h 25m 35s = 278735 total seconds
assert("getSeconds() is total seconds", span.getSeconds(), 3*86400 + 5*3600 + 25*60 + 35);
assert("getSecond() is the seconds component", span.getSecond(), 35);

// --- Preside's _isTimespan() sniff pattern ---
isTs = findNoCase("timespan", oneDay.getClass().getName()) ? true : false;
assertTrue("_isTimespan() sniff returns true", isTs);

// --- a `timespan`-typed parameter accepts a createTimeSpan value ---
_tsTypedParamProbe = function( timespan t ) { return "ok"; };
assert("timespan-typed param accepts a timespan value",
    _tsTypedParamProbe( createTimeSpan(0, 0, 0, 5) ), "ok");

// --- serializeJSON emits the numeric value (not null) ---
assert("serializeJSON(timespan) is its number", serializeJSON(oneDay), "1");

// --- a plain number is NOT a timespan (only createTimeSpan produces one) ---
assertFalse("plain double is not a timespan class",
    findNoCase("timespan", (1.5).getClass().getName()) gt 0);

suiteEnd();
</cfscript>
