component {

	// Pseudo-constructor: seed the state the matrix rows assert against.
	this.pre = "A";
	this.bag = { n = 0 };
	variables.vMark = "";

	public void function noop() {
	}

	public void function writeNested() {
		this.nestedMark = "N";
	}

	// ------------------------------------------------------------
	// RED rows on RustCFML 0.108.0 -- each performs a `this.`-DOT
	// qualified method call, then a this-write. The write is visible
	// in-frame (returned string) but discarded when the frame returns.
	// ------------------------------------------------------------

	public string function dotCallThenNewKeyWrite() {
		this.noop();        // dot-qualified call: detaches `this` on RustCFML
		this.mkNew = "X";   // lands on the detached copy
		return "inFrame=" & structKeyExists(this, "mkNew");
	}

	public string function dotCallThenOverwrite() {
		this.noop();
		this.pre = "B";     // existing key: silently reverts to "A" on return
		return "inFrame=" & this.pre;
	}

	public string function dotCallThenNestedBareWrite() {
		this.noop();
		writeNested();      // a frame called AFTER the dot-call is poisoned too
		return "inFrame=" & structKeyExists(this, "nestedMark");
	}

	// ------------------------------------------------------------
	// GREEN control rows -- the closest non-detaching neighbours of the
	// trigger shape. All already pass on RustCFML 0.108.0.
	// ------------------------------------------------------------

	public string function bareCallThenWrite() {
		noop();             // bare call: no detach
		this.mkBare = "X";
		return "inFrame=" & structKeyExists(this, "mkBare");
	}

	public string function bracketCallThenWrite() {
		this["noop"]();     // bracket-dispatch on this: no detach
		this.mkBracket = "X";
		return "inFrame=" & structKeyExists(this, "mkBracket");
	}

	public string function dotReadThenWrite() {
		var f = this.noop;  // dot-READ (no call): no detach
		this.mkRead = "X";
		return "inFrame=" & structKeyExists(this, "mkRead");
	}

	public string function otherObjectDotCallThenWrite() {
		var other = createObject("component", "ThisDotCallDetachFixture");
		other.noop();       // dot-call, but the receiver is NOT this frame's `this`
		this.mkOther = "X";
		return "inFrame=" & structKeyExists(this, "mkOther");
	}

	public string function helperMemberDotCallThenWrite() {
		this.helperObj.noop();  // dot-call THROUGH a this-member: receiver is helperObj
		this.mkHelper = "X";
		return "inFrame=" & structKeyExists(this, "mkHelper");
	}

	public string function dotCallThenVariablesWrite() {
		this.noop();
		variables.vMark = "V";  // the variables scope is never snapshotted
		return "inFrame=" & variables.vMark;
	}

	public string function getVMark() {
		return variables.vMark;
	}

	// PIN: the detached copy is SHALLOW -- a nested struct reached through
	// `this` is a shared reference, so its mutation escapes the detach.
	public string function dotCallThenNestedStructMutation() {
		this.noop();
		this.bag.n = 7;
		return "inFrame=" & this.bag.n;
	}
}
