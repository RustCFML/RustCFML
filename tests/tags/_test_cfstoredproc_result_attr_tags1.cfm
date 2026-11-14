<!--- Tag-form calls for test_cfstoredproc_result_attr.cfm, step 1: closing </cfscript> inside the suite's open if/try block is legal on RustCFML but a syntax error on Lucee. --->
<cfstoredproc procedure="#raProc#" datasource="#raDsn#" result="tagResult" returnCode="true">
    <cfprocparam cfsqltype="cf_sql_integer" value="1">
    <cfprocparam type="out" cfsqltype="cf_sql_integer" variable="tagB">
    <cfprocresult name="tagRows">
</cfstoredproc>
<cfstoredproc procedure="#raProc#" datasource="#raDsn#" result="raHolder.sproc">
    <cfprocparam cfsqltype="cf_sql_integer" value="2">
    <cfprocparam type="out" cfsqltype="cf_sql_integer" variable="plainB">
</cfstoredproc>
