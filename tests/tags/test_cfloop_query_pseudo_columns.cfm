<cfscript>
suiteBegin("cfloop query: bare currentRow / recordCount / columnList");
</cfscript>

<cfset pq = queryNew( "a,b", "varchar,integer", [ [ "x", 1 ], [ "y", 2 ], [ "z", 3 ] ] )>
<cfset seen = "">
<cfloop query="pq"><cfset seen &= "#a#:#currentRow#/#recordCount# "></cfloop>
<cfset assert("bare pseudo-columns track the loop", trim( seen ), "x:1/3 y:2/3 z:3/3")>

<cfset cols = "">
<cfloop query="pq"><cfset cols = columnList></cfloop>
<cfset assert("bare columnList", cols, "A,B")>

<cfset rowsSeen = []>
<cfloop query="pq"><cfset arrayAppend( rowsSeen, queryGetRow( pq, currentRow ).a )></cfloop>
<cfset assert("queryGetRow( q, currentRow ) — the common idiom", arrayToList( rowsSeen ), "x,y,z")>

<cffunction name="loopInFunction" returntype="string">
	<cfset var fq = queryNew( "v", "varchar", [ [ "p" ], [ "r" ] ] )>
	<cfset var out = "">
	<cfloop query="fq"><cfset out &= currentRow></cfloop>
	<cfreturn out>
</cffunction>
<cfset assert("inside a function", loopInFunction(), "12")>

<cfset windowed = "">
<cfloop query="pq" startrow="2"><cfset windowed &= currentRow></cfloop>
<cfset assert("with startrow, currentRow is the query row", windowed, "23")>

<cfscript>
suiteEnd();
</cfscript>
