/**
 * Fixture for the `__`/`___`-prefix `this`-scope visibility divergence
 * (C2planDoc.md §1). FW/1 AOP stashes the original method under a `___`-prefixed
 * key (`this["___doReverse"]`); Lucee treats `__`/`___` as ordinary identifiers
 * and shows them in structKeyExists/structKeyList/serializeJSON, while the legacy
 * RustCFML marker path HID any `__`-prefixed key. The Phase C.3 flyweight fixes
 * this by construction (the data maps carry no engine reserved keys, so nothing
 * is filtered). See tests/oop/test_reserved_key_visibility.cfm.
 */
component {
    // Set a `___`-prefixed member (the FW/1 AOP shape), a single-underscore member
    // (always visible on both engines), and a plain member. Returns a struct of
    // the introspection observations so the test can assert cross-surface
    // CONSISTENCY without depending on which backing is active.
    public struct function probe() {
        this[ "___orig" ] = "STASHED";
        this[ "_single" ] = "S1";
        this[ "plain" ]   = "P";
        return {
              directRead = this[ "___orig" ]
            , keyExists  = structKeyExists( this, "___orig" )
            , inKeyList  = listFindNoCase( structKeyList( this ), "___orig" ) GT 0
            , inJson     = findNoCase( "___orig", serializeJSON( this ) ) GT 0
            , singleVisible = structKeyExists( this, "_single" )
            // Engine bookkeeping keys MUST stay hidden (narrowing must not leak
            // reserved keys): `__variables` is an engine sentinel, never a user key.
            , engineKeyHidden  = NOT structKeyExists( this, "__variables" )
            , engineNotInList  = listFindNoCase( structKeyList( this ), "__variables" ) EQ 0
            , engineNotInJson  = findNoCase( """__variables""", serializeJSON( this ) ) EQ 0
        };
    }
}
