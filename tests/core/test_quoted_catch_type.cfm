<cfscript>
suiteBegin("Core: quoted string catch type");

// ============================================================
// Background
// ============================================================
// A try/catch clause may name the exception type either as a bare identifier
// (`catch (any e)`, `catch (Application e)`) or as a QUOTED STRING literal
// (`catch ("My.Custom.Type" e)`). The quoted form is how CFML catches a dotted,
// namespaced custom exception, and it is accepted on Lucee 5/6/7, Adobe CF
// 2018-2025, and BoxLang.
//
// RustCFML 0.37.0's catch parser expects a bare identifier in the type position
// and rejects the string literal: "Expected identifier, found String(...)". The
// enclosing component then fails to parse and degrades to a non-object.
//
// This is the next blocker on the Wheels boot path after the component-header
// gaps: vendor/wheels/Public.cfc:56 (instantiated at onApplicationStart) does
// `catch ("Wheels.Packages.RegistryUnavailable" e) {`, and
// vendor/wheels/auth/JwtStrategy.cfc does `catch ("Wheels.Auth.JWT.TokenExpired" e) {`.
//
// The failing catch lives in a fixture (a parse error escapes try/catch and
// would abort the runner); via createObject it degrades to a non-object.
// ============================================================

function loadProbe(required string name) {
	var o = createObject("component", arguments.name);
	return isObject(o) ? o.probe() : "NOT-A-COMPONENT";
}

assert("a quoted string catch type parses and catches the thrown type", loadProbe("QuotedCatchFixture"), "caught");

suiteEnd();
</cfscript>
