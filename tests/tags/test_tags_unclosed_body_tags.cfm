<cfscript>
suiteBegin("Tags: an unclosed body tag is refused, not erased");

// The preprocessor used to return an EMPTY STRING for a body-bearing tag with no
// closing tag — the tag *and its whole body* vanished from the compiled output.
// A compile-time construct that quietly deleted code: `<cfsavecontent>` with a
// missing `</cfsavecontent>` dropped the content and never set the variable, and
// `<cfmail>` sent an empty message (docs known-issues §28).
//
// Lucee 7.0.4 refuses these at compile time with
// `No matching end tag found for tag [X]` — probed per tag, because NOT every
// body tag requires closing: `<cfhttp>`, `<cfexecute>`, `<cfmodule>` and
// `<cfthread>` all compile unclosed on Lucee, run attribute-only, and leave the
// body as page content (RustCFML already matched those, and still does).
// `<cfquery>` is the odd one: Lucee compiles it and fails at RUNTIME instead,
// because a cfquery with no body has no SQL.
unclosedTags = [ "cflock", "cfsilent", "cfstatic", "cftransaction", "cfoutput"
               , "cfsavecontent", "cfmail", "cfswitch" ];

for ( tag in unclosedTags ) {
	err = "";
	try {
		include "unclosed/#tag#.cfm";
	} catch ( any e ) {
		err = e.message ?: "";
	}
	assertTrue( "unclosed <#tag#> is refused with Lucee's wording",
		err contains "No matching end tag found for tag [#tag#]" );
}

// `<cfloop query=…>` reports as cfloop.
loopErr = "";
try {
	include "unclosed/cfloop_query.cfm";
} catch ( any e ) {
	loopErr = e.message ?: "";
}
assertTrue( "unclosed <cfloop query=> is refused", loopErr contains "No matching end tag found for tag [cfloop]" );

// cfquery: Lucee's runtime complaint about the missing SQL, NOT a compile error —
// so a template that merely contains the mistake still compiles. RustCFML used to
// erase the tag and emit its SQL text into the PAGE, leaking the query to the
// browser and running nothing.
queryErr = "";
try {
	include "unclosed/cfquery.cfm";
} catch ( any e ) {
	queryErr = e.message ?: "";
}
assertTrue( "unclosed <cfquery> reports the missing SQL",
	queryErr contains "define the SQL in the body of the tag" );

// The tags Lucee permits unclosed must keep compiling — this is the guard against
// "fix it by refusing everything".
okErr = "";
try {
	include "unclosed_ok/cfhttp_unclosed.cfm";
} catch ( any e ) {
	okErr = e.message ?: "";
}
assert( "an unclosed <cfhttp> still compiles (Lucee permits it)", okErr, "" );

suiteEnd();
</cfscript>
