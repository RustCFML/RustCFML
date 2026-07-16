<cfcomponent output="false">
	<!--- Helper for test_qoq_component_variables_scope.cfm. Mirrors Masa's
	      pluginManager.loadPlugins: build a query into the component `variables`
	      scope via QoQ, then read it back with a second QoQ. --->
	<cffunction name="build" output="false" returntype="query">
		<cfset variables.src1 = queryNew("id,val", "integer,varchar", [[1, "x"], [2, "y"]])>
		<cfset variables.src2 = queryNew("id,val", "integer,varchar", [[3, "z"]])>

		<cfquery name="variables.merged" dbtype="query">
		select * from src1 union select * from src2
		</cfquery>

		<cfquery name="variables.result" dbtype="query">
		select * from merged order by id
		</cfquery>

		<cfreturn variables.result>
	</cffunction>

	<!--- Mirrors Masa's admin framework.cfc: a query is passed INTO a method as an
	      argument and read back via QoQ using an `arguments.rsX` source-table name.
	      find_query_in_scope had no `arguments` case, so QoQ raised
	      "table 'arguments.rsplugins' not found". --->
	<cffunction name="buildFromArgs" output="false" returntype="query">
		<cfargument name="rsplugins" type="query" required="true">
		<cfquery name="local.filtered" dbtype="query">
		select id, val from arguments.rsplugins where id >= 2 order by id
		</cfquery>
		<cfreturn local.filtered>
	</cffunction>
</cfcomponent>
