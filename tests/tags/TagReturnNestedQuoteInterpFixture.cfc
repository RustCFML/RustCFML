<cfcomponent>
	<!--- GH #257: tag-context expression with a single-quoted string whose
	      #interpolation# itself contains nested single-quoted strings. This is
	      TestBox 2.8's MockGenerator.cfc:225 pattern. --->
	<cffunction name="q" output="false">
		<cfargument name="value">
		<cfreturn '"#replaceNoCase( value, '"', '""', 'all' )#"'>
	</cffunction>

	<!--- Same shape via <cfset> to exercise the other freeform-body tag. --->
	<cffunction name="viaSet" output="false">
		<cfargument name="value">
		<cfset var out = '"#replaceNoCase( value, '"', '""', 'all' )#"'>
		<cfreturn out>
	</cffunction>
</cfcomponent>
