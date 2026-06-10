<cfscript>
suiteBegin("Core: struct keys are case-insensitive on WRITE — cross-case writes update in place");

// CFML structs are case-insensitive containers: a write through ANY casing of
// an existing key must update that key IN PLACE. RustCFML 0.103.0 falls back
// across case on READS (s.zip finds a key created as s["ZIP"]), but a
// differently-cased WRITE forks the key — the same logical key ends up stored
// twice with two independent values:
//
//   s = {a: 1}; s["A"] = 99;
//     RustCFML 0.103.0 -> structCount=2, keys "a,A", s.a=1, s["A"]=99
//     Lucee 5.4 / 7    -> structCount=1, every casing reads 99
//
// Target behavior (maintainer-stated: case preservation by default, Lucee
// parity always): ONE key — the first-written casing wins the key list, and
// later cross-case writes update the value without re-casing the stored key.
//
// The fork poisons any set-one-case / read-another-case flow: url/form params,
// query columnList lookups, option/config struct merging — Wheels does all of
// these constantly, so a forked key silently reads back a stale value.
//
// Note on key-CASING asserts: Lucee 5.x UPPERCASES unquoted struct-literal /
// dot-notation keys ({count: 1} stores COUNT) while bracket-created keys
// preserve their casing on every Lucee line. Exact-casing asserts below are
// therefore limited to bracket-created keys; literal-created keys are only
// asserted case-insensitively (CFML == is case-insensitive for strings).

// --- cross-case BRACKET write onto a literal-created key (the fork) ---
forkA = {a: 1};
forkA["A"] = 99;
assert("cross-case bracket write keeps ONE key (StructCount)",
    structCount(forkA), 1);
assert("StructKeyList holds exactly one element after cross-case write",
    listLen(structKeyList(forkA)), 1);
assert("the single surviving key is 'a' (case-insensitive compare)",
    structKeyList(forkA), "a");
assert("dot read returns the cross-case-written value", forkA.a, 99);
assert("bracket read (lower) returns the cross-case-written value", forkA["a"], 99);
assert("bracket read (upper) returns the cross-case-written value", forkA["A"], 99);

// --- cross-case DOT write onto a bracket-created key ---
forkB = structNew();
forkB["zip"] = "30303";
forkB.ZIP = "90210";
assert("cross-case dot write keeps ONE key (StructCount)",
    structCount(forkB), 1);
assert("StructKeyList after cross-case dot write is just 'zip'",
    structKeyList(forkB), "zip");
assertTrue("first-written casing preserved EXACTLY in StructKeyList",
    compare(structKeyList(forkB), "zip") == 0);
assert("dot read (lower) sees the dot-written value", forkB.zip, "90210");
assert("bracket read (first-written casing) sees the dot-written value",
    forkB["zip"], "90210");
assert("bracket read (write casing) sees the dot-written value",
    forkB["ZIP"], "90210");

// --- cross-case DOT write onto a literal-created key ---
forkC = {a: 1};
forkC.A = 99;
assert("cross-case dot write onto literal key keeps ONE key",
    structCount(forkC), 1);
assert("dot read (original casing) returns the updated value", forkC.a, 99);

// --- delete coherence after a cross-case write ---
// On a conforming engine the cross-case write left ONE key, so deleting it
// (under the original casing) empties the struct. A forked key survives.
forkD = {name: "alpha"};
forkD["NAME"] = "beta";
structDelete(forkD, "name");
assert("after cross-case write + structDelete, struct is EMPTY",
    structCount(forkD), 0);

// --- CONTROLS (already correct on both engines) ---
// Cross-case READ fallback works; guards against a fix that breaks lookups.
ctrlRead = {zip: "30303"};
assert("CONTROL: cross-case dot read falls back", ctrlRead.ZIP, "30303");
assert("CONTROL: cross-case bracket read falls back", ctrlRead["ZIP"], "30303");
assertTrue("CONTROL: structKeyExists is case-insensitive",
    structKeyExists(ctrlRead, "ZIP"));

// Same-case writes round-trip through one key.
ctrlWrite = {count: 1};
ctrlWrite.count = 2;
ctrlWrite["count"] = 3;
assert("CONTROL: same-case writes keep one key", structCount(ctrlWrite), 1);
assert("CONTROL: same-case write round-trips", ctrlWrite.count, 3);

// structDelete is case-insensitive even without a prior cross-case write.
ctrlDelete = {foo: 1, bar: 2};
structDelete(ctrlDelete, "FOO");
assert("CONTROL: structDelete is case-insensitive", structCount(ctrlDelete), 1);
assertFalse("CONTROL: deleted key gone under its original casing",
    structKeyExists(ctrlDelete, "foo"));

suiteEnd();
</cfscript>
