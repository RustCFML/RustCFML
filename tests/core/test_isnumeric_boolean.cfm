<cfscript>
suiteBegin("IsNumeric: boolean is not numeric");

// Background: A CFML boolean is NOT a numeric value. On Lucee, Adobe CF, and
// BoxLang, IsNumeric(true) and IsNumeric(false) both return false — only an
// actual number (or a numeric string) is numeric. RustCFML 0.153.0 reports a
// boolean as numeric (IsNumeric(true)=true, IsNumeric(false)=true), which
// silently flips any "is this argument a number?" guard that defaults to a
// boolean. Wheels relies on exactly such a guard: the finder parameterize
// flag (default boolean `true`) is checked with IsNumeric(arguments.parameterize)
// in vendor/wheels/model/sql.cfc, so the divergence breaks multi-condition and
// subquery finders on RustCFML.

// --- Boolean literals are not numeric ---
assertFalse("isNumeric(true) is false", isNumeric(true));
assertFalse("isNumeric(false) is false", isNumeric(false));

// --- Boolean-typed variable is not numeric ---
isnumboolB = true;
assertFalse("isNumeric(boolean var) is false", isNumeric(isnumboolB));

// --- Controls: agree on both engines ---
assertTrue("isNumeric(1) is true", isNumeric(1));
assertTrue("isNumeric('123') is true", isNumeric("123"));
assertFalse("isNumeric('abc') is false", isNumeric("abc"));

suiteEnd();
</cfscript>
