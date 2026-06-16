<cfscript>
suiteBegin("Tags: cfloop list literal");

seen = [];
</cfscript>

<cfloop list="yyyy-MM-dd HH:mm:ss,yyyy-MM-ddTHH:mm:ss,plain" index="pattern">
	<cfset arrayAppend(seen, pattern) />
</cfloop>

<cfscript>
assert("cfloop list literal is passed to listToArray as a string",
	arrayToList(seen, "|"),
	"yyyy-MM-dd HH:mm:ss|yyyy-MM-ddTHH:mm:ss|plain");

suiteEnd();
</cfscript>
