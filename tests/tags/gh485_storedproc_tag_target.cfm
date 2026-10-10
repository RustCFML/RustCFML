<!--- The TAG form of the same call, for the script form to be compared against
      (GH 485). Kept in its own file because the comparison is made from inside
      a <cfscript> block. The caller passes the datasource in request scope and
      catches the error the statement raises. --->
<cfstoredproc procedure="gh485Two" datasource="#request._gh485_tagDs#">
	<cfprocparam cfsqltype="char" value="A">
	<cfprocparam cfsqltype="char" value="B">
</cfstoredproc>
