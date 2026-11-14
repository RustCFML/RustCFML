<!--- Tag-form <cfstoredproc> calls for test_cfstoredproc_result_sets.cfm, kept
      in their own template: closing </cfscript> inside the suite's open
      if/try block is legal on RustCFML but a syntax error on Lucee. Reads
      rsSets, rsDsn and rsOne from the including page. --->
<cfstoredproc procedure="#rsSets#" datasource="#rsDsn#">
    <cfprocparam cfsqltype="cf_sql_integer" value="7">
    <cfprocresult name="tagSecond" resultSet="2">
    <cfprocresult name="tagFirst">
    <cfprocresult name="tagThird" resultSet="3">
    <cfprocresult name="tagMissing" resultSet="4">
</cfstoredproc>
<cfstoredproc procedure="#rsSets#" datasource="#rsDsn#">
    <cfprocparam cfsqltype="cf_sql_integer" value="7">
    <cfprocresult name="tagCapped" resultSet="2" maxRows="#rsOne#">
    <cfprocresult name="tagUncapped" resultSet="2" maxRows="-1">
</cfstoredproc>
<cfstoredproc procedure="#rsSets#" datasource="#rsDsn#">
    <cfprocparam cfsqltype="cf_sql_integer" value="7">
    <cfprocresult name="tagNone" resultSet="2" maxRows="0">
</cfstoredproc>
