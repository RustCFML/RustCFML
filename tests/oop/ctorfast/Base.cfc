component {
	variables.builtOrder = [];
	// Called from the CHILD's pseudo-constructor body — proves the method hoist
	// still populates `variables` before the body runs.
	public string function inheritedGreet() { return "base-greet"; }
	private string function inheritedSecret() { return "base-secret"; }
	public string function overridden() { return "base-version"; }
}
