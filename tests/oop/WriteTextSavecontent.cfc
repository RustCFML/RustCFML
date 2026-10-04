<cfcomponent output="false">
	<cffunction name="s" output="false">
		<cfsavecontent variable="local.c">  A  <cfoutput>  B  </cfoutput>  C  </cfsavecontent>
		<cfreturn "[" & local.c & "]">
	</cffunction>
</cfcomponent>
