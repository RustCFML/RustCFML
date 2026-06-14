<cfscript>
suiteBegin("Core: auto-vivify a nested write on a THIS-scope member (residual of ##111)");

// ============================================================
// Background  (follow-on to test_scoped_nested_autoviv.cfm / PR #111)
// ============================================================
// On Lucee 5/6/7, Adobe ColdFusion 2018-2025, and BoxLang, assigning to a
// member path of an undeclared THIS-scope key auto-creates the container as
// a struct, exactly as for any other scope:
//
//     this.paths.migrate = "X";   // this.paths never initialized
//
// The engine implicitly performs `this.paths = {}` on the first nested write.
//
// PR #111 (merged v0.136) fixed this contract for variables / local / request
// (test_scoped_nested_autoviv.cfm) and those STILL pass on 0.153.0. But that
// fix MISSED the `this` scope. On RustCFML 0.153.0 a nested write to an
// undeclared `this.X` is LOST SILENTLY: no throw at the write, the scope key
// may register (StructKeyExists(this,"X") -> true) but it is bound to a
// non-struct value -- IsStruct(this.X) -> false, the written member vanishes,
// and reading it back yields "" instead of the value.
//
// Wheels hits this on the FIRST real exercise of the migrator. vendor/wheels/
// Migrator.cfc init() does, with no prior `this.paths = {}`:
//
//     this.paths.migrate    = expandPath(arguments.migratePath);
//     this.paths.sql        = ...;
//     this.paths.templates  = ...;
//
// On RustCFML every one of those writes evaporates, so this.paths is not a
// struct and carries none of the path keys. getAvailableMigrations() and every
// other migrator entry point that reads this.paths.* then fails -- the entire
// migrator is unusable. This is the first place the THIS-scope gap surfaces in
// a real Wheels workload.
//
// All assertions below PASS on Lucee/ACF/BoxLang. The risky write is performed
// inside a CFC method (try/catch wrapped in the fixture) so a silent-loss or
// throwing engine fails its assertions gracefully instead of aborting the run.
// ============================================================

zsnaObj = createObject("component", "ThisScopeAutoVivFixture");

// ------------------------------------------------------------
// (1) THE GAP: this.X.migrate on an undeclared this.X must auto-vivify as a
//     struct carrying the written key (Wheels Migrator init() shape).
//     RustCFML 0.153.0 returns "NOT-A-STRUCT" / "KEY-LOST" here.
// ------------------------------------------------------------
assert("CFC method: this.X.migrate auto-vivifies as a struct (Migrator init() shape)",
    zsnaObj.vivThisNested(), "migrate=[viv-this]");

// ------------------------------------------------------------
// (2) Two-level chain on the THIS scope: this.X.a.b must vivify EVERY level.
// ------------------------------------------------------------
assert("CFC method: deep chain this.X.a.b vivifies every level",
    zsnaObj.vivThisDeep(), "b=[deep-this]");

// ------------------------------------------------------------
// (3) CONTROL A -- PR #111 still holds: variables-scope nested autoviv inside
//     a CFC method works on RustCFML 0.153.0 today. Pins that the residual is
//     SPECIFIC to `this`, not a regression of the shipped fix.
// ------------------------------------------------------------
assert("control: variables.X.k autoviv inside the same fixture works (##111 holds)",
    zsnaObj.vivVariablesControl(), "k=[viv-vars]");

// ------------------------------------------------------------
// (4) CONTROL B -- isolates the missing implicit `this.X = {}`: a
//     PRE-INITIALIZED this container takes the same nested write fine, so the
//     failure is auto-VIVIFICATION, not this-scope writes in general.
//     Passes on RustCFML today.
// ------------------------------------------------------------
assert("control: nested write on a PRE-INITIALIZED this container works",
    zsnaObj.vivThisPreInit(), "k=[pre-this]");

suiteEnd();
</cfscript>
