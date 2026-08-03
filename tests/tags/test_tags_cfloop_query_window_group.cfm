<cfscript>
suiteBegin("Tags: cfloop query startrow/endrow/maxrows/group");

// `<cfloop query=>` lowered with only `query` + `index`/`item` honoured:
// startrow=, endrow=, maxrows= and group= were discarded at compile time, so
// EVERY row was iterated and a grouped loop with a bare inner <cfloop> was a
// hard error (docs known-issues §27). Expectations probed against Lucee 7.0.4.
qw = queryNew( "dept,nm", "varchar,varchar", [
	  [ "sales", "ann" ], [ "sales", "bob" ], [ "eng", "cat" ]
	, [ "eng", "dan" ], [ "ENG", "eve" ], [ "hr", "fay" ]
] );
</cfscript>

<cfsavecontent variable="lqPlain"><cfloop query="qw"><cfoutput>|#qw.nm#:#qw.currentRow#/#qw.recordCount#</cfoutput></cfloop></cfsavecontent>
<cfsavecontent variable="lqStart"><cfloop query="qw" startrow="3"><cfoutput>|#qw.nm#</cfoutput></cfloop></cfsavecontent>
<cfsavecontent variable="lqEnd"><cfloop query="qw" endrow="2"><cfoutput>|#qw.nm#</cfoutput></cfloop></cfsavecontent>
<cfsavecontent variable="lqBoth"><cfloop query="qw" startrow="2" endrow="4"><cfoutput>|#qw.nm#</cfoutput></cfloop></cfsavecontent>
<cfsavecontent variable="lqOob"><cfloop query="qw" startrow="9" endrow="20"><cfoutput>|#qw.nm#</cfoutput></cfloop></cfsavecontent>
<cfsavecontent variable="lqMax"><cfloop query="qw" maxrows="2"><cfoutput>|#qw.nm#</cfoutput></cfloop></cfsavecontent>
<cfsavecontent variable="lqGroup"><cfloop query="qw" group="dept"><cfoutput>[#qw.dept#</cfoutput><cfloop><cfoutput>-#qw.nm#</cfoutput></cfloop><cfoutput>]</cfoutput></cfloop></cfsavecontent>
<cfsavecontent variable="lqGroupFlat"><cfloop query="qw" group="dept"><cfoutput>[#qw.dept#:#qw.nm#]</cfoutput></cfloop></cfsavecontent>
<cfsavecontent variable="lqGroupCS"><cfloop query="qw" group="dept" groupcasesensitive="true"><cfoutput>[#qw.dept#</cfoutput><cfloop><cfoutput>-#qw.nm#</cfoutput></cfloop><cfoutput>]</cfoutput></cfloop></cfsavecontent>
<cfsavecontent variable="lqGroupStart"><cfloop query="qw" group="dept" startrow="3"><cfoutput>[#qw.dept#</cfoutput><cfloop><cfoutput>-#qw.nm#</cfoutput></cfloop><cfoutput>]</cfoutput></cfloop></cfsavecontent>
<cfsavecontent variable="lqOutGroup"><cfoutput query="qw" group="dept">[#qw.dept#<cfoutput>-#qw.nm#</cfoutput>]</cfoutput></cfsavecontent>

<cfscript>
assert( "cfloop query all rows", lqPlain, "|ann:1/6|bob:2/6|cat:3/6|dan:4/6|eve:5/6|fay:6/6" );
assert( "cfloop query startrow=3", lqStart, "|cat|dan|eve|fay" );
assert( "cfloop query endrow=2", lqEnd, "|ann|bob" );
assert( "cfloop query startrow=2 endrow=4", lqBoth, "|bob|cat|dan" );
// A window past the end of the recordset yields nothing, and does not error.
assert( "cfloop query window past the end", lqOob, "" );
assert( "cfloop query maxrows=2", lqMax, "|ann|bob" );
// group= breaks on consecutive values; the bare nested <cfloop> is the detail
// block over the current group. Case-INsensitive by default, so eng/ENG merge.
assert( "cfloop query group with detail block", lqGroup, "[sales-ann-bob][eng-cat-dan-eve][hr-fay]" );
assert( "cfloop query group without detail block", lqGroupFlat, "[sales:ann][eng:cat][hr:fay]" );
assert( "cfloop query groupCaseSensitive=true splits case", lqGroupCS, "[sales-ann-bob][eng-cat-dan][ENG-eve][hr-fay]" );
assert( "cfloop query group applies after startrow", lqGroupStart, "[eng-cat-dan-eve][hr-fay]" );
// The same default governs cfoutput's grouping.
assert( "cfoutput query group default is case-insensitive", lqOutGroup, "[sales-ann-bob][eng-cat-dan-eve][hr-fay]" );
// The query variable is left as the query, cursor rewound.
assertTrue( "cfloop query leaves the query intact", isQuery( qw ) );
assert( "cfloop query rewinds the cursor", qw.currentRow, 1 );
assert( "cfloop query recordcount unchanged", qw.recordcount, 6 );

suiteEnd();
</cfscript>
