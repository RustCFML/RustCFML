// Fixture: a try/catch whose catch clause names the exception type as a QUOTED
// string literal (a dotted custom type that is not a bare identifier). On
// Lucee/Adobe CF/BoxLang the catch type may be written as a quoted string, which
// is how you catch a namespaced custom exception. On RustCFML 0.37.0 the catch
// parser expects a bare identifier and rejects the string literal ("Expected
// identifier, found String(...)"), so the component degrades to a non-object.
//
// Wheels uses this on the boot path — e.g. vendor/wheels/Public.cfc:
//   catch ("Wheels.Packages.RegistryUnavailable" e) { ... }
// and vendor/wheels/auth/JwtStrategy.cfc:
//   catch ("Wheels.Auth.JWT.TokenExpired" e) { ... }
component {
	public string function probe() {
		try {
			throw(type = "A.B.C", message = "boom");
		} catch ("A.B.C" e) {
			return "caught";
		}
		return "not-caught";
	}
}
