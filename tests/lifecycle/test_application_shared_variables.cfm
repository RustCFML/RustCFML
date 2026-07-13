<cfscript>
suiteBegin("Lifecycle: Application.cfc shared variables scope during construction");

// Two regressions surfaced by booting Masa CMS on RustCFML:
//   1. A method called during the Application.cfc pseudo-constructor that writes
//      `variables.x` was invisible to a sibling method it called (each call got
//      its own scope). Fixed by building a shared, Arc-backed `variables` handle
//      before running the app-component body — mirroring the normal component path.
//   2. A missing/unreadable dynamic `include` threw an uncatchable hard error
//      instead of a catchable `missingInclude`, so `try/catch(any)` around it
//      never fired.
// Both are exercised by the fixture Application.cfc; onRequest reports the result.

serverPort = structKeyExists(cgi, "server_port") ? trim(cgi.server_port) : "";

if (serverPort == "" || serverPort == "0") {
    assertTrue("shared-variables lifecycle skipped (no cgi.server_port)", true);
} else {
    targetPath = "/tests/lifecycle/application_shared_variables/sub/page.cfm";
    http url="http://127.0.0.1:#serverPort##targetPath#" method="GET" result="svResult";
    assert("shared-variables request status", svResult.statuscode, "200 OK");
    assert("variables written across sibling methods during construction persist + missing include is catchable",
        trim(svResult.filecontent), "ini=ok;missing=yes");
}

suiteEnd();
</cfscript>
