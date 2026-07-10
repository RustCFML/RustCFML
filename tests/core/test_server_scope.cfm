<cfscript>
suiteBegin("Server scope (Lucee compatibility shim)");

// --- server.coldfusion ---
assert("server.coldfusion.productname", server.coldfusion.productname, "RustCFML");
assertTrue("server.coldfusion.productversion is non-empty", len(server.coldfusion.productversion) GT 0);
assertTrue("server.coldfusion.productlevel exists", structKeyExists(server.coldfusion, "productlevel"));

// --- server.lucee (engine-detection shim: frameworks like Wheels/ColdBox/
// Preside sniff server.lucee.version to identify a Lucee-dialect engine) ---
assertTrue("server.lucee exists", structKeyExists(server, "lucee"));
assertTrue("server.lucee.version is non-empty", len(server.lucee.version) GT 0);
assertTrue("server.lucee.version major >= 5", listFirst(server.lucee.version, ".") GTE 5);

// --- server.railo (Lucee's back-compat alias for server.lucee; Preside's
// Lucee error template reads server.railo.version) ---
assertTrue("server.railo exists", structKeyExists(server, "railo"));
assert("server.railo.version mirrors server.lucee.version", server.railo.version, server.lucee.version);

// --- server.os ---
assertTrue("server.os.name is non-empty", len(server.os.name) GT 0);
assertTrue("server.os.arch is non-empty", len(server.os.arch) GT 0);

// --- server.separator (Lucee 4.5+) ---
assertTrue("server.separator.file is non-empty", len(server.separator.file) GT 0);
assertTrue("server.separator.path is non-empty", len(server.separator.path) GT 0);
assertTrue("server.separator.line is non-empty", len(server.separator.line) GT 0);

// --- server.java (no JVM, but keys exist for migration code) ---
assertTrue("server.java exists", isStruct(server.java));
assertTrue("server.java.archModel is 32 or 64", server.java.archModel EQ "64" OR server.java.archModel EQ "32");

// --- server.system.environment (the headline shim) ---
assertTrue("server.system.environment is a struct", isStruct(server.system.environment));
// PATH (Linux/macOS) or Path (Windows) is essentially universal at process start.
hasPath = structKeyExists(server.system.environment, "PATH") OR structKeyExists(server.system.environment, "Path");
assertTrue("server.system.environment exposes PATH", hasPath);

// case-insensitive key access (standard CFML struct semantics)
if (structKeyExists(server.system.environment, "PATH")) {
    assertTrue("env key access is case-insensitive", len(server.system.environment.path) GT 0);
}

// Round-trip: a value visible via getEnvironmentVariable() should be visible in the shim.
// We can't rely on a specific external env var, so set one for ourselves via the
// Rust process env? We can't from CFML. Instead, just assert that for at least one
// key present in the shim, getEnvironmentVariable returns a non-empty string.
testKey = "";
for (k in server.system.environment) {
    if (len(server.system.environment[k]) GT 0) {
        testKey = k;
        break;
    }
}
if (len(testKey)) {
    assertTrue(
        "getEnvironmentVariable agrees with server.system.environment for " & testKey,
        getEnvironmentVariable(testKey) EQ server.system.environment[testKey]
    );
}

// --- server.system.properties ---
assertTrue("server.system.properties is a struct", isStruct(server.system.properties));
assertTrue("properties has os.name (dotted literal key)", structKeyExists(server.system.properties, "os.name"));
assertTrue("properties has file.separator", structKeyExists(server.system.properties, "file.separator"));
assertTrue("properties has user.dir", structKeyExists(server.system.properties, "user.dir"));

// --- server.system.arguments ---
assertTrue("server.system.arguments is an array", isArray(server.system.arguments));

suiteEnd();
</cfscript>
