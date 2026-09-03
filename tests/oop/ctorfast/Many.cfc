component extends="Base" {
	// The pseudo-constructor calls its OWN and its PARENT's methods before the
	// body has finished — only possible if the full method hoist happened.
	variables.ctorSawOwn      = ownDuringCtor();
	variables.ctorSawParent   = inheritedGreet();
	variables.ctorSawPrivate  = privateOne();
	// A ctor value that collides with a method name must WIN over the method.
	variables.collides = "data-not-method";

	public string function ownDuringCtor()  { return "own-ctor"; }
	public string function pub1()           { return "p1"; }
	private string function privateOne()    { return "priv1"; }
	package string function pkgOne()        { return "pkg1"; }
	public string function overridden()     { return "child-version"; }
	// A method whose name collides with a BIF must stay dispatchable as a method
	// without poisoning the bare BIF (the fast path skips the builtin guard, so
	// this pins that skipping it is correct for component methods).
	public string function ucase()          { return "method-not-bif"; }
	public string function collides()       { return "method-not-data"; }
	// Public witness for the three ctor-time captures. The test used to read
	// `o.ctorSawOwn` etc. directly, which only worked because a component's
	// private member scope was externally readable (GH #417) — Lucee answers
	// that with "has no accessible Member". Same values, through the public
	// surface, so the hoist coverage is unchanged.
	public struct function ctorWitness() {
		return {
			  own    = variables.ctorSawOwn
			, parent = variables.ctorSawParent
			, priv   = variables.ctorSawPrivate
		};
	}
	public string function getPrivate()     { return privateOne(); }
	public string function getPkg()         { return pkgOne(); }
}
