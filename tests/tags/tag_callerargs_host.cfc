<cfcomponent output="false">
	<!--- Calls tag_callerargs_probe.cfm from a function, three ways. Each returns
	      what the tag recorded plus the function's own arguments afterwards. --->

	<cffunction name="declared" access="public" returntype="struct" output="false">
		<cfargument name="objectid" default="fnOID">
		<cfargument name="typename" default="fnType">
		<cfargument name="stObj" default="#structNew()#">
		<cfset var out = "">
		<cfmodule template="tag_callerargs_probe.cfm">
		<cfset out = duplicate(request.tagCallerArgs)>
		<cfset out.fnObjectid = arguments.objectid>
		<cfset out.fnTypename = arguments.typename>
		<cfset out.fnStObjTouched = structKeyExists(arguments.stObj, "fromTag")>
		<cfreturn out>
	</cffunction>

	<cffunction name="undeclared" access="public" returntype="struct" output="false">
		<cfset var out = "">
		<cfmodule template="tag_callerargs_probe.cfm">
		<cfset out = duplicate(request.tagCallerArgs)>
		<cfset out.fnObjectid = arguments.objectid>
		<cfset out.fnTypename = arguments.typename>
		<cfset out.fnStObjTouched = structKeyExists(arguments.stObj, "fromTag")>
		<cfreturn out>
	</cffunction>

	<cffunction name="viaImport" access="public" returntype="struct" output="false">
		<cfargument name="objectid" default="fnOID">
		<cfargument name="typename" default="fnType">
		<cfargument name="stObj" default="#structNew()#">
		<cfset var out = "">
		<cfimport taglib="." prefix="tca">
		<tca:tag_callerargs_probe />
		<cfset out = duplicate(request.tagCallerArgs)>
		<cfset out.fnObjectid = arguments.objectid>
		<cfset out.fnTypename = arguments.typename>
		<cfset out.fnStObjTouched = structKeyExists(arguments.stObj, "fromTag")>
		<cfreturn out>
	</cffunction>

</cfcomponent>
