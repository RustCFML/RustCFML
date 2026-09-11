component {
	variables.secret = "from-provider";
	// Handed to another instance as an injected method. Lucee binds a UDF to
	// the component it is INVOKED on, so `variables` here is the target's.
	public function matcherFn( required any expected ) {
		variables.calls++;
		return variables.secret & ":" & arguments.expected & ":" & variables.calls;
	}
	public function getMatcher() { return matcherFn; }
	// A closure keeps what it captured from its defining scope.
	public function getClosure() {
		var captured = variables.secret;
		return function( x ) { return captured & ":" & arguments.x & ":" & variables.secret; };
	}
}
