<cfscript>
suiteBegin("private/package methods are invisible to external dispatch (GH ##330)");

// Access modifiers are enforced on member dispatch exactly as Lucee enforces
// them (ComponentImpl.isAccessible + isPrivate/isPackage):
//   * public/remote  — callable from anywhere
//   * private        — callable from inside the same CLASS (so a sibling INSTANCE
//                      of the class counts: privacy is class-level, as in Java)
//   * package        — callable from any component in the same package
// A denied method is reported as ABSENT, not as access-denied, so a component
// with an onMissingMethod routes there instead of throwing. Every expectation
// below was verified against Lucee 7.0.4.34.

o     = new AccessGate();
other = new AccessGateOther();
child = new AccessGateChild();
omm   = new AccessGateOmm();

// --- callable from outside -------------------------------------------------
assert("public method callable from outside", o.pub(), "pub");
assert("remote method callable from outside", o.rem(), "rem");

// --- refused from outside --------------------------------------------------
assertThrows("private method not callable from outside", () => o.priv());
assertThrows("package method not callable from a plain template", () => o.pkg());
assertThrows("private method not callable via invoke() from outside", () => invoke( o, "priv" ));
assertThrows("private method not callable via bracket dispatch", () => o[ "priv" ]());
assertThrows("inherited private not callable from outside", () => child.parentPriv());
assertThrows("child's own private not callable from outside", () => child.childPriv());

// The refusal reports the method as MISSING (Lucee's ComponentUtil.notFunction),
// never as "access denied" — a private method is invisible, not merely refused.
missingMsg = "";
try { o.priv(); } catch ( any e ) { missingMsg = e.message; }
assertTrue("refusal reads as 'has no function with name'",
	findNoCase( "has no", missingMsg ) GT 0 AND findNoCase( "function with name", missingMsg ) GT 0);

// --- reachable from inside the class --------------------------------------
assert("unqualified sibling call reaches private", o.callUnqualified(), "priv");
assert("this.priv() reaches private", o.callViaThis(), "priv");
assert("variables.priv() reaches private", o.callViaVariables(), "priv");
assert("invoke(this,'priv') reaches private", o.callViaInvoke(), "priv");
assert("unqualified call reaches package method", o.callPackageUnqualified(), "pkg");
assert("private is class-level: sibling instance may call it", o.callOnSibling(), "priv");
assert("closure minted inside the component keeps its access", o.makePrivateCaller()(), "priv");

// --- inheritance ----------------------------------------------------------
assert("child method calls inherited private", child.childCallsInheritedPrivate(), "parentPriv");
assert("child this.parentPriv() allowed", child.childCallsInheritedPrivateViaThis(), "parentPriv");
assert("super.parentPriv() allowed", child.childCallsInheritedPrivateViaSuper(), "parentPriv");
assert("parent method calls its own private on a child instance", child.parentCallsPrivate(), "parentPriv");
assert("parent this.parentPriv() on a child instance", child.parentCallsPrivateViaThis(), "parentPriv");

// --- cross-class ----------------------------------------------------------
assertThrows("another class cannot reach a private method", () => other.reachPrivateOf( o ));
assertThrows("a component cannot reach another class's private method", () => o.callForeignPrivate( other ));
assert("package method IS reachable from the same package", other.reachPackageOf( o ), "pkg");

// --- onMissingMethod ------------------------------------------------------
// Denied == absent, so the call routes to onMissingMethod rather than throwing —
// and must NOT be answered by the lenient implicit-accessor synthesis either.
assert("denied private routes to onMissingMethod", omm.hidden(), "omm:hidden");

// --- introspection --------------------------------------------------------
assertFalse("structKeyExists hides a private method", structKeyExists( o, "priv" ));

suiteEnd();
</cfscript>
