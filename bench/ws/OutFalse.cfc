<cfcomponent output="false">
	<cffunction name="f" output="false">
		LITERAL_TEXT
		<cfoutput>CFOUTPUT_TEXT</cfoutput>
		<cfset writeOutput("WRITEOUTPUT_TEXT")>
		<cfreturn "R">
	</cffunction>
	<cffunction name="g" output="true">
		LITERAL_G
		<cfreturn "G">
	</cffunction>
	<cffunction name="h">
		LITERAL_H
		<cfreturn "H">
	</cffunction>
</cfcomponent>
