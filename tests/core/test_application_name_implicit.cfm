<cfscript>
suiteBegin("Implicit application.applicationName key");

// Lucee/ACF auto-populate the application scope with an implicit `applicationName`
// key equal to the app name (Application.cfc `this.name`). Frameworks rely on it:
// Wheels builds lock names like `"controllerLock" & application.applicationName`,
// so a missing key throws "Variable 'applicationName' is undefined" and every
// request 500s. This test's app name is "RustCFMLTests" (see tests/Application.cfc).

assertTrue("structKeyExists sees applicationName", structKeyExists(application, "applicationName"));
assert("application.applicationName equals this.name", application.applicationName, "RustCFMLTests");

// The framework usage pattern: concatenation into a lock name must not throw.
lockName = "controllerLock" & application.applicationName;
assert("applicationName usable in string concat", lockName, "controllerLockRustCFMLTests");

// Case-insensitive access (CFML scope keys are case-insensitive).
assert("case-insensitive read", application.APPLICATIONNAME, "RustCFMLTests");

suiteEnd();
</cfscript>
