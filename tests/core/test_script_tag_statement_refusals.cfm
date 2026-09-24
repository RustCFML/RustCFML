<cfscript>
// A CFML tag we do not implement must refuse IDENTICALLY in both syntaxes.
// Lucee accepts `tag attr="v";` in script for any tag, so that shape is real
// code — and it used to parse here as a bare identifier plus a run of
// assignments: no error, no effect, and the attribute names left behind as
// variables in the caller's scope (the GH ##355 class).
suiteBegin( "Script-form tag statements refuse like their tag form" );

// A representative spread of the unimplemented set: a Lucee-only tag, two ACF
// UI tags, a search tag and two whose names are ordinary words (the shapes most
// at risk of being parsed as an expression).
unimplemented = [ "ldap", "form", "input", "index", "select", "object", "wddx" ];

for( tag in unimplemented ) {
    threw = false;
    message = "";
    try {
        // Built at runtime so one unimplemented tag cannot stop the file
        // parsing — the statement form is a compile-time construct.
        evaluate( "__probeTagStatement( tag )" );
    } catch( any e ) {
        threw   = true;
        message = e.message;
    }
    assert( "#tag# statement form throws", threw, true );
    assert( "#tag# names the tag", message contains "cf#tag#", true );
    assert( "#tag# says not implemented", message contains "not implemented", true );
    // The failure that made this class invisible: the attributes were being
    // written into the caller's scope as ordinary variables.
    assert( "#tag# leaks no attribute variable", structKeyExists( variables, "__probeattr" ), false );
}

function __probeTagStatement( required string tag ) {
    switch( arguments.tag ) {
        case "ldap"  : ldap   __probeAttr="x"; break;
        case "form"  : form   __probeAttr="x"; break;
        case "input" : input  __probeAttr="x"; break;
        case "index" : index  __probeAttr="x"; break;
        case "select": select __probeAttr="x"; break;
        case "object": object __probeAttr="x"; break;
        case "wddx"  : wddx   __probeAttr="x"; break;
    }
}

// The `cf` prefix is accepted on the statement form too, and refuses the same way.
assertThrows( "cf-prefixed statement form refuses", function(){ cfldap __probeAttr="x"; } );

// ...and the tag form still refuses, so the two genuinely agree.
assertThrows( "tag form still refuses", function(){ return evaluate( "__probeTagForm()" ); } );
function __probeTagForm() {
    include "fixtures/unimplemented_tag.cfm";
}

// A tag we DO implement is untouched by the refusal list — the guard only fires
// for names that have no implementation at all.
result = "";
savecontent variable="result" { writeOutput( "implemented" ); }
assert( "an implemented tag statement still works", result, "implemented" );

// And an ordinary assignment whose NAME is one of those words is not hijacked:
// the refusal only fires on the `ident attr=` shape, never on `ident =`.
// (`form` itself is a scope, so `select` stands in for the general case.)
select = "not a tag here";
assert( "a variable named after a tag is still a variable", select, "not a tag here" );
index = 7;
assert( "...and can hold a number", index + 1, 8 );

suiteEnd();
</cfscript>
