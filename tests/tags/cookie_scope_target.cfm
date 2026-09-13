<cfscript>
// Target for test_cookie_scope_write_sets_header.cfm (GH #423). Sets BOTH the
// scalar and the struct form, so the caller can assert each independently —
// cfhttp surfaces repeated Set-Cookie headers as an array since GH #424.
// Any cookie arriving on the request is deliberately left alone, so the caller
// can also assert that untouched cookies are not echoed back.
cookie.scopescalar = "scalarvalue";
cookie.scopestruct = {value="structvalue", expires="never", httponly=true, path="/"};
writeOutput("ok");
</cfscript>
