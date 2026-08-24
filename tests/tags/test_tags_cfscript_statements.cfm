<cfscript>
suiteBegin("CFScript Tag Statements");

// ============================================================
// setting (cfsetting) — safe, no HTTP side-effects
// ============================================================

setting requesttimeout="60";
assertTrue("setting requesttimeout parsed", true);

setting showdebugoutput="false";
assertTrue("setting showdebugoutput parsed", true);

// ============================================================
// log (cflog) — safe, no HTTP side-effects
// ============================================================

log text="Test log message" type="information";
assertTrue("log text/type parsed", true);

log text="Debug message" type="debug" file="testlog";
assertTrue("log with file parsed", true);

// Hash interpolation inside quoted log attributes. CFML supports the bare
// `log text=...` statement form and the parenthesized `cflog(...)` call form;
// both interpolate #expr# in quoted attributes. (The cf-prefixed bare form
// `cflog text=...` is NOT valid CFScript on Lucee, so it is not tested here.)
logMessage = "dynamic";
logBareError = "";
try {
    log text="cfml_literal_#logMessage#" type="information";
} catch (any e) {
    logBareError = e.message;
}
assert("log statement hash interpolation in quoted attr", logBareError, "");

logCallError = "";
try {
    cflog(text="cfml_literal_#logMessage#", type="information");
} catch (any e) {
    logCallError = e.message;
}
assert("cflog() call hash interpolation in quoted attr", logCallError, "");

// ============================================================
// thread (cfthread) — run/join/terminate actions
// ============================================================

// thread run with body
thread name="testThread1" action="run" {
    // Thread body - just needs to parse and run
    var x = 42;
}
thread name="testThread1" action="join" timeout="5";
assertTrue("thread run/join parsed", true);

// thread with default action (run)
thread name="testThread2" {
    var y = 100;
}
thread name="testThread2" action="join";
assertTrue("thread default action parsed", true);

// ============================================================
// HTTP-affecting statements (header, cookie, content, location)
// Tested via cfhttp to a target page so headers don't bleed
// into the test runner's own HTTP response.
//
// These tests require RustCFML to be running as a web server so the
// target page is reachable. We discover the live port from cgi.server_port
// (set by the server at request-time) and skip the HTTP subtests when the
// runner is invoked from the CLI with no server available.
// ============================================================

serverPort = structKeyExists(cgi, "server_port") ? trim(cgi.server_port) : "";
if (serverPort == "" || serverPort == "0") {
    writeOutput(chr(10) & "  skipped HTTP subtests (no cgi.server_port — run via rustcfml --serve)" & chr(10));
} else {
    baseUrl = "http://127.0.0.1:" & serverPort;
    targetPath = "/tests/tags/http_statements_target.cfm";

    // --- header ---
    http url=baseUrl & targetPath & "?test=header" method="GET" result="headerResult";
    assert("header target responds", headerResult.statuscode, "200 OK");
    assert("header body", trim(headerResult.filecontent), "header-ok");
    assertTrue("header X-Test-Header set",
        structKeyExists(headerResult.responseheader, "X-Test-Header")
        && headerResult.responseheader["X-Test-Header"] == "hello123");

    // --- header via parenthesized call form with direct named args (issue #141) ---
    http url=baseUrl & targetPath & "?test=header-named" method="GET" result="headerNamedResult";
    assert("header-named target responds", headerNamedResult.statuscode, "200 OK");
    assert("header-named body", trim(headerNamedResult.filecontent), "header-named-ok");
    assertTrue("cfheader(name=,value=) named-arg form sets header",
        structKeyExists(headerNamedResult.responseheader, "X-Script-Named")
        && headerNamedResult.responseheader["X-Script-Named"] == "snamed");
    assertTrue("cfheader(attributeCollection=) form sets header",
        structKeyExists(headerNamedResult.responseheader, "X-Script-AC")
        && headerNamedResult.responseheader["X-Script-AC"] == "sac");

    // --- cookie ---
    http url=baseUrl & targetPath & "?test=cookie" method="GET" result="cookieResult";
    assert("cookie target responds", cookieResult.statuscode, "200 OK");
    assert("cookie body", trim(cookieResult.filecontent), "cookie-ok");

    // --- content type ---
    http url=baseUrl & targetPath & "?test=content" method="GET" result="contentResult";
    assert("content target responds", contentResult.statuscode, "200 OK");
    assert("content body", trim(contentResult.filecontent), '{"status":"ok"}');
    assertTrue("content type is json",
        findNoCase("application/json", contentResult.responseheader["Content-Type"]) > 0);

    // --- content type via cfheader (issue #148: must REPLACE, not append) ---
    http url=baseUrl & targetPath & "?test=content-header" method="GET" result="ctHeaderResult";
    assert("content-header target responds", ctHeaderResult.statuscode, "200 OK");
    assert("content-header body", trim(ctHeaderResult.filecontent), '{"ok":1}');
    ctVal = ctHeaderResult.responseheader["Content-Type"];
    // A duplicate would surface as an array or a comma-joined string containing
    // the engine default; a correct singleton is just the cfheader value.
    assertTrue("cfheader Content-Type is a single (not duplicated) value",
        isSimpleValue(ctVal));
    assertTrue("cfheader Content-Type is the json type, not the html default",
        findNoCase("application/json", ctVal) > 0 && findNoCase("text/html", ctVal) == 0);

    // --- location (redirect) ---
    http url=baseUrl & targetPath & "?test=location" method="GET" redirect="false" result="locResult";
    assertTrue("location returns 3xx",
        left(locResult.statuscode, 1) == "3");
    assertTrue("location header set",
        structKeyExists(locResult.responseheader, "Location")
        && findNoCase("redirect-target", locResult.responseheader["Location"]) > 0);
}

// ============================================================
// throw — keyword statement, attribute statement, and call forms.
// All forms must populate cfcatch.message/.type (Lucee/ACF/BoxLang parity).
// ============================================================

// Bare keyword form: message is the string, type defaults to Application.
try {
    throw "bare message";
    assertTrue("throw bare should not reach here", false);
} catch (any e) {
    assert("throw bare string message", e.message, "bare message");
    assert("throw bare string default type", e.type, "Application");
}

// NB: the bare attribute statement form `throw message=... type=...;` is NOT
// portable — Lucee/ACF/BoxLang reject it (throw is a reserved keyword). Use the
// parenthesized call form below instead.

// Parenthesized call form (named): regression guard + portable attribute form.
try {
    throw(message="call msg", type="CallType");
    assertTrue("throw() should not reach here", false);
} catch (any e) {
    assert("throw() named message", e.message, "call msg");
    assert("throw() named type", e.type, "CallType");
}

// ============================================================
// cfparam — statement form, cf-prefixed call form, and type validation
// must stay in sync.
// ============================================================

param name="pStmt" default="stmt-default";
assert("param statement default applied", pStmt, "stmt-default");

cfparam(name="pCall", default="call-default");
assert("cfparam() call default applied", pCall, "call-default");

pExisting = "kept";
cfparam(name="pExisting", default="ignored");
assert("cfparam() preserves existing value", pExisting, "kept");

cfparam(name="pNum", default="42", type="numeric");
assert("cfparam() typed numeric default", pNum, "42");

threw = false;
try {
    cfparam(name="pBad", default="notnum", type="numeric");
} catch (any e) {
    threw = true;
}
assertTrue("cfparam() type validation rejects bad default", threw);

// ============================================================
// dump — both the bare statement (`dump var=...`, where `var` is also a
// keyword) and the cf-prefixed call form must parse and run without error.
// Output is captured/discarded; we only assert no exception is thrown.
// ============================================================

savecontent variable="__dumpSink" {
    dump var="dump statement form";
    cfdump(var="dump call form");
}
assertTrue("dump statement + cfdump() call forms run", len(__dumpSink) >= 0);

// ============================================================
// execute / cfexecute — GH #341 (function form threw) and GH #355 (the
// statement form was a SILENT no-op: nothing spawned, nothing written, nothing
// raised). Both spellings must reach the same intercept the tag form uses.
// /bin/echo is present on both CI targets; Windows would need a different
// binary, hence the guard.
// ============================================================

if (fileExists("/bin/echo")) {
    execute name="/bin/echo" arguments="stmt-form" timeout=10 variable="execStmtOut";
    assert("execute statement form writes its variable", trim(execStmtOut), "stmt-form");

    cfexecute(name="/bin/echo", arguments="fn-form", timeout=10, variable="execFnOut");
    assert("cfexecute() call form writes its variable", trim(execFnOut), "fn-form");

    // The written value is raw stdout, trailing newline and all.
    execute name="/bin/echo" arguments="raw" timeout=10 variable="execRaw";
    assert("execute keeps the trailing newline", execRaw, "raw" & chr(10));

    // A scoped target resolves; it is not a variable literally named "local.x".
    (function() {
        execute name="/bin/echo" arguments="scoped" timeout=10 variable="local.eo";
        assert("execute writes a scoped target", trim(local.eo), "scoped");
    })();

    // errorVariable takes stderr while variable takes stdout.
    if (fileExists("/bin/sh")) {
        execute name="/bin/sh" arguments='-c "echo to-err 1>&2"' timeout=10
                variable="execOut2" errorVariable="execErr2";
        assert("execute variable is stdout only", trim(execOut2), "");
        assert("execute errorVariable is stderr", trim(execErr2), "to-err");
    }

    // With no variable=, output goes to the response, same as the tag form.
    savecontent variable="execDirect" {
        execute name="/bin/echo" arguments="to-buffer" timeout=10;
    }
    assert("execute with no variable writes to output", trim(execDirect), "to-buffer");

    // A binary that cannot be launched raises an `application` exception on
    // Lucee — code shelling out writes `catch( application e )`, so a generic
    // runtime error would be uncatchable by type.
    execTypedCatch = false;
    try {
        execute name="/no/such/binary_rustcfml_probe" arguments="x" timeout=10 variable="never";
    } catch (application e) {
        execTypedCatch = true;
    } catch (any e) {
        execTypedCatch = false;
    }
    assertTrue("a failed spawn is catchable as type='application'", execTypedCatch);
}

// ============================================================
// Other script forms found missing by censusing every attribute-driven tag
// against Lucee rather than only fixing the reported one (GH #341).
// ============================================================

// `exit method=…;` is control flow — as a no-op the template just ran on.
request._exitProbe = "";
include "exit_statement_target.cfm";
assert("exit statement form stops the template", request._exitProbe, "before");

// `image action="info" …;` silently set nothing.
imgProbePath = getTempDirectory() & "/rustcfml_img_stmt_probe.png";
fileWrite(imgProbePath, toBinary("iVBORw0KGgoAAAANSUhEUgAAAAoAAAAKCAIAAAACUFjqAAAAEklEQVR4nGP4z8CAB+GTG8HSALfKY52fTcuYAAAAAElFTkSuQmCC"));
image action="info" source=imgProbePath structname="imgStmtInfo";
assert("image statement form populates structname", imgStmtInfo.width, 10);
fileDelete(imgProbePath);

// These function-call forms resolved to nothing and threw
// "Variable 'cfX' is undefined"; their statement forms already worked.
// NB `cfapplication(name=…)` is deliberately NOT exercised here: starting a
// differently-named application REPLACES the application's mapping set, and on
// Lucee that aborted 15 later files in this runner. Its call form is covered by
// the cross-engine probe, not by the shared suite.
cfloginuser(name="probeUser", password="pw", roles="probeRole");
assertTrue("cfloginuser() call form runs", true);
cflogout();
assertTrue("cflogout() call form runs", true);

suiteEnd();
</cfscript>
