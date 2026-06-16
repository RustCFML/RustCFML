<cfscript>
suiteBegin("Tags: escaped hash in attributes");

caughtMessage = "";
</cfscript>

<cftry>
	<cfthrow message="literal ##278" />
	<cfcatch>
		<cfset caughtMessage = cfcatch.message />
	</cfcatch>
</cftry>

<cfscript>
assert("escaped hash in a tag attribute stays literal after tag conversion",
	caughtMessage,
	"literal " & chr(35) & "278");

suiteEnd();
</cfscript>
