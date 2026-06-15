<cfscript>
suiteBegin("OOP: cfinvoke method=<name> dispatches an unknown method to onMissingMethod");

// Background: invoking a method by name via cfinvoke must follow the same dispatch
// rules as a direct call — if the named method is not declared on the component but
// the component defines onMissingMethod(), cfinvoke must route through it. Lucee,
// Adobe CF and BoxLang all dispatch an unknown cfinvoke target to onMissingMethod.
//
// RustCFML 0.161.0 throws "Method '<name>' not found in component" from cfinvoke
// for an undeclared method, even though a DIRECT dot-call to the same undeclared
// method on the same object correctly routes through onMissingMethod.
//
//   obj.deleteAllWidgets()                                  -> onMissingMethod on BOTH (CONTROL)
//   cfinvoke(component=obj, method="deleteAllWidgets", ...) -> Lucee: onMissingMethod; RustCFML 0.161: THROWS
//
// Why it matters: Wheels dispatches association cascade methods by name via cfinvoke.
// hasMany(dependent="delete"|"deleteAll"|"removeAll") builds a "deleteAll<assoc>"/
// "removeAll<assoc>" method name and invokes it through $invoke()'s cfinvoke
// (Global.cfc), which lands in the model's onMissingMethod. On RustCFML the whole
// parent .delete() throws ("Method 'deleteAllcomments' not found") — the parent is
// not deleted and children are not cascaded — confirmed on the demo blog's own
// Post hasMany Comment dependent="delete".

obj = createObject("component", "oop.CfInvokeOmmTarget");

// --- CONTROL (green on both engines): cfinvoke to a DECLARED method works ---
cfinvoke(component = obj, method = "doSomething", returnVariable = "declaredResult");
assert("CONTROL: cfinvoke to a declared method returns its value", declaredResult, "did-something");

// --- CONTROL (green on both engines): a dot-call to an UNDECLARED method routes to onMissingMethod ---
assert("CONTROL: dot-call to an undeclared method routes through onMissingMethod",
    obj.deleteAllWidgets(), "omm:deleteAllWidgets");

// --- the gap: cfinvoke to an UNDECLARED method must ALSO route through onMissingMethod ---
ivResult = "(threw)";
try {
    cfinvoke(component = obj, method = "deleteAllWidgets", returnVariable = "ivResult");
} catch (any e) {
    ivResult = "THREW: " & e.message;
}
assert("cfinvoke to an undeclared method routes through onMissingMethod (does not throw)",
    ivResult, "omm:deleteAllWidgets");

suiteEnd();
</cfscript>
