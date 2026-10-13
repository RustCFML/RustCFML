<!--- Included by tag_callerargs_probe.cfm: runs in the tag's scopes. --->
<cftry>
	<cfset r.incReadTypename = typename>
	<cfcatch><cfset r.incReadTypename = "THREW"></cfcatch>
</cftry>
<cfset fromInclude = "inc">
