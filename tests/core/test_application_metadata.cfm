<cfscript>
suiteBegin("getApplicationMetadata exposes Application.cfc settings");

// Regression: getApplicationMetadata() used to be a stub returning {name:""}.
// It now reflects the live Application.cfc `this` settings (name,
// sessionManagement, ...). WireBox's ScopeStorage reads
// getApplicationMetadata().sessionManagement to decide whether the session
// scope is available.

md = getApplicationMetadata();
assert("application name is reported", md.name, "RustCFMLTests");
assertTrue("sessionManagement is exposed", structKeyExists(md, "sessionManagement"));
assertTrue("sessionManagement is true (set in tests/Application.cfc)", md.sessionManagement);

// --- mappings: the APPLICATION's, not the server's (GH ##348) ---
// Lucee lists only what the application declared. We used to add the implicit
// webroot `/` and any `.cfconfig.json` CFMappings, which broke the standard
// add-a-mapping idiom: since v0.621.0 `application action="update" mappings={…}`
// REPLACES the application's set (Lucee parity), so the idiom is a
// read-modify-write, and sweeping server mappings into the read re-registered
// them as the application's — a later update would then drop them.
maps = md.mappings;
assertTrue("mappings is a struct", isStruct(maps));
assertTrue("declared /oop mapping is reported", structKeyExists(maps, "/oop"));
assertTrue("declared /core mapping is reported", structKeyExists(maps, "/core"));
assertTrue("declared /tags mapping is reported", structKeyExists(maps, "/tags"));
assertFalse("the implicit webroot mapping is NOT reported", structKeyExists(maps, "/"));

// The round-trip the idiom depends on: read, add, write back, and every
// previously declared mapping must survive.
maps["/appmetaprobe"] = expandPath("./oop/");
application action="update" mappings="#maps#";
after = getApplicationMetadata().mappings;
assertTrue("round-trip keeps /oop", structKeyExists(after, "/oop"));
assertTrue("round-trip keeps /core", structKeyExists(after, "/core"));
assertTrue("round-trip keeps /tags", structKeyExists(after, "/tags"));
assertTrue("round-trip adds the new mapping", structKeyExists(after, "/appmetaprobe"));
assertFalse("round-trip did not adopt the webroot", structKeyExists(after, "/"));

// Leave the application as we found it — a bare `action="update"` REPLACES the
// set, so the probe mapping has to be removed by writing the set back without
// it, not left for the next test file to trip over.
structDelete(after, "/appmetaprobe");
application action="update" mappings="#after#";

suiteEnd();
</cfscript>
