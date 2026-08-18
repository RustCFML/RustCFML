/**
 * Fixture for the `private`/`package` access gate on external method dispatch
 * (GH #330). Every internal-call shape a component can use to reach its own
 * non-public methods lives here, so the gate can be pinned against Lucee.
 */
component {

	public string function pub() {
		return "pub";
	}

	remote string function rem() {
		return "rem";
	}

	private string function priv() {
		return "priv";
	}

	package string function pkg() {
		return "pkg";
	}

	// --- internal call shapes -------------------------------------------------

	public string function callUnqualified() {
		return priv();
	}

	public string function callViaThis() {
		return this.priv();
	}

	public string function callViaVariables() {
		return variables.priv();
	}

	public string function callViaInvoke() {
		return invoke( this, "priv" );
	}

	public string function callPackageUnqualified() {
		return pkg();
	}

	// A sibling INSTANCE of the same class: Lucee's privacy is class-level, so
	// reaching another instance's private method from inside the class is legal.
	public string function callOnSibling() {
		var other = new AccessGate();
		return other.priv();
	}

	// A closure minted inside a component method, invoked LATER from outside.
	// Lucee unwraps the ClosureScope to the component scope, so the closure keeps
	// its owner's access rights.
	public any function makePrivateCaller() {
		return function() {
			return this.priv();
		};
	}

	// Reaching into an instance of a DIFFERENT class — must be refused.
	public string function callForeignPrivate( required any target ) {
		return arguments.target.otherPriv();
	}
}
