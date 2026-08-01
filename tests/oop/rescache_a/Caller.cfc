component {
	// Bare-name resolution: must find the Shared.cfc sitting next to THIS file.
	public string function resolve() {
		return new Shared().whoAmI();
	}
}
