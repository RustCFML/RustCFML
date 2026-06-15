<cfscript>
suiteBegin("Core: SerializeJSON must not leak internal arguments-scope sentinel keys");

// Background: RustCFML tags an arguments-derived struct with internal sentinel
// keys (__arguments_scope, __arguments_params) — and structKeyList / structCount
// / structKeyExists / for-in all correctly FILTER them. But SerializeJSON does
// NOT filter them, so a struct built by structAppend(s, arguments) serializes
// the sentinels into the JSON. Lucee has no such keys.
//
//   function f() { var s = {}; structAppend(s, arguments); return serializeJSON(s); }
//   f(from="a", template="welcome")
//     RustCFML 0.161.0 -> {"from":"a","template":"welcome","__arguments_scope":true,"__arguments_params":[]}
//     Lucee 5.4.8.2    -> {"template":"welcome","from":"a"}
//
// Why it matters for Wheels: controller sendEmail() / many helpers build option
// structs by copying the arguments scope, then serialize/spool them — the
// leaked sentinels corrupt the output. The fix is the same filtering
// structKeyList/Count/Exists/for-in already apply, extended to SerializeJSON.

function sjasBuild() {
    var s = {};
    structAppend(s, arguments);
    return serializeJSON(s);
}
sjasJson = sjasBuild(from = "a", template = "welcome");

assertFalse("serializeJSON of an arguments-derived struct omits __arguments_scope",
    findNoCase("__arguments_scope", sjasJson) gt 0);
assertFalse("serializeJSON of an arguments-derived struct omits __arguments_params",
    findNoCase("__arguments_params", sjasJson) gt 0);
assertTrue("the real keys are present", findNoCase("welcome", sjasJson) gt 0 && findNoCase("from", sjasJson) gt 0);

// CONTROL (green on both): the struct funcs already filter the sentinels.
function sjasKeys() { var s = {}; structAppend(s, arguments); return structKeyList(s); }
sjasKl = sjasKeys(from = "a", template = "welcome");
assertFalse("CONTROL: structKeyList already filters __arguments_scope", listFindNoCase(sjasKl, "__arguments_scope") gt 0);

suiteEnd();
</cfscript>
