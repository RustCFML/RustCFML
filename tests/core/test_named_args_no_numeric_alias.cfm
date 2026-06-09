<cfscript>
suiteBegin("Core: named-arg call to a declared-param fn leaks no numeric alias key");

// A named-argument call to a function that DECLARES parameters must populate
// the arguments scope by NAME only. RustCFML 0.92.0 leaks a spurious numeric
// positional-alias key into the arguments scope when a named argument lands in
// a declared positional slot:
//
//   function cf(any position="last", any overwrite="false"){ return StructKeyList(arguments); }
//   cf(body="X", overwrite=true)
//     RustCFML 0.92.0 -> "overwrite,2,body,position"   (spurious "2")
//     Lucee 7         -> "position,overwrite,body"      (no numeric key)
//
// The bug poisons any code that iterates / counts / serializes the arguments
// scope after such a call (StructCount is inflated, for-in sees a bogus "2",
// argument-forwarding round-trips a junk key). Wheels' option-forwarding
// helpers do exactly this all over the framework.
//
// Assertions key on PRESENCE / ABSENCE, never on order: Lucee uppercases keys
// and reorders the scope, so any order-sensitive assert would be a false
// failure on a conforming engine.

// --- helper that reports the shape of its own arguments scope by name only ---
function probeArgs(any position = "last", any overwrite = "false") {
    var result = {
        hasNumericKey = false,
        has2          = structKeyExists(arguments, "2"),
        hasBody       = structKeyExists(arguments, "body"),
        hasOverwrite  = structKeyExists(arguments, "overwrite"),
        hasPosition   = structKeyExists(arguments, "position")
    };
    for (var k in arguments) {
        if (isNumeric(k)) {
            result.hasNumericKey = true;
        }
    }
    return result;
}

shape = probeArgs(body = "X", overwrite = true);

// No purely-numeric key may exist in the arguments scope.
assertFalse("arguments scope has NO purely-numeric key", shape.hasNumericKey);
assertFalse("arguments scope has no key named '2'", shape.has2);

// The named arguments are present under their own names.
assertTrue("named arg 'overwrite' present by name", shape.hasOverwrite);
assertTrue("extra named arg 'body' present by name", shape.hasBody);
// 'position' was declared but not passed -> present via its declared default.
assertTrue("declared 'position' present (default applied)", shape.hasPosition);

// Cross-check via StructKeyList: still no numeric token in the key list.
function keyListProbe(any position = "last", any overwrite = "false") {
    return structKeyList(arguments);
}
keys = keyListProbe(body = "X", overwrite = true);
assertFalse("StructKeyList contains no '2' token",
    listFindNoCase(keys, "2") gt 0);
assertTrue("StructKeyList contains 'body'",
    listFindNoCase(keys, "body") gt 0);
assertTrue("StructKeyList contains 'overwrite'",
    listFindNoCase(keys, "overwrite") gt 0);
assertTrue("StructKeyList contains 'position'",
    listFindNoCase(keys, "position") gt 0);

// --- CONTROL: paramless fn called with all named args (already correct on
//     both engines). Guards the wiring so a regression in the named-arg path
//     can't masquerade as the gap under test. ---
function paramless() {
    return structCount(arguments);
}
assert("CONTROL: paramless fn, two named args -> StructCount == 2",
    paramless(alpha = 1, sort = 2), 2);

suiteEnd();
</cfscript>
