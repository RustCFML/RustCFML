<cfscript>
suiteBegin("Component metadata defaults, and a per-instance pseudo-constructor include (GH 468, 481)");

// GH 468 — getComponentMetadata() reports each parameter's default: the literal
// for a literal, "[runtime expression]" for anything else. Frameworks read
// defaults from metadata to build schemas and injections.
params = {};
for ( p in getComponentMetadata( "oop.fixtures.MetaDefaultsProbe" ).functions[ 1 ].parameters ) {
	params[ p.name ] = structKeyExists( p, "default" ) ? p.default : "<none>";
}
assert("a required parameter has no default", params.a, "<none>");
assert("a numeric literal default", params.n, 25);
assert("a string literal default", params.s, "x");
assert("a boolean literal default", params.b, false);
assert("a struct default is a runtime expression", params.st, "[runtime expression]");
assert("an array default is a runtime expression", params.ar, "[runtime expression]");
assert("a call default is a runtime expression", params.expr, "[runtime expression]");
assert("an undeclared default is absent", params.noDefault, "<none>");

// GH 481 — a pseudo-constructor include whose path depends on runtime state is
// resolved per INSTANCE. The second instance used to keep the first instance's
// functions, whatever its own path resolved to.
request.gh481Dir = "dirA";
first = new oop.fixtures.gh481.DynBase();
request.gh481Dir = "dirB";
second = new oop.fixtures.gh481.DynBase();
assert("the first instance has its own template's function",
	structKeyExists( first, "gh481HelperA" ), true);
assert("the first instance does not have the other's", structKeyExists( first, "gh481HelperB" ), false);
assert("the second instance has ITS template's function",
	structKeyExists( second, "gh481HelperB" ), true);
assert("the second instance does not keep the first's",
	structKeyExists( second, "gh481HelperA" ), false);
assert("and the function is callable", second.gh481HelperB(), "B");

// GH 481 (second report) — the include's path is a LITERAL, but it sits under a
// condition the first instance fails. The include runs per instance, so the
// second instance (condition now true) must get the template's functions even
// though the first construction built the class without them.
structDelete( request, "gh481Ready" );
condFirst = new oop.fixtures.gh481.CondSub();
request.gh481Ready = true;
condSecond = new oop.fixtures.gh481.CondSub();
assert("the skipping instance does not have the helper",
	structKeyExists( condFirst, "gh481HelperG" ), false);
assert("the including instance has the helper",
	structKeyExists( condSecond, "gh481HelperG" ), true);
assert("and an unscoped call from a method finds it", condSecond.gh481CallHelper(), "g");
structDelete( request, "gh481Ready" );

suiteEnd();
</cfscript>
