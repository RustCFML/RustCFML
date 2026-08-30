<cfscript>
suiteBegin("Requests serialise behind a running onApplicationStart");

// ============================================================
// Background
// ============================================================
// CFML's application lifecycle contract: onApplicationStart runs to
// completion before ANY request executes against the new application scope.
// Lucee implements this as a gate -- the request that triggers app start runs
// it, and every other request that arrives while it is mid-flight BLOCKS
// until it finishes, then proceeds against the fully-built scope.
//
// RustCFML gates only the triggering request. Concurrent requests see that
// the application scope exists and proceed straight into onRequest against
// whatever half-built state onApplicationStart has reached. In a real app
// that initialises singletons and a route registry progressively, a restart
// under live traffic serves a window of 500s ("Variable 'auth' is
// undefined") and then 404s (route registry not yet loaded) -- every
// concurrent in-flight request during app start is another bypass.
//
// Behavioural round-trip: the fixture app under application_start_gate/
// sets application.phase, sleeps 1500ms inside onApplicationStart, then sets
// application.ready. Three threads: t1 fires immediately (triggers app
// start); t2 and t3 fire at +400ms and +800ms, landing mid-app-start. Each
// records the response body ("ready=true" or "ready=MISSING"). The fixture
// keys this.name on a per-run token so a warm server still starts cold.
// Runs only when served (cgi.server_port present); skips from the CLI.
// ============================================================

serverPort = structKeyExists(cgi, "server_port") ? trim(cgi.server_port) : "";
skip = serverPort == "" || serverPort == "0";

if (skip) {
    assertTrue("app-start serialisation skipped (no cgi.server_port)", true);
} else {
    token = replace(createUUID(), "-", "", "all");
    base = "http://127.0.0.1:" & serverPort & "/tests/lifecycle/application_start_gate/page.cfm?app=" & token;

    thread name="gateT1" action="run" reqUrl="#base#" {
        try {
            cfhttp(url="#attributes.reqUrl#", result="r", timeout=20);
            thread.body = r.filecontent;
        } catch (any e) {
            thread.body = "THREW " & e.message;
        }
    }
    thread name="gateT2" action="run" reqUrl="#base#" {
        try {
            sleep(400);
            cfhttp(url="#attributes.reqUrl#", result="r", timeout=20);
            thread.body = r.filecontent;
        } catch (any e) {
            thread.body = "THREW " & e.message;
        }
    }
    thread name="gateT3" action="run" reqUrl="#base#" {
        try {
            sleep(800);
            cfhttp(url="#attributes.reqUrl#", result="r", timeout=20);
            thread.body = r.filecontent;
        } catch (any e) {
            thread.body = "THREW " & e.message;
        }
    }
    threadJoin("gateT1,gateT2,gateT3", 30000);

    // Leg 1 (control): the triggering request runs app start and sees the
    // completed scope. Green on both engines.
    assert("triggering request runs onApplicationStart and sees ready=true",
        cfthread.gateT1.body ?: "NO-BODY", "ready=true");

    // Legs 2-3 (the contract): requests arriving mid-app-start must block
    // until it completes, never observe the half-built scope.
    assert("request at +400ms blocks behind app start and sees ready=true",
        cfthread.gateT2.body ?: "NO-BODY", "ready=true");
    assert("request at +800ms blocks behind app start and sees ready=true",
        cfthread.gateT3.body ?: "NO-BODY", "ready=true");

    // Leg 4 (control): steady state after app start.
    try {
        cfhttp(url="#base#", result="rAfter", timeout=20);
        after = rAfter.filecontent;
    } catch (any e) {
        after = "THREW " & e.message;
    }
    assert("steady-state request sees ready=true", after, "ready=true");

    // Leg 5: onApplicationStart ran exactly once for the application --
    // blocked requests must not re-trigger it.
    try {
        cfhttp(url="#base#&op=runs", result="rRuns", timeout=20);
        runs = rRuns.filecontent;
    } catch (any e) {
        runs = "THREW " & e.message;
    }
    assert("onApplicationStart ran exactly once", runs, "runs=1");
}

suiteEnd();
</cfscript>
