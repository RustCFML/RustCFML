<cfscript>
suiteBegin("Core: savecontent delivers to a scope-qualified target variable");

// ============================================================
// Background
// ============================================================
// savecontent's variable= attribute accepts a SCOPE-QUALIFIED name:
// inside a function, variable="local.cap" must populate local.cap, and
// variable="variables.cap" must populate the variables scope — on Lucee
// (and Adobe CF) every form works identically to the unqualified one.
//
// RustCFML 0.128.0 captures correctly for UNQUALIFIED targets
// (variable="cap", incl. a pre-declared `var cap`) but silently drops
// the capture for scope-qualified targets: after the block,
// local.cap / variables.cap simply don't exist. The body DOES execute
// (side effects fire) — only the captured string is lost.
//
//   function f() {
//       savecontent variable="local.cap" { writeOutput("X"); }
//       return structKeyExists(local, "cap");   // RustCFML: false | Lucee: true
//   }
//
// Why it matters for Wheels: EVERY view render runs through
// $includeAndReturnOutput (vendor/wheels/Global.cfc):
//
//       savecontent variable="local.$wheels" { include "#template#"; }
//       return local.$wheels;
//
// so on RustCFML every rendered view comes back EMPTY — the framework
// boots, routes, and executes the view, then returns nothing. This is
// the last gap standing between Wheels' blog tutorial app and a fully
// stock RustCFML binary.
// ============================================================

function scstLocal() {
    savecontent variable="local.scstCap" { writeOutput("LOCAL_OK"); }
    return structKeyExists(local, "scstCap") ? local.scstCap : "(local.scstCap missing)";
}
function scstVariablesScope() {
    savecontent variable="variables.scstCap2" { writeOutput("VARIABLES_OK"); }
    return structKeyExists(variables, "scstCap2") ? variables.scstCap2 : "(variables.scstCap2 missing)";
}
function scstBare() {
    savecontent variable="scstCap3" { writeOutput("BARE_OK"); }
    return isDefined("scstCap3") ? scstCap3 : "(scstCap3 missing)";
}
function scstVarDeclared() {
    var scstCap4 = "";
    savecontent variable="scstCap4" { writeOutput("VAR_OK"); }
    return scstCap4;
}

// --- the gap: scope-qualified targets must receive the capture ---
assert("variable='local.X' populates local.X (the Wheels view-render shape)",
    scstLocal(), "LOCAL_OK");
assert("variable='variables.X' populates the variables scope",
    scstVariablesScope(), "VARIABLES_OK");

// --- CONTROLS (green on both engines): unqualified targets work ---
assert("CONTROL: unqualified variable='x' captures",
    scstBare(), "BARE_OK");
assert("CONTROL: pre-declared `var x` + variable='x' captures",
    scstVarDeclared(), "VAR_OK");

suiteEnd();
</cfscript>
