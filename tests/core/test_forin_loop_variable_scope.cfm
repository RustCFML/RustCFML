<cfscript>
suiteBegin("for-in loop variable lands where a plain assignment would (Lucee parity)");

// Lucee 7.1, probed with this fixture: the loop variable of `for (i in arr)` is
// an ordinary unscoped write. In a classic-localmode method it goes to the
// component's `variables` scope (NOT `local`); in a closure inside a method
// likewise; at page level it is a page variable; under localmode="modern" it is
// in `local`. RustCFML used to declare it function-local in every shape.
o = new ForInScopeFixture();
assert( "classic method: variables, not local", o.classic(), "vars=true local=false" );
assert( "modern method: local",                 o.modern(),  "local=true" );
assert( "for (var v in …): local",                o.varForm(), "vars=false local=true" );
assert( "closure in a method: variables",        o.viaClosure(), "vars=true local=false" );
assert( "loop var readable after the loop",      o.afterLoop(), 7 );
for (forInPageVar in [1]) {}
assert( "page level: a page variable", structKeyExists(variables, "forInPageVar"), true );
function forInPageUdf() { for (u in [1]) {} return "vars=" & structKeyExists(variables,"u") & " local=" & structKeyExists(local,"u"); }
assert( "page UDF: the page variables scope", forInPageUdf(), "vars=true local=false" );

suiteEnd();
</cfscript>
