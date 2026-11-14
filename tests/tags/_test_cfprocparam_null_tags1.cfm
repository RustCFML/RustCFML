<!--- Tag-form calls for test_cfprocparam_null.cfm, step 1: closing </cfscript> inside the suite's open if/try block is legal on RustCFML but a syntax error on Lucee. --->
<cfstoredproc procedure="#pnProc#" datasource="#pnDsn#">
    <cfprocparam cfsqltype="cf_sql_integer" value="#pnEmpty#" null="#( NOT len( pnEmpty ) )#">
    <cfprocparam cfsqltype="cf_sql_varchar" null="true">
    <cfprocresult name="tagNull">
</cfstoredproc>
<cfstoredproc procedure="#pnProc#" datasource="#pnDsn#">
    <cfprocparam cfsqltype="cf_sql_integer" value="5" null="false">
    <cfprocparam cfsqltype="cf_sql_varchar" value="x" null="no">
    <cfprocresult name="tagValue">
</cfstoredproc>
<cfstoredproc procedure="#pnProc#" datasource="#pnDsn#">
    <cfif true><cfprocparam cfsqltype="cf_sql_integer" value="" null="yes"></cfif>
    <cfprocparam cfsqltype="cf_sql_varchar" null="1">
    <cfprocresult name="runtimeNull">
</cfstoredproc>
