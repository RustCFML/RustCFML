<cfscript>
// Target for test_cookie_scope_write_sets_header.cfm (GH #423). ONE cookie
// only, deliberately: cfhttp exposes just one entry per header name, so a
// second Set-Cookie would be invisible to the caller and the assertion would
// fail for a reason that has nothing to do with the cookie scope.
// The struct form is the one set here because it carries the richer attribute
// path (value + expires + httponly + path); the scalar form is asserted
// in-scope by the caller.
cookie.scopestruct = {value="structvalue", expires="never", httponly=true, path="/"};
writeOutput("ok");
</cfscript>
