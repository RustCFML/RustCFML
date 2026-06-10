<cfscript>
suiteBegin("Struct literal: null as a bare key name");

// Lucee/ACF treat a bare `null` (and other keyword literals) directly before
// a key separator in a struct literal as the KEY NAME "null", not as the
// null value literal. cfqueryparam relies on this: its attribute struct is
// `{value: ..., cfsqltype: ..., null: true}`, so losing the key silently
// drops null= handling.

// --- gap: bare `null` key with colon separator ---
s = { value: "", cfsqltype: "varchar", null: true };
assert("bare null key: struct has 3 keys", structCount(s), 3);
assertTrue("bare null key exists", structKeyExists(s, "null"));
assertTrue("bare null key value is true", (s["null"] ?: false) eq true);

// --- gap: bare `null` key with equals separator (cfqueryparam attribute style) ---
s2 = { value = "", null = true };
assertTrue("bare null key (equals separator) exists", structKeyExists(s2, "null"));
assertTrue("bare null key (equals separator) value", (s2["null"] ?: false) eq true);

// --- control: quoted "null" key works today ---
s3 = { value: "", "null": true };
assertTrue("quoted null key exists (control)", structKeyExists(s3, "null"));

// --- control: a struct literal without the keyword key is unaffected ---
s4 = { value: "x", cfsqltype: "varchar" };
assert("plain literal key count (control)", structCount(s4), 2);

suiteEnd();
</cfscript>
