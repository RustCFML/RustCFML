<cfscript>
// Target page for CFScript tag statement tests.
// Called via cfhttp from test_tags_cfscript_statements.cfm.
// Sets HTTP headers, cookies, and handles location redirect.

param name="url.test" default="";

switch (url.test) {
    case "header":
        header name="X-Test-Header" value="hello123";
        header statuscode="200" statustext="OK";
        writeOutput("header-ok");
        break;

    case "cookie":
        cookie name="testcookie" value="cookievalue";
        cookie name="securecookie" value="secret" httponly="true" secure="true";
        writeOutput("cookie-ok");
        break;

    case "location":
        location url="/redirect-target" statuscode="301";
        break;

    case "content":
        content type="application/json";
        writeOutput('{"status":"ok"}');
        break;

    case "echo":
        header name="X-Echo-Method" value=cgi.request_method ?: "";
        writeOutput("echo-ok");
        break;

    default:
        writeOutput("unknown-test");
}
</cfscript>
