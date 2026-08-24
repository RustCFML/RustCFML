<cfscript>
// Included by test_tags_cfscript_statements.cfm. The script-form `exit`
// statement must stop THIS template and hand control back to the includer —
// exactly as <cfexit> does. It used to parse as a bare `exit` identifier plus
// a `method=…` assignment, so execution simply carried on (GH #341 census).
request._exitProbe = "before";
exit method="exittemplate";
request._exitProbe = "after";
</cfscript>
