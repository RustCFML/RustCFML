<cfscript>
suiteBegin("cfconfig — loading");

// Skip on Lucee — this suite verifies RustCFML-specific config wiring.
hasCfconfig = isDefined("server") && isStruct(server) && structKeyExists(server, "cfconfig");
if (!hasCfconfig) {
    suiteEnd();
    return;
}

cfg = server.cfconfig;
assert("cfconfig is a struct", isStruct(cfg), true);

// The whole `server.*` section (port, welcomeFiles, maxRequestBodySize, ...) is
// SERVER-level config, not application-level. It is owned by the server baseline
// (`--cfconfig` / discovery) and is intentionally NOT overlaid from a per-app
// `.cfconfig.json`. The listening port is never a cfconfig setting at all —
// pages read `cgi.server_port`.
//
// Under the CLI runner this fixture (tests/.cfconfig.json) IS the baseline, so
// the server.* values are in effect and asserted. Over HTTP the same file is
// discovered as a per-application overlay (it sits beside tests/Application.cfc),
// whose server section is deliberately ignored — so skip the server.* asserts.
runningOverHttp = structKeyExists(cgi, "server_port") && val(cgi.server_port) GT 0;
if (!runningOverHttp) {
    assert("server.welcomeFiles[2]", cfg.server.welcomeFiles[2], "default.cfm");
    assert("server.maxRequestBodySize", cfg.server.maxRequestBodySize, 5242880);
}

// Application-level values from tests/.cfconfig.json — overlaid per request.
assert("runtime.locale", cfg.runtime.locale, "en-GB");
assert("runtime.timezone", cfg.runtime.timezone, "Europe/London");
assert("runtime.sessionTimeout", cfg.runtime.sessionTimeout, "0,0,15,0");

// Defaults filled in where the fixture is silent
// "0.0.0.0", not "127.0.0.1": the listener always bound all interfaces and
// never read this key, so the old loopback default documented behaviour the
// engine did not have. The key IS now honoured by the TCP listener, so the
// default was changed to match what the server actually does; set it to
// "127.0.0.1" to restrict the server to the local machine.
assert("server.host default", cfg.server.host, "0.0.0.0");
assert("runtime.dotNotationUpperCase default", cfg.runtime.dotNotationUpperCase, true);

// Datasource declared
assert("datasources.testds.driver", cfg.datasources.testds.driver, "sqlite");

// Security flags
assert("security.csrfEnabled", cfg.security.csrfEnabled, true);
assert("security.disallowedFunctions has __cfexecute", arrayLen(cfg.security.disallowedFunctions), 1);
assert("disallowedFunctions[1]", cfg.security.disallowedFunctions[1], "cfconfigSecurityProbe");

// Debugging
assert("debugging.enabled", cfg.debugging.enabled, true);
assert("debugging.errorStatusCode", cfg.debugging.errorStatusCode, false);

suiteEnd();
</cfscript>
