<cfscript>
// invoke() must keep undeclared named arguments (Lucee parity).
//
// Direct method calls already keep named arguments that match no declared
// cfargument — they land in the arguments scope by name (covered by
// core/test_undeclared_named_args.cfm). Lucee behaves the same when the call
// goes through invoke() or <cfinvoke>. RustCFML currently DROPS the
// undeclared names on the invoke() path, so only declared parameters arrive.
// This breaks dynamic dispatch frameworks that pass route/request parameters
// as an argument struct: invoke(handler, endpoint, args) silently loses
// every key the handler did not declare.

suiteBegin("Core: invoke() keeps undeclared named arguments");

o = createObject("component", "InvokeUndeclaredArgFixture");

// --- control: direct call keeps undeclared named args ---
assert("direct call keeps undeclared named args (control)",
	o.probe(a = "A", b = "B", c = "C"), "a=A,b=B,c=C");

// --- control: invoke() passes DECLARED args fine (no missing-arg error) ---
declaredOnly = invoke(o, "probe", { a: "A" });
assertTrue("invoke() passes declared args (control)",
	find("MISSING", declaredOnly) EQ 1);

// --- gap: invoke() with an argument struct keeps undeclared names ---
assert("invoke() keeps undeclared named args",
	invoke(o, "probe", { a: "A", b: "B", c: "C" }), "a=A,b=B,c=C");
</cfscript>

<!--- --- gap: <cfinvoke> with extra attributes behaves like Lucee too --- --->
<cfinvoke component="#o#" method="probe" returnvariable="tagResult"
	a="A" b="B" c="C" />

<cfscript>
assert("cfinvoke keeps undeclared named args", tagResult, "a=A,b=B,c=C");

suiteEnd();
</cfscript>
