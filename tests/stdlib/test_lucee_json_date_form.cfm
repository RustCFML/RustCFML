<cfscript>
suiteBegin("Dates: Lucee's serializeJSON date form round-trips through the date BIFs");

// Lucee serialises a CFML date as "Month, dd yyyy HH:mm:ss +0000" — e.g.
//   serializeJSON({d: createDateTime(2026,8,25,9,0,14)})
//   -> {"D":"August, 25 2026 09:00:14 +0000"}           (Lucee 7.0.5, measured)
// and its date parser accepts that string back: isDate() is true, and
// dateDiff / parseDateTime / dateFormat all treat it as the date it names.
//
// RustCFML rejects the string: isDate() is false and every date BIF given it
// throws "Invalid date2" (or the equivalent for its argument position). Its
// own serializeJSON emits "2026-08-25 09:00:14", which BOTH engines parse, so
// data written by RustCFML is fine everywhere — the gap is one-directional:
// any JSON/jsonb column written under Lucee that holds a raw date is
// unreadable after switching engines.
//
// Repro class: titan's zoho_oauth.tokens jsonb row, written under Lucee with
// expires_when = dateAdd('s', expires_in, now()). On RustCFML the token
// expiry check ran dateDiff('s', now(), tokens.expires_when) and every Zoho
// integration call died with {"success":false,"message":"Invalid date2"} —
// before the refresh that would have rewritten the row could run.
//
// Every leg is under try/catch; a throw is asserted as a value.

luceeForm    = "August, 25 2026 09:00:14 +0000";  // measured serializeJSON output
luceeFormNoZ = "August, 25 2026 09:00:14";         // same, sans offset (older Lucee)
isoForm      = "2026-08-25 09:00:14";              // what RustCFML's serializeJSON writes; both parse

function tryDiff(required string a, required string b) {
    try { return dateDiff("s", arguments.a, arguments.b); }
    catch (any e) { return "THREW: " & (e.message ?: ""); }
}
function tryFormat(required string d) {
    try { return dateFormat(parseDateTime(arguments.d), "yyyy-mm-dd") & " " & timeFormat(parseDateTime(arguments.d), "HH:mm:ss"); }
    catch (any e) { return "THREW: " & (e.message ?: ""); }
}

// Control: the ISO-style form is a date on both engines.
assertTrue("control: isDate on 'yyyy-mm-dd HH:mm:ss'", isDate(isoForm));
assert("control: dateDiff between two ISO-style strings", tryDiff("2026-08-25 09:00:00", isoForm), 14);

// The gap: Lucee's own JSON date form.
assertTrue("isDate accepts Lucee's serializeJSON date form (with +0000)", isDate(luceeForm));
assertTrue("isDate accepts Lucee's serializeJSON date form (no offset)", isDate(luceeFormNoZ));
assert("dateDiff with the Lucee form as date2", tryDiff("2026-08-25 09:00:00", luceeForm), 14);
assert("dateDiff with the Lucee form as date1", tryDiff(luceeForm, "2026-08-25 09:01:14"), 60);
assert("parseDateTime + dateFormat/timeFormat of the Lucee form names the same instant", tryFormat(luceeFormNoZ), "2026-08-25 09:00:14");

suiteEnd();
</cfscript>
