component {

	variables.shared   = "SHARED";
	variables.cfgHome  = "/cfg";

	// a() has a var-local `loc`; b() must NOT see it, but MUST reach the shared
	// component `this`/`variables` through the bare call.
	public string function run() {
		var loc = "SHOULD-NOT-LEAK";
		return b();
	}
	private string function b() {
		var thisOk  = isValid( "string", this.getShared() ) ? "ok" : "no";
		var leakVal = loc ?: "CLEAN";
		return "this=" & thisOk & ";var=" & variables.shared & ";leak=" & leakVal;
	}
	public string function getShared() {
		return variables.shared;
	}

	// A component variable set at construction must remain reachable when a
	// method calls another method that reads it (inherited-scope propagation —
	// the flip side of the leak fix: legitimate lexical scope must still flow).
	public string function chain() {
		return level1();
	}
	private string function level1() {
		return level2();
	}
	private string function level2() {
		return variables.cfgHome & "/path";
	}
}
