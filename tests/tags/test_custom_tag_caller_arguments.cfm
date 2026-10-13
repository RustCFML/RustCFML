<cfscript>
// A custom tag called from inside a function can read that function's
// arguments scope with an explicit `arguments.x`, but the scope is not part of
// the tag's bare-name chain (measured on Lucee 7.0.4 and 5.3.10):
//
//   - an unscoped read of an argument name does not find it;
//   - an unscoped write lands in the tag's `variables`, even when the calling
//     function has an argument of that name, and leaves the argument alone;
//   - an unscoped member write (`stObj.x = v`) creates a struct in the tag's
//     `variables` instead of mutating the caller's argument;
//   - a template the tag includes follows the same rules.
//
// RustCFML resolved those names through the caller's arguments instead: a
// write such as `ObjectID = attributes.ObjectID` went into that scope
// and was gone from `variables`, and `stObj.x = v` changed the struct the
// caller passed in. FarCry's formtools object tag does exactly that write when
// rendered from a component method that has an `objectid` argument, so its
// later `variables.ObjectID` read failed and every webtop grid page with it.
//
// Fixtures: tag_callerargs_host.cfc (the calling functions),
// tag_callerargs_probe.cfm (the tag), tag_callerargs_inc.cfm (its include).

suiteBegin("Custom tag called from a function: the caller's arguments are explicit-only");

host = createObject("component", "tag_callerargs_host");

// ── A: cfmodule from a function with declared arguments ──
r = host.declared();
assert("A: an unscoped read does not find the caller's argument", r.readBefore, "THREW");
assert("A: arguments.objectid is the caller's", r.argBefore, "fnOID");
assertTrue("A: an unscoped write lands in the tag's variables", r.varsHasObjectid);
assert("A: and reads back unscoped", r.readAfter, "fromTag");
assert("A: typename too", r.varsTypename, "typeFromTag");
assert("A: arguments.objectid is unchanged inside the tag", r.argAfter, "fnOID");
assertTrue("A: an unscoped member write creates variables.stObj", r.varsHasStObj);
assertFalse("A: and leaves the caller's struct argument alone", r.fnStObjTouched);
assert("A: the function's own objectid argument is unchanged", r.fnObjectid, "fnOID");
assert("A: the function's own typename argument is unchanged", r.fnTypename, "fnType");
assert("A: an include in the tag reads the tag's variables", r.incReadTypename, "typeFromTag");
assert("A: and writes into them", r.varsFromInclude, "inc");

// ── B: cfmodule from a function whose arguments are passed but not declared ──
r = host.undeclared(objectid = "undeclOID", typename = "undeclType", stObj = {});
assert("B: an unscoped read does not find the caller's argument", r.readBefore, "THREW");
assert("B: arguments.objectid is the caller's", r.argBefore, "undeclOID");
assertTrue("B: an unscoped write lands in the tag's variables", r.varsHasObjectid);
assert("B: arguments.objectid is unchanged inside the tag", r.argAfter, "undeclOID");
assertFalse("B: the caller's struct argument is untouched", r.fnStObjTouched);
assert("B: the function's own objectid argument is unchanged", r.fnObjectid, "undeclOID");

// ── C: the same tag through a cfimport prefix ──
r = host.viaImport();
assert("C: an unscoped read does not find the caller's argument", r.readBefore, "THREW");
assertTrue("C: an unscoped write lands in the tag's variables", r.varsHasObjectid);
assert("C: arguments.objectid is unchanged inside the tag", r.argAfter, "fnOID");
assertFalse("C: the caller's struct argument is untouched", r.fnStObjTouched);

structDelete(request, "tagCallerArgs");

suiteEnd();
</cfscript>
