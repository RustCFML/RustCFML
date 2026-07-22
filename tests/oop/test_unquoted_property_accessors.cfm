<cfscript>
// UNQUOTED property attribute syntax must parse identically to the quoted form
// (Lucee/ACF parity). This unblocked the FW/1 DI stubs, whose properties are all
// declared unquoted (`property name = username getters = true ...;`). Before the
// parser fix, the declared properties were dropped and NO implicit accessors were
// generated — `getUserName()`/`getColour()` threw "has no function".
suiteBegin( "Unquoted `property` attribute syntax" );

p = new oop.UnquotedPropertyProbe();

// getMetadata().properties must list the DECLARED properties by name/type — not a
// mis-parsed `name` token, and not application-scope keys.
md = getMetadata( p );
propNames = "";
propByName = {};
if ( structKeyExists( md, "properties" ) ) {
    for ( pr in md.properties ) {
        propNames = listAppend( propNames, pr.name );
        propByName[ pr.name ] = pr;
    }
}
assert( "declared property `colour` is present", listFindNoCase( propNames, "colour" ) GT 0, true );
assert( "declared property `size` is present",   listFindNoCase( propNames, "size" )   GT 0, true );
assert( "declared property `label` is present",  listFindNoCase( propNames, "label" )  GT 0, true );
assert( "the mis-parsed `name` token is NOT a property", listFindNoCase( propNames, "name" ), 0 );
assert( "property `colour` keeps its declared type", propByName.colour.type, "string" );
assert( "property `size` keeps its declared type",   propByName.size.type,   "numeric" );

// Implicit getters must work on a FRESH instance (value seeded in `variables` by
// init, never via the setter) — this is exactly what FW/1 relied on.
assert( "getColour() reads variables-seeded value", p.getColour(), "red" );
assert( "getSize() reads variables-seeded value",   p.getSize(),   5 );

// And the implicit setter round-trips.
p.setColour( "blue" );
assert( "setColour() then getColour()", p.getColour(), "blue" );

suiteEnd();
</cfscript>
