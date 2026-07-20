<cfscript>
suiteBegin("Core: undeclared named arguments keep their names");

o = createObject("component", "UndeclaredArgFixture");
assert("extra named args are reachable by name in the arguments scope",
	o.probe(), "a=A,b=B,c=C");

// Regression: unscoped compound write to an UNDECLARED named-arg struct must
// mutate the arguments-scope struct (by reference), not a phantom frame local.
// This is the ColdBox preHandler( event, action ) pattern that populates `prc`.
myPrc = {};
o.preHandlerLike( event = "e", action = "a", prc = myPrc );
assert("unscoped member-write in an undeclared-arg method mutates the caller's struct by reference",
	myPrc.injectedSingle ?: "MISSING", "from-prehandler");

// Multi-level nested member-write on the same undeclared-arg struct.
myPrc2 = {};
o.preHandlerLikeNested( event = "e", action = "a", prc = myPrc2 );
assert("nested member-write in an undeclared-arg method reaches the caller's struct",
	myPrc2.nested.deep ?: "MISSING", "from-prehandler-nested");

suiteEnd();
</cfscript>
