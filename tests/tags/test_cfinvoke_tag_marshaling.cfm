<cfscript>
suiteBegin("Tags: cfinvoke call form marshals attributeCollection, returnVariable, sibling dispatch");

// ============================================================
// Background  (companion to tags/test_cfinvoke_statement.cfm — the cf-LESS
//              `invoke` STATEMENT form — and to
//              core/test_invoke_undeclared_keys.cfm, whose tag-form asserts
//              cover undeclared-ATTRIBUTE keeping, surfaced by PR 106)
// ============================================================
// Lucee accepts every CFML tag as a parenthesized CFScript CALL — including
// `cfinvoke(component=o, method="m", returnVariable="r")` and the generic
// `attributeCollection` spelling that passes ALL attributes as one struct.
// Three contracts of that call form hold on Lucee 5/6/7:
//
//   (1) cfinvoke(attributeCollection = "#st#") reads its attributes
//       (component, method, returnVariable, nested argumentCollection)
//       from the struct;
//   (2) returnVariable delivers the method's return value into the caller's
//       scope — plain name, dotted `local.rv`, page level and function level;
//   (3) cfinvoke(method = "m") with NO component attribute, written inside a
//       CFC method, dispatches the SIBLING method `m` on the current
//       component.
//
// RustCFML 0.130.0 parses and runs the call form but breaks all three:
//
//   (1) the method attribute is never read out of attributeCollection
//         -> "Method '' not found in component";
//   (2) returnVariable is accepted and silently dropped — the variable is
//       never written, in ANY form (direct attrs, attributeCollection,
//       dotted, page level, function level);
//   (3) the componentless form resolves the METHOD name as a COMPONENT path
//         -> "Component 'sibling' not found".
//
// The cfquery CONTROL below proves the attributeCollection plumbing itself
// already works on both engines (same generic call-form spelling; name +
// dbtype arrive via the struct) — cfinvoke just never reads from it. And the
// cf-less `invoke` STATEMENT form (test_cfinvoke_statement.cfm) already
// delivers returnVariable correctly, so the fix locus is the cfinvoke CALL
// form's attribute marshaling, not the underlying __cfinvoke dispatch.
//
// Wheels rides all three on every request: Global.cfc $invoke() copies its
// arguments into a struct and ends in exactly shapes (1)+(2) —
//
//     arguments.returnVariable = "local.rv";
//     ...
//     cfinvoke(attributeCollection = "#local.args#");
//     if (StructKeyExists(local, "rv")) return local.rv;
//
// — and when the target method already exists on the calling component it
// sets NO component attribute, riding (3). $invoke() backs $simpleLock() /
// $doubleCheckedLock(), controller action dispatch ($callAction ->
// $invoke(method = action) in controller/processing.cfc), filters, layouts,
// model callbacks and validations. This family alone kept a pristine Wheels
// app from booting on stock v0.130.0: onApplicationStart -> $loadRoutes ->
// $simpleLock -> $invoke -> cfinvoke died with "Method '' not found in
// component", 500ing every request before the first action could dispatch.
//
// Assertion style: each gap shape runs inside try/catch and surfaces a
// thrown message as a "THREW: ..." string, so an engine error reads as a
// failed assertion instead of aborting the suite.
// ============================================================

zzcimFx = createObject("component", "tags.CfInvokeMarshalFixture");

// --- CONTROL: positional invoke() BIF dispatches on the fixture ---
// Guards the wiring: if this fails, the fixture itself is broken, not the
// cfinvoke call-form marshaling under test.
assert("CONTROL: invoke() BIF dispatches on the fixture",
	invoke(zzcimFx, "hello", { who: "bif" }), "hello-bif");

// --- CONTROL: the SAME attributeCollection spelling is honored by cfquery ---
// Query-of-query, so no datasource is needed: `name` and `dbtype` must both
// arrive via the struct for the assert to even be reachable.
zzcimSrc = queryNew("id,name", "integer,varchar", [[1, "alpha"]]);
zzcimQAttrs = { name: "zzcimOut", dbtype: "query" };
cfquery(attributeCollection = "#zzcimQAttrs#") {
	writeOutput("SELECT name FROM zzcimSrc");
}
assert("CONTROL: cfquery(attributeCollection) honors name + dbtype from the struct",
	zzcimOut.name[1], "alpha");

// --- (1) attributeCollection must deliver method + returnVariable ---
function zzcimAttrColl(required any comp) {
	local.args = { component: arguments.comp, method: "hello", returnVariable: "local.rv" };
	try {
		cfinvoke(attributeCollection = "#local.args#");
	} catch (any e) {
		return "THREW: " & e.message;
	}
	return StructKeyExists(local, "rv") ? local.rv : "RV-UNSET";
}
assert("attributeCollection delivers method + returnVariable",
	zzcimAttrColl(zzcimFx), "hello-world");

// --- (1) the EXACT Global.cfc $invoke() shape: argumentCollection nested in the attrColl ---
function zzcimWheelsShape(required any comp) {
	local.args = {
		component: arguments.comp,
		method: "hello",
		returnVariable: "local.rv",
		argumentCollection: { who: "wheels" }
	};
	try {
		cfinvoke(attributeCollection = "#local.args#");
	} catch (any e) {
		return "THREW: " & e.message;
	}
	return StructKeyExists(local, "rv") ? local.rv : "RV-UNSET";
}
assert("Wheels $invoke() shape: nested argumentCollection reaches the method",
	zzcimWheelsShape(zzcimFx), "hello-wheels");

// --- (2) returnVariable, direct named attrs, plain name, function level ---
function zzcimDirectRv(required any comp) {
	try {
		cfinvoke(component = arguments.comp, method = "hello", returnVariable = "zzcimInnerRv");
	} catch (any e) {
		return "THREW: " & e.message;
	}
	return IsDefined("zzcimInnerRv") ? zzcimInnerRv : "RV-UNSET";
}
assert("direct-attrs returnVariable is delivered inside a function",
	zzcimDirectRv(zzcimFx), "hello-world");

// --- (2) returnVariable, dotted local.rv, pre-initialized ---
function zzcimDottedRv(required any comp) {
	local.rv = "";
	try {
		cfinvoke(component = arguments.comp, method = "hello", returnVariable = "local.rv");
	} catch (any e) {
		return "THREW: " & e.message;
	}
	return "[" & local.rv & "]";
}
assert("dotted local.rv returnVariable is written",
	zzcimDottedRv(zzcimFx), "[hello-world]");

// --- (2) returnVariable, page level ---
try {
	cfinvoke(component = zzcimFx, method = "hello", returnVariable = "zzcimPageRv");
	zzcimPageReport = IsDefined("zzcimPageRv") ? zzcimPageRv : "RV-UNSET";
} catch (any e) {
	zzcimPageReport = "THREW: " & e.message;
}
assert("page-level returnVariable is delivered", zzcimPageReport, "hello-world");

// --- (3) componentless cfinvoke inside a CFC dispatches the SIBLING method ---
assert("componentless cfinvoke dispatches the sibling method, not a component path",
	zzcimFx.invokeSibling(), "sibling-ok");

suiteEnd();
</cfscript>
