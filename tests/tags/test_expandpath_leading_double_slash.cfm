<cfscript>
suiteBegin("Tags: expandPath normalizes a leading double-slash before mapping resolution");

// ============================================================
// Background
// ============================================================
// A leading "//" in a path is equivalent to a single leading "/": CFML
// normalizes redundant leading slashes before resolving the path against
// this.mappings. On Lucee/Adobe/BoxLang expandPath("//x") == expandPath("/x"),
// so a "//"-prefixed mapped path resolves to the SAME mapping target.
//
// RustCFML 0.144.0 resolves expandPath("/wheelsmapprobe") through the
// this.mappings["/wheelsmapprobe"] entry correctly, but expandPath(
// "//wheelsmapprobe") does NOT normalize the leading double-slash: it falls
// through to a docroot-relative path (<docroot>/wheelsmapprobe), missing the
// mapping entirely. A mid-string double-slash ("/wheelsmapprobe//x") still
// resolves the mapping; only the LEADING "//" defeats it.
//
// Why it matters for Wheels: the stock `wheels new` Application.cfc declares
// a "/plugins" mapping, and Wheels' boot builds the plugins path by joining
// application.webPath("/") & application.pluginPath("/plugins") = "//plugins".
// Plugins.cfc then does cfdirectory(action="list") on ExpandPath("//plugins").
// On RustCFML that expands to <docroot>/plugins instead of the mapped target,
// the directory doesn't exist, cfdirectory throws "directory not found", and
// $init aborts — every request 500s before the app can render. (The split
// public/ webroot + /plugins mapping is the stock template layout, so this
// reproduces on any pristine Wheels app served by RustCFML.)
// ============================================================

// /wheelsmapprobe is declared in tests/Application.cfc -> tests/tags/, which
// exists; it is NOT a real webroot subdirectory, so only the mapping can
// resolve it (the same isolation tags/test_mapping_include.cfm relies on).
epdsSingle = expandPath("/wheelsmapprobe");
epdsDouble = expandPath("//wheelsmapprobe");

// --- CONTROL (green on both engines): the single-slash mapped path resolves ---
assertTrue("CONTROL: expandPath('/wheelsmapprobe') resolves the mapping (dir exists)",
    directoryExists(epdsSingle));

// --- the gap: a leading '//' must normalize to the same resolved path ---
assert("expandPath('//x') equals expandPath('/x') (leading-slash normalization)",
    epdsDouble, epdsSingle);
assertTrue("expandPath('//wheelsmapprobe') still resolves the mapping (dir exists)",
    directoryExists(epdsDouble));

suiteEnd();
</cfscript>
