<cfcomponent output="false" accessors="true">
	<!--- accessors="true": getX/setX ARE synthesized and win over onMissingMethod
	      for a declared property. Guards the fix from over-correcting — a real
	      accessors component must still get generated accessors, not route to OMM. --->
	<cfproperty name="table" type="string" default="">

	<cffunction name="onMissingMethod" output="false">
		<cfargument name="missingMethodName">
		<cfargument name="missingMethodArguments">
		<cfthrow message="onMissingMethod should NOT be reached for a declared property when accessors=true (called: #arguments.missingMethodName#)">
	</cffunction>
</cfcomponent>
