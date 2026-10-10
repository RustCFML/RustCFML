<cfscript>
// A cookie written in LONG form and then overwritten with a plain value in the
// same request (GH #486). What reaches the browser must leave it holding the
// LAST value — the long form is just a write with attributes, not a lock on the
// name.
cookie[ "gh486a" ] = { value = "first", httpOnly = true, path = "/" };
cookie[ "gh486a" ] = "second";

// Control: two plain writes, which always behaved.
cookie[ "gh486b" ] = "first";
cookie[ "gh486b" ] = "second";

// Deleting a long-form cookie must take its attributes with it, or a later
// plain write under the same name is rendered from the dead struct.
cookie[ "gh486c" ] = { value = "gone", httpOnly = true, path = "/" };
structDelete( cookie, "gh486c" );
cookie[ "gh486c" ] = "reset";

// A long form left alone still carries its attributes to the browser.
cookie[ "gh486d" ] = { value = "kept", httpOnly = true, path = "/deep" };

writeOutput( "gh486 set" );
</cfscript>
