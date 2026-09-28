<cfscript>
// An exception thrown by an included template must reach the caller's catch
// with its own type, detail, errorcode and extendedinfo. The include opcodes
// rebuilt a bare { message, type, detail: "" } struct from the Rust error, so
// every custom exception arrived as `Runtime` and a typed catch never matched
// (GH #431: Wheels maps its ViewNotFound type to a 404; it became a 500).
suiteBegin("include preserves the thrown exception");

function includeThrowFields(e) {
    return arrayToList([e.type, e.message, e.detail, e.errorcode, e.extendedinfo], "|");
}
expected = "Custom.NotFound|include sentinel|detail sentinel|E431|extended sentinel";

try { include "include_custom_exception_thrower.cfm"; } catch (any e) { got = includeThrowFields(e); }
assert("static include keeps type, detail, errorcode and extendedinfo", got, expected);

incTarget = "include_custom_exception_thrower.cfm";
try { include "#incTarget#"; } catch (any e) { got = includeThrowFields(e); }
assert("dynamic include keeps them too", got, expected);

caughtBy = "";
try { include "include_custom_exception_thrower.cfm"; }
catch (Custom.NotFound e) { caughtBy = "exact"; }
catch (any e) { caughtBy = "any: " & e.type; }
assert("a catch on the exact custom type matches", caughtBy, "exact");

caughtBy = "";
try { include "include_custom_exception_thrower.cfm"; }
catch (Custom e) { caughtBy = "parent"; }
catch (any e) { caughtBy = "any: " & e.type; }
assert("a catch on the parent type matches", caughtBy, "parent");

rethrownType = "";
try {
    try { include "#incTarget#"; } catch (any e) { rethrow; }
} catch (any e) { rethrownType = e.type; }
assert("rethrow keeps the type", rethrownType, "Custom.NotFound");

missingType = "";
try { include "does_not_exist_gh431.cfm"; } catch (any e) { missingType = e.type; }
assert("a missing include is still missingInclude", missingType, "missingInclude");

suiteEnd();
</cfscript>
