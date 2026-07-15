<cfcomponent output="false">
	<!--- Fixture for test_cfinvoke_argument_children.cfm. Mirrors Mura/Masa's
	      event dispatch: a handler whose scope object is an argument literally
	      named `$`, invoked via <cfinvoke> with <cfinvokeargument> children. --->
	<cffunction name="onEvent" output="false" returntype="string">
		<cfargument name="event">
		<cfargument name="$">
		<cfargument name="mura">
		<cfreturn arguments.event & "|" & arguments["$"].name & "|" & arguments.mura.name>
	</cffunction>
</cfcomponent>
