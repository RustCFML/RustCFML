<cfcomponent hint="Fixture for test_fn_type_enforcement.cfm (docs/known-issues.md §29)">

	<!--- Tag-declared argument/return types must be enforced exactly like
	      script-declared ones. --->
	<cffunction name="tagArg" returntype="any">
		<cfargument name="n" type="numeric" required="true">
		<cfreturn arguments.n>
	</cffunction>

	<cffunction name="tagRet" returntype="numeric">
		<cfreturn "abc">
	</cffunction>

	<cffunction name="tagRetOk" returntype="numeric">
		<cfreturn "42">
	</cffunction>

	<!--- A component-typed argument: satisfied by an instance of this CFC,
	      not by a plain struct. --->
	<cffunction name="componentArg" returntype="string">
		<cfargument name="c" type="TypeEnforcementFixture" required="true">
		<cfreturn "accepted">
	</cffunction>

</cfcomponent>
