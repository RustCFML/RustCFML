<cfscript>
// A java.util.Date shim instance must be usable AS A DATE by the CFML date
// BIFs — on Lucee (real JVM) isDate() is true and dateAdd/dateDiff/
// dateCompare/dateTimeFormat all accept it. On RustCFML the shim exists and
// its methods work (getTime() is fine), but every date BIF rejects it:
//
//   dateAdd:     Invalid date: java.util.date
//   dateCompare: Invalid date1
//
// Repro class: this sits directly on the vendored jwt-cfml surface the
// v0.606.0 Signature/KeyFactory shims enabled. jwt-cfml anchors epoch math
// on `createObject('java','java.util.Date').init(javacast('int', 0))` and
// converts exp/nbf claims via dateAdd/dateDiff against it — so the moment
// canVerifyAsymmetric() went true, titan's Auth0 decode moved off its
// fallback and died here instead: the shim unlocked a path the Date shim
// then broke. (titan worked around it by rewriting the converters in pure
// CFML; this pins the engine-side contract.)
//
// Assertions are timezone-agnostic (relative arithmetic, not absolute
// formatting), measured on Lucee 7.0 (docker lucee/lucee:7.0).

suiteBegin("java.util.Date shim instances work with the CFML date BIFs");

epochBase = createObject("java", "java.util.Date").init(javacast("long", 0));

// The shim itself is present and its methods work — that part is fine today.
assert( "shim method getTime() works (control)", epochBase.getTime(), 0 );

// isDate: a Date IS a date.
assertTrue( "isDate() is true for a java.util.Date instance", isDate(epochBase) );

// dateAdd accepts it and yields a real date.
r1 = "(threw)";
try { r1 = isDate(dateAdd("s", 60, epochBase)); } catch (any e) { r1 = "THREW: " & e.message; }
assert( "dateAdd() accepts a java.util.Date base", r1, true );

// Relative round trip: +2 minutes is 120 seconds, in any timezone.
r2 = "(threw)";
try { r2 = dateDiff("s", epochBase, dateAdd("n", 2, epochBase)); } catch (any e) { r2 = "THREW: " & e.message; }
assert( "dateDiff() round-trips against a java.util.Date base", r2, 120 );

// The epoch-conversion shape jwt-cfml uses: seconds offset -> whole days.
r3 = "(threw)";
try { r3 = dateDiff("d", epochBase, dateAdd("s", 86400, epochBase)); } catch (any e) { r3 = "THREW: " & e.message; }
assert( "epoch-seconds conversion shape (dateAdd s / dateDiff d)", r3, 1 );

// dateCompare: 1970 is before now.
r4 = "(threw)";
try { r4 = dateCompare(epochBase, now()); } catch (any e) { r4 = "THREW: " & e.message; }
assert( "dateCompare() accepts a java.util.Date instance", r4, -1 );

// dateTimeFormat accepts it (year is 1970 UTC / 1969 in negative-offset zones).
r5 = "(threw)";
try { r5 = dateTimeFormat(epochBase, "yyyy"); } catch (any e) { r5 = "THREW: " & e.message; }
assertTrue( "dateTimeFormat() accepts a java.util.Date instance (saw: " & r5 & ")",
    listFind("1969,1970", r5) GT 0 );

suiteEnd();
</cfscript>
