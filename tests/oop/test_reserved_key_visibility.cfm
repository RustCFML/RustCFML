<cfscript>
// C2planDoc.md §1 / Phase C.3 Slice 4: a `__`/`___`-prefixed PUBLIC member is
// legitimate user data (FW/1 AOP's `this["___doReverse"]`), not an engine key.
//
// This test is written to pass on BOTH backings:
//   * the legacy marker-struct build (default), which still HIDES `__`-prefixed
//     keys from introspection (the documented divergence, accepted until the
//     flyweight is flipped default-on — C2planDoc.md §8), and
//   * the Phase C.3 `component-instance` flyweight, which shows them (fixed by
//     construction: the instance data maps carry no engine reserved keys, so
//     introspection reads them directly with no `__` filter).
// It therefore asserts the CROSS-SURFACE CONSISTENCY invariant (the property the
// bug actually violated is caught at Slice 6 when the flyweight goes default-on):
// structKeyExists / structKeyList / serializeJSON must all AGREE about a key.
suiteBegin( "Component reserved-key (`__`/`___`) visibility consistency" );

probe = new oop.UnderscoreKeyProbe();
r = probe.probe();

// The value is ALWAYS stored and directly readable on both backings.
assert( "`___orig` is stored and directly readable", r.directRead, "STASHED" );
// A single leading underscore is a normal identifier on every engine.
assert( "single-underscore member is visible", r.singleVisible, true );

// Cross-surface consistency: whatever structKeyExists reports for `___orig`,
// structKeyList and serializeJSON MUST report the same. (On the flyweight all
// three are true — matching Lucee; on the marker path all three are false.)
assert( "keyExists agrees with keyList",     r.keyExists, r.inKeyList );
assert( "keyExists agrees with serializeJSON", r.keyExists, r.inJson );

// When the flyweight backing is active, the key is fully visible (the fix). This
// is only asserted when keyExists is already true, so the marker build (where it
// is false) stays green — the hard, unconditional check lands at Slice 6.
if ( r.keyExists ) {
    assert( "flyweight: `___orig` visible in keyList",       r.inKeyList, true );
    assert( "flyweight: `___orig` visible in serializeJSON", r.inJson,    true );
}

suiteEnd();
</cfscript>
