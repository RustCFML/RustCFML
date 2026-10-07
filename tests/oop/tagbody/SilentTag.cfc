<cfcomponent output="false" hint="a tag component that must stay silent">

	SILENT_PSEUDO_CONSTRUCTOR_TEXT

	<cffunction name="init" access="public" returntype="any" output="false" hint="constructor">
		<cfargument name="label" required="true" hint="the label">
		<cfset variables.label = arguments.label>
		<cfset variables.items = []>
		<cfreturn this>
	</cffunction>

	<cffunction name="getLabel" access="public" returntype="string" output="false" hint="returns the label">
		<cfreturn variables.label>
	</cffunction>

	<cffunction name="addItem" access="public" returntype="any" output="false" hint="adds one item">
		<cfargument name="item" required="true">
		<cfset arrayAppend( variables.items, arguments.item )>
		<cfreturn this>
	</cffunction>

	<cffunction name="itemCount" access="public" returntype="numeric" output="false" myannotation="hello">
		<cfreturn arrayLen( variables.items )>
	</cffunction>

</cfcomponent>
