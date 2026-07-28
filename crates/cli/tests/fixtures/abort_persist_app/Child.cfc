component extends="Parent" {
	public string function greet() {
		return "child->" & super.greet();
	}
}
