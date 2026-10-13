<cfif thistag.executionMode eq "start">
	<!--- Invoked from a function by tag_callerargs_host.cfc. Records how that
	      function's arguments scope behaves inside the tag; the test asserts on
	      request.tagCallerArgs. objectid, typename and stObj are names the
	      calling function has as arguments. --->
	<cfset r = {}>
	<cftry>
		<cfset r.readBefore = objectid>
		<cfcatch><cfset r.readBefore = "THREW"></cfcatch>
	</cftry>
	<cfset r.argBefore = arguments.objectid>
	<cfset ObjectID = "fromTag">
	<cfset typename = "typeFromTag">
	<cfset stObj.fromTag = true>
	<cfset r.readAfter = objectid>
	<cfset r.varsHasObjectid = structKeyExists(variables, "objectid")>
	<cfset r.varsTypename = structKeyExists(variables, "typename") ? variables.typename : "(none)">
	<cfset r.varsHasStObj = structKeyExists(variables, "stObj")>
	<cfset r.argAfter = arguments.objectid>
	<cfinclude template="tag_callerargs_inc.cfm">
	<cfset r.varsFromInclude = structKeyExists(variables, "fromInclude") ? variables.fromInclude : "(none)">
	<cfset request.tagCallerArgs = r>
</cfif>
