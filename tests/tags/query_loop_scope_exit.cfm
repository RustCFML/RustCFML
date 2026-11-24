<!--- Custom tag for test_query_loop_scope.cfm (GH #544): leaves the tag with
      cfexit from inside a query loop, which skips the loop's own clean-up. --->
<cfif thisTag.executionMode EQ "start">
	<cfset exitQuery = queryNew( "exitcol", "varchar", [ [ "a" ] ] )>
	<cfloop query="exitQuery"><cfexit method="exitTag"></cfloop>
</cfif>
