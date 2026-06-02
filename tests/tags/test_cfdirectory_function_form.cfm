<cfscript>
suiteBegin("Tags: cfdirectory function-call form");

// ============================================================
// Background
// ============================================================
// Invoked as a CFScript function with named arguments — either explicitly
// (action / directory / filter / name) or via attributeCollection — `cfdirectory`
// performs the directory operation on Lucee 5/6/7, Adobe CF 2018-2025, and
// BoxLang. For action="list" it populates the `name` variable with a directory
// query.
//
// On RustCFML the cfdirectory function-call form rejects its named arguments and
// throws "cfdirectory requires a struct argument" — BOTH the explicit-args form
// and the attributeCollection form fail. (The `directoryList()` function works,
// and attributeCollection works on other tags e.g. cfheader, so this is specific
// to cfdirectory's function-call handler.)
//
// Wheels reaches this on the boot path: vendor/wheels/Global.cfc's $directory()
// wrapper calls cfdirectory(attributeCollection=...), used during plugin
// discovery and case-insensitive file lookup at onApplicationStart.
//
// The error is a catchable runtime error (not a parse error), so this test wraps
// the call in try/catch and asserts the listing succeeds rather than throws.
// ============================================================

function listViaCfdirectory() {
	var tmp = getTempDirectory() & "rustcfml_cfdir_" & createUUID() & "/";
	directoryCreate(tmp);
	fileWrite(tmp & "a.txt", "1");
	fileWrite(tmp & "b.txt", "2");
	var result = "";
	try {
		cfdirectory(action = "list", directory = tmp, filter = "*.txt", name = "dirQ");
		result = isQuery(dirQ) ? "rows=" & dirQ.recordCount : "not-a-query";
	} catch (any e) {
		result = "ERROR: " & e.message;
	}
	directoryDelete(tmp, true);
	return result;
}

assert("cfdirectory(action='list', directory=, filter=, name=) lists the directory as a query",
	listViaCfdirectory(), "rows=2");

suiteEnd();
</cfscript>
