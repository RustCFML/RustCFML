<!--- Tag-form calls for test_cfprocparam_out.cfm, step 1: closing </cfscript> inside the suite's open if/try block is legal on RustCFML but a syntax error on Lucee. --->
<cfstoredproc procedure="#poProc#" datasource="#poDsn#">
    <cfprocparam cfsqltype="cf_sql_integer" value="5">
    <cfprocparam type="out" cfsqltype="cf_sql_integer" variable="tagDoubled">
    <cfprocparam type="inout" cfsqltype="cf_sql_integer" value="41" variable="tagCounter">
    <cfprocparam type="out" cfsqltype="cf_sql_varchar" variable="tagLabel">
    <cfprocparam type="out" cfsqltype="cf_sql_integer" variable="tagNothing">
    <cfprocresult name="tagRows">
</cfstoredproc>
<cfstoredproc procedure="#poProc#" datasource="#poDsn#">
    <cfif true><cfprocparam cfsqltype="cf_sql_integer" value="7"></cfif>
    <cfprocparam type="OUT" cfsqltype="cf_sql_integer" variable="runtimeDoubled">
    <cfprocparam type="inout" cfsqltype="cf_sql_integer" value="1" variable="runtimeCounter">
    <cfprocparam type="out" cfsqltype="cf_sql_varchar" variable="runtimeLabel">
    <cfprocparam type="out" cfsqltype="cf_sql_integer" variable="runtimeNothing">
</cfstoredproc>
