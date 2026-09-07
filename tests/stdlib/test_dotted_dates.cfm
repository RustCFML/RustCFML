<!---
  GitHub #411: `function f( numeric v )` called with "5.3.2" threw on RustCFML
  and was accepted by Lucee.

  The issue's premise — that Lucee's `numeric` tolerates version-style strings —
  is wrong, and reproducing it as a version rule would have been wrong too.
  Lucee's numeric cast FALLS BACK TO A DATE CAST, and "5.3.2" is 3 May 2002. The
  acceptance set therefore tracks date validity exactly: "2.29.2004" is accepted
  and "2.29.2005" is not, "1.2.0" is accepted and "1.0.0" is not.

  So the real gap was that we did not parse dot-separated dates at all. Both
  halves are asserted here: the date parsing, and the numeric type check that
  rides on it. Every expectation below was measured against Lucee 7.1.0+204.
--->
<cfscript>
suiteBegin("Dot-separated dates, and the numeric type check that rides on them (GitHub 411)");

function fmt(required string s) {
    try { return dateTimeFormat(parseDateTime(arguments.s), "yyyy-mm-dd HH:nn:ss"); }
    catch (any e) { return "ERR"; }
}

// --- Which component is the day, month and year is decided positionally. ---

// A is a full year -> Y.M.D
assert("2020.11.5 is Y.M.D", fmt("2020.11.5"), "2020-11-05 00:00:00");
// A cannot be a month and C is a full year -> D.M.Y
assert("31.12.2020 is D.M.Y", fmt("31.12.2020"), "2020-12-31 00:00:00");
// A cannot be a month and C is not a year -> Y.M.D
assert("13.1.2 is Y.M.D", fmt("13.1.2"), "2013-01-02 00:00:00");
assert("45.1.2 is Y.M.D", fmt("45.1.2"), "1945-01-02 00:00:00");
// A COULD be a month -> M.D.Y, and that reading is the only one tried
assert("5.3.2 is M.D.Y", fmt("5.3.2"), "2002-05-03 00:00:00");
assert("1.2.2020 is M.D.Y", fmt("1.2.2020"), "2020-01-02 00:00:00");
assert("12.12.12 is M.D.Y", fmt("12.12.12"), "2012-12-12 00:00:00");

// The subtle one: "0.1.2" parses fine as Y.M.D (2000-01-02) and Lucee STILL
// rejects it, because a component that could be a month commits the string to
// M.D.Y, pass or fail. Month 0 then fails. Accepting this would mean accepting
// a `numeric` argument Lucee rejects.
assert("0.1.2 is rejected, not re-read as Y.M.D", fmt("0.1.2"), "ERR");

// --- Two-digit years use a FIXED 1930-2029 window, not a sliding one. ---
assert("29 is 2029", fmt("1.1.29"), "2029-01-01 00:00:00");
assert("30 is 1930", fmt("1.1.30"), "1930-01-01 00:00:00");
assert("00 is 2000", fmt("1.1.00"), "2000-01-01 00:00:00");
assert("99 is 1999", fmt("1.1.99"), "1999-01-01 00:00:00");
// Three or more digits is a literal year, not a two-digit one.
assert("026 is a literal 26 -> 2026", fmt("1.1.026"), "2026-01-01 00:00:00");

// --- Real date validity, including leap years. ---
assert("2.29.2004 is a leap day", fmt("2.29.2004"), "2004-02-29 00:00:00");
assert("2.29.2005 is not", fmt("2.29.2005"), "ERR");
assert("day 32 is never valid", fmt("1.32.0"), "ERR");
assert("month 0 is never valid", fmt("0.0.0"), "ERR");

// --- An optional trailing time. ---
assert("dotted date with time", fmt("5.3.2 10:30:00"), "2002-05-03 10:30:00");
assert("dotted date with HH:mm", fmt("1.2.3 10:30"), "2003-01-02 10:30:00");

// --- Non-dates stay non-dates. Four components, signs, empty components and
//     letters must not become dates just because dots are now meaningful.
assert("four components", fmt("5.3.2.1"), "ERR");
assert("negative component", fmt("-1.2.3"), "ERR");
assert("empty component", fmt("5..3"), "ERR");
assert("trailing dot", fmt("5.3."), "ERR");
assert("leading dot", fmt(".5.3"), "ERR");
assert("letters", fmt("a.b.c"), "ERR");
assert("version suffix", fmt("5.3.2-RC1"), "ERR");

// NOTE - an OPEN divergence, deliberately not asserted here because the two
// engines disagree and it is not what GH #411 reported. Lucee also parses the
// TWO-component dotted form as M.D with the current year implied:
//
//   Lucee 7.1.0:  isDate("5.3") true  -> 2026-05-03
//                 isDate("1.5") true  -> 2026-01-05
//   RustCFML:     isDate("5.3") false
//
// This predates the three-component work above and is unchanged by it. It is a
// wider behavioural change than the reported case (two-component decimals are
// far more common in real data than version triples), so it is left for a
// separate decision rather than folded in here.

// isNumeric() is NOT affected — it is false for "5.3.2" on BOTH engines. Only
// the `numeric` TYPE CHECK (and isValid) take the date fallback.
assert("isNumeric is unmoved", isNumeric("5.3.2"), false);
assert("isValid numeric takes the date fallback", isValid("numeric", "5.3.2"), true);
assert("isValid numeric rejects a non-date", isValid("numeric", "2.30.5"), false);

// --- The reported repro: a numeric-typed parameter. ---
function versionParam( numeric luceeVersion = "5.3.2" ) {
    return "v=[" & arguments.luceeVersion & "]";
}
assert("numeric param accepts a defaulted 5.3.2", versionParam(), "v=[5.3.2]");
assert("numeric param accepts an explicit 5.3.2", versionParam(luceeVersion="5.3.2"), "v=[5.3.2]");

// Validation, never coercion — the value arrives exactly as passed.
function echoNum( numeric n ) { return arguments.n; }
assert("the value is not coerced to a date serial", echoNum("5.3.2"), "5.3.2");

// A genuinely non-numeric, non-date string is still rejected.
threwAbc = false;
try { echoNum("abc"); } catch (any e) { threwAbc = true; }
assert("numeric still rejects abc", threwAbc, true);
threwFeb30 = false;
try { echoNum("2.30.5"); } catch (any e) { threwFeb30 = true; }
assert("numeric rejects an invalid date", threwFeb30, true);

suiteEnd();
</cfscript>
