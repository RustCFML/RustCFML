component {
	public string function greet( required string who, string punct = "!" ) {
		return "hi " & arguments.who & arguments.punct;
	}
}
