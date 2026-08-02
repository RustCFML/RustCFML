<cfscript>
// ============================================================
// Locale (ls*) Functions Tests
// ============================================================

// --- setLocale / getLocale ---
suiteBegin("setLocale / getLocale");
// GH #304: this used to read `assert(result, "en_US")` — asserting that setLocale
// returns the locale it just SET. Lucee returns the PREVIOUS one (that is what
// makes the return value useful for save-and-restore), and the previous locale
// here is en_GB because tests/.cfconfig.json sets `runtime.locale: "en-GB"` — a
// key that had no consumer until locale became real request state. Both halves of
// this assertion is load-bearing. Set a known baseline first so the assertion does
// not depend on the configured default, which differs by engine (RustCFML reads
// tests/.cfconfig.json; Lucee uses its own server config).
setLocale("en_GB");
result = setLocale("English (US)");
assert("setLocale returns the PREVIOUS locale", result, "en_GB");
assert("getLocale returns locale", lCase(getLocale()), "english (us)");
suiteEnd();

// --- lsDateFormat ---
suiteBegin("lsDateFormat");
d = createDate(2024, 3, 15);
assert("lsDateFormat default", len(lsDateFormat(d)) > 0, true);
assert("lsDateFormat mm/dd/yyyy", lsDateFormat(d, "mm/dd/yyyy"), "03/15/2024");
assert("lsDateFormat with locale arg", lsDateFormat(d, "mm/dd/yyyy", "English (US)"), "03/15/2024");
suiteEnd();

// --- lsTimeFormat ---
suiteBegin("lsTimeFormat");
dt = createDateTime(2024, 3, 15, 14, 30, 45);
assert("lsTimeFormat HH:mm:ss", lsTimeFormat(dt, "HH:mm:ss"), "14:30:45");
assert("lsTimeFormat with locale", lsTimeFormat(dt, "HH:mm:ss", "English (US)"), "14:30:45");
suiteEnd();

// --- lsDateTimeFormat ---
suiteBegin("lsDateTimeFormat");
dt = createDateTime(2024, 3, 15, 14, 30, 45);
assert("lsDateTimeFormat date part", lsDateTimeFormat(dt, "mm/dd/yyyy"), "03/15/2024");
assert("lsDateTimeFormat time part", lsDateTimeFormat(dt, "HH:nn:ss"), "14:30:45");
assert("lsDateTimeFormat combined", lsDateTimeFormat(dt, "yyyy-mm-dd HH:nn:ss"), "2024-03-15 14:30:45");
suiteEnd();

// --- lsCurrencyFormat ---
suiteBegin("lsCurrencyFormat");
assert("lsCurrencyFormat default (local)", lsCurrencyFormat(1234.56), "$1,234.56");
assert("lsCurrencyFormat local", lsCurrencyFormat(1234.56, "local"), "$1,234.56");
assert("lsCurrencyFormat international", lsCurrencyFormat(1234.56, "international"), "USD 1,234.56");
assert("lsCurrencyFormat none", lsCurrencyFormat(1234.56, "none"), "1,234.56");
assert("lsCurrencyFormat negative", lsCurrencyFormat(-99.99, "local"), "($99.99)");
assert("lsCurrencyFormat zero", lsCurrencyFormat(0), "$0.00");
suiteEnd();

// --- lsEuroCurrencyFormat ---
suiteBegin("lsEuroCurrencyFormat");
assert("lsEuroCurrencyFormat international", lsEuroCurrencyFormat(1234.56, "international"), "USD 1,234.56");
assert("lsEuroCurrencyFormat none", lsEuroCurrencyFormat(1234.56, "none"), "1,234.56");
assert("lsEuroCurrencyFormat negative", lsEuroCurrencyFormat(-50.00, "international"), "USD (50.00)");
suiteEnd();

// --- lsIsDate ---
suiteBegin("lsIsDate");
assertTrue("lsIsDate valid date", lsIsDate("2024-03-15"));
assertTrue("lsIsDate valid date string", lsIsDate("March 15, 2024"));
assertFalse("lsIsDate invalid", lsIsDate("not a date"));
suiteEnd();

// --- lsIsNumeric ---
suiteBegin("lsIsNumeric");
assertTrue("lsIsNumeric integer", lsIsNumeric("42"));
assertTrue("lsIsNumeric float", lsIsNumeric("3.14"));
assertTrue("lsIsNumeric negative", lsIsNumeric("-100"));
assertFalse("lsIsNumeric string", lsIsNumeric("hello"));
suiteEnd();

// --- lsIsCurrency ---
suiteBegin("lsIsCurrency");
assertTrue("lsIsCurrency dollar", lsIsCurrency("$1,234.56"));
assertTrue("lsIsCurrency plain number", lsIsCurrency("1234.56"));
assertTrue("lsIsCurrency negative", lsIsCurrency("-$99.99"));
assertFalse("lsIsCurrency non-currency", lsIsCurrency("hello"));
assertFalse("lsIsCurrency empty", lsIsCurrency(""));
suiteEnd();

// --- lsParseCurrency ---
suiteBegin("lsParseCurrency");
assert("lsParseCurrency dollar", lsParseCurrency("$1,234.56"), 1234.56);
assert("lsParseCurrency USD prefix", lsParseCurrency("USD1,234.56"), 1234.56);
assert("lsParseCurrency plain", lsParseCurrency("99.99"), 99.99);
suiteEnd();

// --- lsParseDateTime ---
suiteBegin("lsParseDateTime");
result = lsParseDateTime("2024-03-15");
assert("lsParseDateTime ISO date", dateFormat(result, "yyyy-mm-dd"), "2024-03-15");
suiteEnd();

// --- lsNumberFormat ---
suiteBegin("lsNumberFormat");
assert("lsNumberFormat no mask", lsNumberFormat(1234), "1,234");
assert("lsNumberFormat with mask", lsNumberFormat(1234.567, "9,999.99"), "1,234.57");
suiteEnd();

// --- lsWeek ---
suiteBegin("lsWeek");
// 2024-01-01 is ISO week 1
w = lsWeek("2024-01-01");
assertTrue("lsWeek returns positive", w > 0);
assertTrue("lsWeek in range", w <= 53);
suiteEnd();

// --- lsDayOfWeek ---
suiteBegin("lsDayOfWeek");
// 2024-03-15 is a Friday
dow = lsDayOfWeek("2024-03-15");
assert("lsDayOfWeek Friday", dow, 6); // CFML: 1=Sun, 6=Fri
// 2024-03-17 is a Sunday
assert("lsDayOfWeek Sunday", lsDayOfWeek("2024-03-17"), 1);
suiteEnd();
</cfscript>
