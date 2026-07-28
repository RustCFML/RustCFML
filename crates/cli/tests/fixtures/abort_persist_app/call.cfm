<cftry>
	<cfoutput>nested=[#application.holder.nested.greet()#] top=[#application.topLevel.greet()#]</cfoutput>
	<cfcatch type="any"><cfoutput>CAUGHT: #cfcatch.message#</cfoutput></cfcatch>
</cftry>
