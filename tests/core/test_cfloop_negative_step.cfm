<cfscript>
suiteBegin("CFLoop negative step");

descending = "";
</cfscript>
<cfloop from="3" to="1" step="-1" index="i">
    <cfset descending = descending & i />
</cfloop>
<cfscript>
ascending = "";
</cfscript>
<cfloop from="1" to="3" index="i">
    <cfset ascending = ascending & i />
</cfloop>
<cfscript>
assert("counted cfloop supports negative step", descending & "|" & ascending, "321|123");

suiteEnd();
</cfscript>
