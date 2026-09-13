<cfscript>
// GH #399. getMetaData(cfc).functions[n].parameters[m] carried only `name` and
// `required`. Lucee also reports `type` (defaulting to "any") and, when the
// parameter declares one, `default` — preserving the declared literal's type.
// A parameter declared someParam="test" had no `default` key at all, so
// reflection-driven code (validators, DI, form builders) could not see it.
// Measured against Lucee 7.1.0.204.
suiteBegin("getMetaData parameter defaults");

md = getMetaData( createObject("component", "oop.MetaDefaultsBean") );
params = {};
for ( f in md.functions ) {
	for ( p in f.parameters ) {
		params[ f.name & "." & p.name ] = p;
	}
}

// `type` is always present, "any" when undeclared.
assert("an undeclared type reports any", params["validator2.fieldName"].type, "any");
assert("a declared type is reported", params["withTypes.a"].type, "string");
assert("and so is numeric", params["withTypes.b"].type, "numeric");

// `default` is present only when declared, and keeps the literal's type.
assertFalse("a parameter with no default has no default key",
	structKeyExists( params["validator2.fieldName"], "default" ));
assert("a string default is reported", params["validator2.someParam"].default, "test");
assertTrue("a boolean default stays a boolean",
	isBoolean( params["validator2.anotherParam"].default ));
assertFalse("with its declared value", params["validator2.anotherParam"].default);
assert("a numeric default stays numeric", params["validator2.num"].default, 7);
assert("as does a decimal", params["withTypes.b"].default, 1.5);

// `required` was already correct and must stay so.
assertTrue("a required parameter reports required", params["withTypes.a"].required);
assertFalse("an optional one does not", params["validator2.someParam"].required);

// A default the engine can only evaluate at call time has no value to report;
// Lucee reports the fixed marker string rather than the source text.
assert("a call-time default reports the runtime marker",
	params["runtimeDefaults.a"].default, "[runtime expression]");
assert("an array literal default does too",
	params["runtimeDefaults.b"].default, "[runtime expression]");

// Function-level custom attributes were already reported; guard the regression.
assertTrue("function annotations still survive",
	structKeyExists( params["validator2.someParam"], "name" ));

suiteEnd();
</cfscript>
