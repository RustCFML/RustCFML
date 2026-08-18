/**
 * A DIFFERENT class in the same package as AccessGate — used to prove that
 * `private` is class-level (refused here) while `package` is package-level
 * (allowed here). See GH #330.
 */
component {

	private string function otherPriv() {
		return "otherPriv";
	}

	public string function reachPrivateOf( required any target ) {
		return arguments.target.priv();
	}

	public string function reachPackageOf( required any target ) {
		return arguments.target.pkg();
	}
}
