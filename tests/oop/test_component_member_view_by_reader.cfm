<cfscript>
suiteBegin("Component member view follows the READER, not the syntax (GH ##420/##417)");

// ---------------------------------------------------------------------------
// INSIDE the component: dot, bracket and `variables[]` are ONE read.
//
// v0.636.0 resolved all three. v0.655.0 answered NULL for the bracket form
// only, because #417 gated the read paths per OPCODE and an opcode cannot know
// who is reading — so `GetIndex` denied the insider while `GetProperty` still
// served the outsider. Lucee resolves all three.
// ---------------------------------------------------------------------------
o = new MemberViewFixture();
assert("dot/bracket/variables agree inside the component"
      , o.nativeLookup()
      , "dot=fn bracket=fn vars=fn");

// A private method extracted BY BRACKET is a callable function value.
assert("bracket-extracted private method is callable", o.callThroughBracket(), "priv");

// The Preside merger shape: every declared function re-homes, private included.
// getMetaData().functions lists pub, priv, nativeLookup, callThroughBracket,
// reHomeOwnFunctions and readSibling — none may come back Null.
target = {};
moved  = o.reHomeOwnFunctions( target );
assert("every declared function re-homed by this[ name ]", moved, 6);
assertTrue("the PRIVATE one re-homed too", structKeyExists( target, "priv" ));
assert("re-homed private method runs", target.priv(), "priv");

// `private` is class-level, not instance-level: a sibling of the same class is
// readable from inside (same rule the method-access gate already applies).
other = new MemberViewFixture();
assert("a sibling instance's private scope is readable from inside"
      , o.readSibling( other )
      , "fn/shh");

// ---------------------------------------------------------------------------
// OUTSIDE the component: the private scope is not part of the surface, in ANY
// spelling and through ANY receiver shape.
//
// `c.secret` was already gated (the compiler fuses a bare local into
// LoadLocalProperty), but a receiver the compiler CANNOT fuse — an array
// element, a struct member — went through GetProperty's Instance arm, which
// #417 left on the full view. So `arr[ 1 ].secret` still read the private
// scope 20 releases after it was declared fixed.
// ---------------------------------------------------------------------------
assertTrue("outside: c.secret is unreachable"        , isNull( o.secret ));
assertTrue('outside: c[ "secret" ] is unreachable'   , isNull( o[ "secret" ] ));
assertTrue("outside: c.priv is unreachable"          , isNull( o.priv ));
assertTrue('outside: c[ "priv" ] is unreachable'     , isNull( o[ "priv" ] ));

arr = [ o ];
assertTrue("outside: arr[1].secret is unreachable"   , isNull( arr[ 1 ].secret ));
assertTrue("outside: arr[1].priv is unreachable"     , isNull( arr[ 1 ].priv ));

holder = { k = o };
assertTrue("outside: st.k.secret is unreachable"     , isNull( holder.k.secret ));
assertTrue('outside: st.k[ "secret" ] is unreachable', isNull( holder.k[ "secret" ] ));

// Being inside SOME component is not being inside THIS one: a component of
// another class is an outsider, in both spellings.
assert("a foreign class is an outsider"
      , new MemberViewForeign().probeOther( o )
      , "priv=NULL bracket=NULL dot=NULL");

// ---------------------------------------------------------------------------
// Insider is a CLASS relationship, and it follows inheritance and closures.
// ---------------------------------------------------------------------------
child = new MemberViewChild();
assert("a subclass reaches an inherited private member"
      , child.readInheritedPrivate()
      , "bracket=fn data=shh");
assert("a closure minted inside a method reads as an insider"
      , child.readFromClosure()
      , "fn");

// The public surface is unaffected in every spelling.
assert("outside: public method still dispatches", o.pub(), "pub");
assertFalse("outside: public method reads as a value", isNull( o[ "pub" ] ));

suiteEnd();
</cfscript>
