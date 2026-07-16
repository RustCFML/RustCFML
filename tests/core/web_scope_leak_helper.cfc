<cfcomponent output="false">
	<!--- A scoped write to a web request scope (url/form/cgi/cookie) from inside
	      a CFC method must land in the LIVE request scope (self.globals), never
	      the component's `variables` scope. For an application-scoped singleton
	      the latter would persist across requests — the Masa CMS front-controller
	      infinite redirect loop (contentServer.parseURLRoot accumulated url.path
	      on the singleton). --->

	<cffunction name="writeUrl" output="false">
		<cfargument name="key">
		<cfargument name="value">
		<cfset url[arguments.key] = arguments.value>
	</cffunction>

	<cffunction name="writeUrlDotted" output="false">
		<cfargument name="value">
		<cfset url.path = arguments.value>
	</cffunction>

	<cffunction name="readUrl" output="false">
		<cfargument name="key">
		<cfreturn isDefined("url." & arguments.key) ? url[arguments.key] : "(undef)">
	</cffunction>

	<cffunction name="urlKeyLeakedIntoVariables" output="false" returntype="boolean">
		<!--- The bare `url` scope must NOT have been vivified onto the component
		      variables scope by the scoped write. --->
		<cfreturn structKeyExists(variables, "url")>
	</cffunction>
</cfcomponent>
