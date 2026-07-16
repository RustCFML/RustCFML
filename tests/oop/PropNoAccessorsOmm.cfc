<cfcomponent output="false">
	<!--- NO accessors="true": declaring <cfproperty> alone must NOT synthesize
	      getX/setX. A component that defines onMissingMethod expects getX/setX to
	      route there (Lucee/ACF parity). This is the exact shape of Masa CMS's
	      mura.bean.bean: setTable()/getTable() are handled by onMissingMethod and
	      map onto variables.instance.table. The lenient implicit accessor used to
	      intercept them and write the wrong scope, leaving the value empty. --->
	<cfproperty name="table" type="string" default="">

	<cfset variables.instance = {}>
	<cfset variables.instance.table = "">

	<cffunction name="setValue" output="false">
		<cfargument name="property">
		<cfargument name="propertyValue">
		<cfset variables.instance[arguments.property] = arguments.propertyValue>
		<cfreturn this>
	</cffunction>

	<cffunction name="readInstanceTable" output="false">
		<cfreturn variables.instance.table>
	</cffunction>

	<cffunction name="ommHits" output="false">
		<cfreturn variables.instance.keyExists("__ommCount") ? variables.instance.__ommCount : 0>
	</cffunction>

	<cffunction name="onMissingMethod" output="false">
		<cfargument name="missingMethodName">
		<cfargument name="missingMethodArguments">
		<cfset variables.instance.__ommCount = (variables.instance.keyExists("__ommCount") ? variables.instance.__ommCount : 0) + 1>
		<cfset var prefix = left(arguments.missingMethodName, 3)>
		<cfset var prop = right(arguments.missingMethodName, len(arguments.missingMethodName) - 3)>
		<cfif prefix eq "get">
			<cfreturn variables.instance.keyExists(prop) ? variables.instance[prop] : "">
		</cfif>
		<cfif not structIsEmpty(arguments.missingMethodArguments)>
			<cfreturn setValue(prop, arguments.missingMethodArguments[1])>
		</cfif>
		<cfreturn this>
	</cffunction>
</cfcomponent>
