<cfscript>
suiteBegin("Tags: cfhttp timeout interpolation");
</cfscript>

<!---
    ============================================================
    Background
    ============================================================
    A quoted <cfhttp> attribute value may carry #...# interpolation. The merged
    cfhttp interpolation work routes url/method/charset/username/password/
    useragent/proxyserver through the interpolation path, but `timeout` was
    emitted verbatim into the generated cfhttp({ ... }) call. So
    timeout="#arguments.timeout#" produced "timeout: #arguments.timeout#" --
    literal hashes in CFScript -- and the template failed to PARSE. A literal
    timeout (timeout="60") happened to work because it has no hashes.

    Because that is a hard parse error (escapes try/catch, would abort the
    runner), the cases live in runtime-instantiated fixture CFCs. The control
    (literal timeout) parses today and guards the fixture wiring; the
    interpolated fixture is expected to fail on current upstream until `timeout`
    is interpolated like every other cfhttp attribute. The cfhttp call is guarded
    by <cfif false> so it is compiled but never executed (no network).

    Why it matters for Moopa: code/moopa/lib/cloudflare_stream.cfc (isPlaybackReady)
    issues <cfhttp ... timeout="#arguments.timeout#">.
    ============================================================
--->

<cfscript>
// Instantiate a fixture and run run(); returns "ok" when the fixture parsed and
// ran, or a diagnostic string when it did not (so a parse failure becomes a
// clean assertion mismatch rather than an aborted suite).
function loadRun(required string name) {
    try {
        var o = createObject("component", arguments.name);
        if (!isObject(o)) {
            return "NOT-A-COMPONENT";
        }
        return o.run();
    } catch (any e) {
        return "THREW: " & e.message;
    }
}

// --- control: literal timeout already parses (regression guard) --------------

assert("control: cfhttp with literal timeout parses",
    loadRun("CfhttpTimeoutControlFixture"), "ok");

// --- gap: interpolated timeout -----------------------------------------------

assert("cfhttp with interpolated timeout attribute parses",
    loadRun("CfhttpTimeoutInterpFixture"), "ok");

suiteEnd();
</cfscript>
