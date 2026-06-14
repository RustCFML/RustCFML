// Fixture for cfinvoke argumentCollection positional/numeric-key forwarding.
// take() has a positional first param + a named param; the test confirms a
// numeric-keyed argumentCollection entry arrives as the positional arg.
component {
	public string function take(any a, string named = "DEF") {
		var aDesc = isStruct(arguments.a) ? ("struct:" & (arguments.a.body ?: "?"))
			: (isNull(arguments.a) ? "(null)" : toString(arguments.a));
		return "a=" & aDesc & "|named=" & arguments.named;
	}
}
