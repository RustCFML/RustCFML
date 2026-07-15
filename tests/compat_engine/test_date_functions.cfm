<cfscript>
// Lucee 7 Compatibility Tests: Date Functions
// Synthesized from https://github.com/lucee/Lucee/tree/7.0/test/functions
// Original tests Copyright (c) 2014, the Railo Company LLC / Copyright (c) 2015-2016, Lucee Association Switzerland
// Licensed under the GNU Lesser General Public License v2.1
// Adapted for RustCFML test harness

// ============================================================
// CreateDate / IsDate (from Lucee CreateDate.cfc)
// ============================================================
suiteBegin("Lucee7: CreateDate/IsDate");
d = createDate(2024, 3, 15);
assertTrue("createDate returns date", isDate(d));
assert("year of createDate", year(d), 2024);
assert("month of createDate", month(d), 3);
assert("day of createDate", day(d), 15);
suiteEnd();

// ============================================================
// CreateDateTime (from Lucee CreateDateTime.cfc)
// ============================================================
suiteBegin("Lucee7: CreateDateTime");
dt = createDateTime(2024, 3, 15, 10, 30, 45);
assertTrue("createDateTime returns date", isDate(dt));
assert("year of createDateTime", year(dt), 2024);
assert("month of createDateTime", month(dt), 3);
assert("day of createDateTime", day(dt), 15);
assert("hour of createDateTime", hour(dt), 10);
assert("minute of createDateTime", minute(dt), 30);
assert("second of createDateTime", second(dt), 45);
suiteEnd();

// ============================================================
// DayOfWeek (from Lucee DayOfWeek.cfc)
// ============================================================
suiteBegin("Lucee7: DayOfWeek");
// 2024-03-15 is a Friday; CFML: Sun=1, Mon=2, ..., Fri=6, Sat=7
assert("dayOfWeek Friday", dayOfWeek(createDate(2024, 3, 15)), 6);
assert("dayOfWeek Sunday", dayOfWeek(createDate(2024, 3, 17)), 1);
assert("dayOfWeek Saturday", dayOfWeek(createDate(2024, 3, 16)), 7);
suiteEnd();

// ============================================================
// DayOfYear (from Lucee DayOfYear.cfc)
// ============================================================
suiteBegin("Lucee7: DayOfYear");
assert("dayOfYear Jan 15", dayOfYear(createDate(2024, 1, 15)), 15);
assert("dayOfYear Jan 1", dayOfYear(createDate(2024, 1, 1)), 1);
assert("dayOfYear Dec 31 leap year", dayOfYear(createDate(2024, 12, 31)), 366);
suiteEnd();

// ============================================================
// DaysInMonth (from Lucee DaysInMonth.cfc)
// ============================================================
suiteBegin("Lucee7: DaysInMonth");
assert("daysInMonth Feb leap year", daysInMonth(createDate(2024, 2, 1)), 29);
assert("daysInMonth Feb non-leap", daysInMonth(createDate(2023, 2, 1)), 28);
assert("daysInMonth Jan", daysInMonth(createDate(2024, 1, 1)), 31);
assert("daysInMonth Apr", daysInMonth(createDate(2024, 4, 1)), 30);
suiteEnd();

// ============================================================
// DaysInYear (from Lucee DaysInYear.cfc)
// ============================================================
suiteBegin("Lucee7: DaysInYear");
assert("daysInYear leap year", daysInYear(createDate(2024, 1, 1)), 366);
assert("daysInYear non-leap", daysInYear(createDate(2023, 1, 1)), 365);
suiteEnd();

// ============================================================
// IsLeapYear (from Lucee IsLeapYear.cfc)
// ============================================================
suiteBegin("Lucee7: IsLeapYear");
assertTrue("isLeapYear 2024", isLeapYear(2024));
assertFalse("isLeapYear 2023", isLeapYear(2023));
assertTrue("isLeapYear 2000", isLeapYear(2000));
assertFalse("isLeapYear 1900", isLeapYear(1900));
suiteEnd();

// ============================================================
// DateAdd (from Lucee DateAdd.cfc)
// ============================================================
suiteBegin("Lucee7: DateAdd");
d = createDate(2024, 3, 15);
d2 = dateAdd("d", 1, d);
assert("dateAdd day", day(d2), 16);
assert("dateAdd day month unchanged", month(d2), 3);

d3 = dateAdd("m", 1, d);
assert("dateAdd month", month(d3), 4);

d4 = dateAdd("yyyy", 1, d);
assert("dateAdd year", year(d4), 2025);

d5 = dateAdd("d", -1, d);
assert("dateAdd negative day", day(d5), 14);
suiteEnd();

// ============================================================
// DateDiff (from Lucee DateDiff.cfc)
// ============================================================
suiteBegin("Lucee7: DateDiff");
assert("dateDiff days", dateDiff("d", createDate(2024, 1, 1), createDate(2024, 1, 31)), 30);
assert("dateDiff zero", dateDiff("d", createDate(2024, 1, 1), createDate(2024, 1, 1)), 0);
assert("dateDiff negative", dateDiff("d", createDate(2024, 1, 31), createDate(2024, 1, 1)), -30);
assert("dateDiff months", dateDiff("m", createDate(2024, 1, 1), createDate(2024, 6, 1)), 5);
assert("dateDiff years", dateDiff("yyyy", createDate(2020, 1, 1), createDate(2024, 1, 1)), 4);
suiteEnd();

// ============================================================
// DateCompare (from Lucee DateCompare.cfc)
// ============================================================
suiteBegin("Lucee7: DateCompare");
assert("dateCompare less than", dateCompare(createDate(2024, 1, 1), createDate(2024, 1, 2)), -1);
assert("dateCompare greater than", dateCompare(createDate(2024, 1, 2), createDate(2024, 1, 1)), 1);
assert("dateCompare equal", dateCompare(createDate(2024, 1, 1), createDate(2024, 1, 1)), 0);
suiteEnd();

// ============================================================
// DateFormat (from Lucee DateFormat.cfc)
// ============================================================
suiteBegin("Lucee7: DateFormat");
d = createDate(2024, 3, 15);
assert("dateFormat yyyy-mm-dd", dateFormat(d, "yyyy-mm-dd"), "2024-03-15");
assert("dateFormat mm/dd/yyyy", dateFormat(d, "mm/dd/yyyy"), "03/15/2024");
assert("dateFormat m/d/yyyy", dateFormat(createDate(2024, 1, 5), "m/d/yyyy"), "1/5/2024");
suiteEnd();

// ============================================================
// TimeFormat (from Lucee TimeFormat.cfc)
// ============================================================
suiteBegin("Lucee7: TimeFormat");
dt = createDateTime(2024, 1, 1, 14, 30, 0);
assert("timeFormat HH:mm:ss", timeFormat(dt, "HH:mm:ss"), "14:30:00");
dt2 = createDateTime(2024, 1, 1, 9, 5, 7);
assert("timeFormat HH:mm:ss single digits", timeFormat(dt2, "HH:mm:ss"), "09:05:07");
suiteEnd();

// ============================================================
// Now (from Lucee Now.cfc)
// ============================================================
suiteBegin("Lucee7: Now");
assertTrue("now() is a date", isDate(now()));
assertTrue("year of now() is reasonable", year(now()) >= 2024);
suiteEnd();

// ============================================================
// Quarter (from Lucee Quarter.cfc)
// ============================================================
suiteBegin("Lucee7: Quarter");
assert("quarter Q1", quarter(createDate(2024, 3, 15)), 1);
assert("quarter Q2", quarter(createDate(2024, 6, 15)), 2);
assert("quarter Q3", quarter(createDate(2024, 7, 1)), 3);
assert("quarter Q4", quarter(createDate(2024, 12, 31)), 4);
suiteEnd();

// ============================================================
// FirstDayOfMonth (from Lucee FirstDayOfMonth.cfc)
// ============================================================
suiteBegin("Lucee7: FirstDayOfMonth");
// firstDayOfMonth returns the day-of-year ordinal for the 1st of that month
assert("firstDayOfMonth January", firstDayOfMonth(createDate(2024, 1, 15)), 1);
assert("firstDayOfMonth March (leap year)", firstDayOfMonth(createDate(2024, 3, 15)), 61);
assert("firstDayOfMonth February", firstDayOfMonth(createDate(2024, 2, 10)), 32);
suiteEnd();

// ============================================================
// MonthAsString / DayOfWeekAsString (from Lucee MonthAsString.cfc, DayOfWeekAsString.cfc)
// ============================================================
suiteBegin("Lucee7: MonthAsString/DayOfWeekAsString");
assert("monthAsString March", monthAsString(3), "March");
assert("monthAsString January", monthAsString(1), "January");
assert("monthAsString December", monthAsString(12), "December");
assert("dayOfWeekAsString Friday", dayOfWeekAsString(6), "Friday");
assert("dayOfWeekAsString Sunday", dayOfWeekAsString(1), "Sunday");
assert("dayOfWeekAsString Saturday", dayOfWeekAsString(7), "Saturday");
suiteEnd();

// ============================================================
// CreateTimeSpan (from Lucee CreateTimeSpan.cfc)
// ============================================================
suiteBegin("Lucee7: CreateTimeSpan");
ts = createTimeSpan(1, 2, 3, 4);
assertTrue("createTimeSpan does not error", isDefined("ts"));
ts2 = createTimeSpan(0, 0, 0, 0);
assertTrue("createTimeSpan zero", isDefined("ts2"));
suiteEnd();

// ============================================================
// GetTickCount (from Lucee GetTickCount.cfc)
// ============================================================
suiteBegin("Lucee7: GetTickCount");
assertTrue("getTickCount > 0", getTickCount() > 0);
t1 = getTickCount();
t2 = getTickCount();
assertTrue("getTickCount monotonic", t2 >= t1);
suiteEnd();

// ============================================================
// GH #273: date-aware EQ/compare — two date VALUES are equal when they name
// the same instant, even if their textual forms differ. A DB temporal column
// stringifies to the plain `YYYY-MM-DD HH:MM:SS` form; createODBCDateTime
// produces the `{ts '...'}` ODBC literal. Comparing the two (e.g. Preside's
// PresideObjectServiceTest) must be true, matching Lucee.
// ============================================================
suiteBegin("GH ##273: date-aware comparison");
plain = "1990-01-01 00:00:00";
odbc  = "{ts '1990-01-01 00:00:00'}";
assertTrue(  "plain EQ odbc-literal",           plain EQ odbc );
assertTrue(  "odbc-literal EQ plain",           odbc EQ plain );
assertTrue(  "createODBCDateTime EQ plain date-only-promoted",
             createODBCDateTime( createDateTime(1990,1,1,0,0,0) ) EQ plain );
assertFalse( "different instants are NOT equal", plain EQ "{ts '1990-01-02 00:00:00'}" );
assertTrue(  "earlier LT later (date-aware)",    plain LT "1990-01-02 00:00:00" );
// A non-date string never engages date comparison (stays lexical).
assertFalse( "plain word NEQ date", "hello" EQ plain );
assertTrue(  "two identical words still equal", "hello" EQ "hello" );
suiteEnd();
</cfscript>
