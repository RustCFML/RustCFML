<cfscript>
// C2planDoc.md §1 / Phase C.3: a `__`/`___`-prefixed PUBLIC member is legitimate
// user data (FW/1 AOP's `this["___doReverse"]`/`___orig`), NOT an engine key.
// Lucee/ACF treat `__`/`___` as ordinary identifiers and surface them in
// structKeyExists / structKeyList / serializeJSON / for-in / writeDump.
//
// This now passes IDENTICALLY on BOTH backings:
//   * the flyweight (`component-instance`) fixes it by construction (the instance
//     data maps carry no engine reserved keys, so introspection reads them
//     directly), and
//   * the legacy marker-struct build (default) — the blanket `starts_with("__")`
//     introspection filters were narrowed to the EXACT engine-reserved set
//     (`is_reserved_component_key`), so a user/framework `__`/`___` member is no
//     longer wrongly hidden. This is the C.4 blanket-filter deletion applied early
//     to the marker path (it unblocked the FW/1 AOP suite without the flyweight
//     flip). Engine bookkeeping keys (`__variables`/`__name`/…) stay hidden.
suiteBegin( "Component reserved-key (`__`/`___`) visibility consistency" );

probe = new oop.UnderscoreKeyProbe();
r = probe.probe();

// The value is ALWAYS stored and directly readable.
assert( "`___orig` is stored and directly readable", r.directRead, "STASHED" );
// A single leading underscore is a normal identifier on every engine.
assert( "single-underscore member is visible", r.singleVisible, true );

// A user `___`-prefixed public member is now VISIBLE across every introspection
// surface on both backings (Lucee parity), and the surfaces AGREE.
assert( "`___orig` visible via structKeyExists",   r.keyExists, true );
assert( "`___orig` visible via structKeyList",     r.inKeyList, true );
assert( "`___orig` visible via serializeJSON",     r.inJson,    true );
// Cross-surface consistency (the property the bug violated): all three agree.
assert( "keyExists agrees with keyList",           r.keyExists, r.inKeyList );
assert( "keyExists agrees with serializeJSON",     r.keyExists, r.inJson );

// Narrowing must NOT leak engine bookkeeping keys — `__variables` stays hidden
// on every surface (guards the exact reserved set against under-coverage).
assert( "engine key `__variables` hidden from structKeyExists", r.engineKeyHidden, true );
assert( "engine key `__variables` hidden from structKeyList",   r.engineNotInList, true );
assert( "engine key `__variables` hidden from serializeJSON",   r.engineNotInJson, true );

suiteEnd();
</cfscript>
