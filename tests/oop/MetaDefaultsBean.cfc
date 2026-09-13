component {
	// GH #399: reflection must report each parameter's declared `default` with
	// its own type, and a `type` of "any" when none is declared.
	public boolean function validator2( fieldName, value, someParam="test", anotherParam=false, num=7 ) validatorMessage="x" {
		return true;
	}
	public string function withTypes( required string a, numeric b=1.5 ) {
		return "x";
	}
	public function runtimeDefaults( a=now(), b=[ 1, 2 ] ) {}
}
