<!--- §107/§108: Lucee 7.1 wording for undefined variables, missing keys, out-of-range
      array reads and unknown functions; bracket reads THROW (they returned quietly). --->
<cfscript>
suiteBegin("Error wording matches Lucee (undefined variable / missing key / array index / function)");

function ewMsg(fn) {
    try { fn(); return "NO THROW"; } catch (any e) { return e.type & "|" & e.message; }
}

ewSt = { a = 1, nested = { x = 1 } };
ewArr = [ 1, 2 ];
ewQ = queryNew( "col", "varchar", [ [ "v" ] ] );

assert("bare undefined variable", ewMsg(function(){ x = ewNoSuchVar; }), "expression|variable [EWNOSUCHVAR] doesn't exist");
assert("dot missing key is upper-cased", ewMsg(function(){ x = ewSt.missingKey; }), "expression|key [MISSINGKEY] doesn't exist");
assert("bracket missing key keeps its casing", ewMsg(function(){ x = ewSt["MissingKey"]; }), "expression|key [MissingKey] doesn't exist");
assert("bracket dynamic key keeps its casing", ewMsg(function(){ k = "dynKey"; x = ewSt[k]; }), "expression|key [dynKey] doesn't exist");
assert("nested dot miss names the leaf", ewMsg(function(){ x = ewSt.nested.deep; }), "expression|key [DEEP] doesn't exist");
assert("nested dot miss on a missing intermediate names it", ewMsg(function(){ x = ewSt.nope.deep; }), "expression|key [NOPE] doesn't exist");
assert("nested bracket miss", ewMsg(function(){ x = ewSt["nested"]["zz"]; }), "expression|key [zz] doesn't exist");
assert("numeric bracket key on a struct", ewMsg(function(){ x = ewSt[1]; }), "expression|key [1] doesn't exist");
assert("variables.x miss", ewMsg(function(){ x = variables.ewNoSuchVar; }), "expression|key [EWNOSUCHVAR] doesn't exist");
assert("variables['x'] miss", ewMsg(function(){ x = variables["ewNoSuchVar"]; }), "expression|key [ewNoSuchVar] doesn't exist");
assert("request.x miss names the scope", ewMsg(function(){ x = request.ewNoSuchVar; }), "expression|key [EWNOSUCHVAR] doesn't exist in the request scope");
assert("request['x'] miss names the scope", ewMsg(function(){ x = request["ewNoSuchVar"]; }), "expression|key [ewNoSuchVar] doesn't exist in the request scope");
assert("local.x miss", ewMsg(function(){ x = local.ewNoSuchVar; }), "expression|key [EWNOSUCHVAR] doesn't exist");
assert("local['x'] miss", ewMsg(function(){ x = local["ewNoSuchVar"]; }), "expression|key [ewNoSuchVar] doesn't exist");
assert("url.x miss", ewMsg(function(){ x = url.ewNoSuchVar; }), "expression|key [EWNOSUCHVAR] doesn't exist");
assert("form.x miss", ewMsg(function(){ x = form.ewNoSuchVar; }), "expression|key [EWNOSUCHVAR] doesn't exist");
assert("application.x miss", ewMsg(function(){ x = application.ewNoSuchVar; }), "expression|key [EWNOSUCHVAR] doesn't exist");
assert("server.x miss", ewMsg(function(){ x = server.ewNoSuchVar; }), "expression|key [EWNOSUCHVAR] doesn't exist");

function ewArgsDot(alpha, beta) { return arguments.gamma; }
function ewArgsBracket(alpha) { return arguments["nope"]; }
function ewArgsPositional(alpha, beta) { return arguments[5]; }
assert("arguments.x miss lists the declared keys", ewMsg(function(){ ewArgsDot(1, 2); }), "expression|The key [GAMMA] doesn't exist in the arguments scope. The existing keys are [alpha, beta]");
assert("arguments.x miss lists declared keys even when omitted", ewMsg(function(){ ewArgsDot(1); }), "expression|The key [GAMMA] doesn't exist in the arguments scope. The existing keys are [alpha, beta]");
assert("arguments['x'] miss keeps casing", ewMsg(function(){ ewArgsBracket(1); }), "expression|The key [nope] doesn't exist in the arguments scope. The existing keys are [alpha]");
assert("arguments[N] out of range", ewMsg(function(){ ewArgsPositional(1, 2); }), "expression|The key [5] doesn't exist in the arguments scope. The existing keys are [alpha, beta]");

assert("arr[9] out of range", ewMsg(function(){ x = ewArr[9]; }), "expression|Array index [9] out of range, array size is [2]");
assert("arr[0] out of range", ewMsg(function(){ x = ewArr[0]; }), "expression|Array index [0] out of range, array size is [2]");
assert("empty[1] out of range", ewMsg(function(){ e = []; x = e[1]; }), "expression|Array index [1] out of range, array size is [0]");
assert("arr['2'] string index reads", ewMsg(function(){ x = ewArr["2"]; }), "NO THROW");

assert("undefined function", ewMsg(function(){ ewNoSuchFn(); }), "expression|No matching function [EWNOSUCHFN] found");
assert("undefined function with args", ewMsg(function(){ ewNoSuchFnMixed(1, 2); }), "expression|No matching function [EWNOSUCHFNMIXED] found");

assert("query missing column, dot", ewMsg(function(){ x = ewQ.nocol; }), "database|Column [NOCOL] not found in query");
assert("query missing column, bracket", ewMsg(function(){ x = ewQ["nocol"]; }), "database|Column [nocol] not found in query");

assert("deleted variable reads as undefined", ewMsg(function(){ ewY = 1; structDelete(variables, "ewY"); x = ewY; }), "expression|variable [EWY] doesn't exist");
assert("member call on a missing key names the key", ewMsg(function(){ ewSt.zz.foo(); }), "expression|key [ZZ] doesn't exist");

// The tolerant forms stay quiet.
assert("isNull(st['missing']) is true, no throw", isNull(ewSt["missing"]), true);
assert("st['missing'] ?: default", ewSt["missing"] ?: "d", "d");
assert("arr[5] ?: default", ewArr[5] ?: "d", "d");
assert("st?.missing is null", isNull(ewSt?.missing), true);
assert("isDefined on a bracket path is false", isDefined("ewSt['missing']"), false);
assert("structKeyExists is false", structKeyExists(ewSt, "missing"), false);
assert("negative array index reads null, no throw", ewMsg(function(){ x = ewArr[-1]; }), "NO THROW");

// Compound assignment / auto-vivification through a bracket path still works.
ewSt["counter"] = 1;
ewSt["counter"] += 1;
assert("bracket compound assign", ewSt["counter"], 2);
ewViv = {};
ewViv["a"]["b"] = 3;
assert("bracket auto-vivify", ewViv.a.b, 3);

// for-in reads the array length LIVE (Lucee): a body that deletes skips the
// next element and ends early; one that appends iterates the new elements.
ewDel = [ 1, 2, 3, 4 ];
ewSeen = [];
for ( ewX in ewDel ) { arrayAppend( ewSeen, ewX ); if ( ewX == 2 ) arrayDeleteAt( ewDel, 2 ); }
assert("delete during for-in skips and ends early", arrayToList( ewSeen ), "1,2,4");
ewApp = [ 1, 2 ];
ewSeen = [];
for ( ewX in ewApp ) { arrayAppend( ewSeen, ewX ); if ( arrayLen( ewApp ) < 4 ) arrayAppend( ewApp, arrayLen( ewApp ) + 1 ); }
assert("append during for-in is iterated", arrayToList( ewSeen ), "1,2,3,4");
// cfloop array= (tag and script) is bounded by min(entry length, live length):
// a delete ends it early, an appended element is NOT iterated (Lucee).
ewApp = [ 1, 2 ];
ewSeen = [];
cfloop( array = ewApp, item = "ewX" ) { arrayAppend( ewSeen, ewX ); if ( arrayLen( ewApp ) < 4 ) arrayAppend( ewApp, arrayLen( ewApp ) + 1 ); }
assert("cfloop array: append is not iterated", arrayToList( ewSeen ), "1,2");
ewApp = [ 1, 2 ];
ewSeen = [];
cfloop( array = ewApp, item = "ewX", index = "ewI" ) { arrayAppend( ewSeen, ewI & ":" & ewX ); if ( arrayLen( ewApp ) < 4 ) arrayAppend( ewApp, arrayLen( ewApp ) + 1 ); }
assert("cfloop array item+index: append is not iterated", arrayToList( ewSeen ), "1:1,2:2");
ewDel = [ 1, 2, 3, 4 ];
ewSeen = [];
cfloop( array = ewDel, item = "ewX" ) { arrayAppend( ewSeen, ewX ); if ( ewX == 2 ) arrayDeleteAt( ewDel, 2 ); }
assert("cfloop array: delete skips and ends early", arrayToList( ewSeen ), "1,2,4");

suiteEnd();
</cfscript>
