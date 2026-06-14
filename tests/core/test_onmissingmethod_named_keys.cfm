<cfscript>
suiteBegin("OOP: onMissingMethod preserves named-argument NAMES in missingMethodArguments");

// ============================================================
// Background
// ============================================================
// When a method call falls through to onMissingMethod, the engine hands the
// call's arguments to the handler as `missingMethodArguments`. The KEYS of
// that struct depend on HOW the method was called:
//
//   * NAMED call    obj.probe(label="world")  -> keyed by NAME   ("label")
//   * POSITIONAL    obj.probe("world")        -> keyed by POSITION ("1")
//
// Lucee 5/6/7 and Adobe ColdFusion honor this for both shapes. RustCFML
// 0.153.0 keys the NAMED-call struct by numeric position as well, dropping
// the argument names entirely:
//
//   component {
//     function onMissingMethod(missingMethodName, missingMethodArguments){
//       return structKeyList(missingMethodArguments);
//     }
//   }
//   obj.probe(label = "world")
//     Lucee 5.4.8.2    -> "label"
//     RustCFML 0.153.0 -> "1"          (name lost; structKeyExists(...,"label") false)
//
//   obj.probe(label = "a", extra = "b")
//     Lucee 5.4.8.2    -> "label,extra"
//     RustCFML 0.153.0 -> "1,2"
//
// The POSITIONAL call is correct on BOTH engines (numeric keys), so it serves
// as the CONTROL that guards the wiring.
//
// SIBLINGS (no assertion overlap): this is NOT
//   - #126 cfinvoke argumentCollection positional binding,
//   - #95  invoke() dropping undeclared argument-struct keys, nor
//   - #82  named-arg numeric-alias leak on a DECLARED-param call.
// Those concern the invoke()/cfinvoke marshaling path and declared-param
// frames. THIS gap is specifically onMissingMethod failing to preserve the
// NAMES of named arguments in the missingMethodArguments struct.
//
// WHEELS IMPACT: Wheels' dynamic query scopes — the CLAUDE.md pattern
//   scope(name="byStatus", handler="scopeByStatus")
//   model("Post").byStatus(status="published")
// dispatch the named call through onMissingMethod; the handler reads the
// option BY NAME (arguments.status). On RustCFML the struct arrives as
// {1="published"}, so arguments.status is undefined and the scope's WHERE
// clause is empty -> the query returns the wrong rows (0, or all). Surfaced
// laddering named query scopes.
//
// Assertions key on PRESENCE / VALUE, never on key ORDER: Lucee uppercases
// and reorders struct keys, so any order-sensitive assert would false-fail on
// a conforming engine.
// ============================================================

ommnk_fixture = createObject("component", "OnMissingMethodNamedKeysFixture");

// ---- (1) single NAMED arg: the NAME key must be present ----
ommnk_one = ommnk_fixture.probe(label = "world");

assertTrue("single named arg: 'label' key present in missingMethodArguments",
	ommnk_one.hasLabel);
assert("single named arg: missingMethodArguments.label == 'world'",
	ommnk_one.labelVal, "world");
assertTrue("single named arg: StructKeyList contains 'label'",
	listFindNoCase(ommnk_one.keyList, "label") gt 0);
// The name must not have been replaced by a numeric position key.
assertFalse("single named arg: no numeric key '1' substituted for the name",
	ommnk_one.has1);

// ---- (2) two NAMED args: BOTH names must be present ----
ommnk_two = ommnk_fixture.probe(label = "a", extra = "b");

assertTrue("two named args: 'label' present", ommnk_two.hasLabel);
assertTrue("two named args: 'extra' present", ommnk_two.hasExtra);
assert("two named args: label value", ommnk_two.labelVal, "a");
assert("two named args: extra value", ommnk_two.extraVal, "b");
assert("two named args: StructCount == 2", ommnk_two.count, 2);
assertTrue("two named args: StructKeyList contains 'label'",
	listFindNoCase(ommnk_two.keyList, "label") gt 0);
assertTrue("two named args: StructKeyList contains 'extra'",
	listFindNoCase(ommnk_two.keyList, "extra") gt 0);

// ---- (3) WHEELS-shaped dynamic scope: named 'status' read BY NAME ----
// The on-disk Wheels pattern: model("Post").byStatus(status="published").
ommnk_scope = ommnk_fixture.byStatus(status = "published");

assertTrue("dynamic scope: 'status' present by name in missingMethodArguments",
	ommnk_scope.hasStatus);
assert("dynamic scope: missingMethodArguments.status == 'published'",
	ommnk_scope.statusVal, "published");

// ---- CONTROL: POSITIONAL call uses numeric keys on BOTH engines ----
// Guards the wiring so a regression in the named-arg path can't masquerade as
// the gap under test.
ommnk_pos = ommnk_fixture.probe("world");

assertTrue("CONTROL: positional call -> numeric key '1' present",
	ommnk_pos.has1);
assert("CONTROL: positional call -> missingMethodArguments['1'] == 'world'",
	ommnk_pos.val1, "world");
// A positional call carries no name, so 'label' must be ABSENT on both.
assertFalse("CONTROL: positional call has no 'label' key",
	ommnk_pos.hasLabel);

suiteEnd();
</cfscript>
