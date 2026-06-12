<cfscript>
suiteBegin("Tags: cfdirectory attributeCollection delivers name= result");

// ============================================================
// Background
// ============================================================
// cfdirectory's `name=` attribute names the variable that receives the
// listing query. That contract must hold no matter HOW the attribute
// reaches the tag: direct named attributes, attributeCollection = struct,
// or the string-interpolated attributeCollection = "#local.args#" form.
//
//     local.rv = "";
//     local.args = {action: "list", directory: dir, name: "rv"};
//     cfdirectory(attributeCollection = "#local.args#");
//     // Lucee 7          -> local.rv is the listing query
//     // RustCFML 0.130.0 -> broken in BOTH execution modes:
//     //   CLI mode:   the attributeCollection struct never reaches the
//     //               tag at all -> throws "cfdirectory: directory not
//     //               found: " (note the EMPTY path)
//     //   serve mode: the attributes get through (the listing runs) but
//     //               the name= result is delivered to NO scope --
//     //               local.rv stays "", variables.rv unset,
//     //               IsDefined("rv") false
//
// The direct form cfdirectory(action="list", ..., name="rv") delivers
// correctly on RustCFML in both modes, and cfquery(attributeCollection=...)
// delivers its name= / result= -- the delivery plumbing exists; only
// cfdirectory's attributeCollection path is broken.
//
// Wheels hits this on EVERY boot: Global.cfc::$directory() is exactly this
// shape (copy arguments into a struct -> attributeCollection -> return
// local.rv), and Plugins.cfc::$folders() feeds its return value straight
// into a query-of-queries at onApplicationStart -- the silent "" return
// surfaces as "Query of Queries: table 'query' not found". The same helper
// backs $fileExistsNoCase() -> $objectFileName(), i.e. every
// controller/model class-file resolution.
//
// All cfdirectory calls are wrapped in try/catch so the CLI-mode throw is
// reported as an assertion failure instead of aborting the bundle.
// ============================================================

// --- self-contained fixture: scratch dir with two known files ---
cfdirAcTmp = getTempDirectory() & "rustcfml_cfdirac_" & createUUID() & "/";
directoryCreate(cfdirAcTmp);
fileWrite(cfdirAcTmp & "alpha.txt", "1");
fileWrite(cfdirAcTmp & "beta.txt", "2");

// helper: render the delivery outcome readable in failure messages
function cfdirAcShape(required any rv) {
	if (isQuery(arguments.rv)) {
		return "QUERY rows=" & arguments.rv.recordCount;
	}
	if (isSimpleValue(arguments.rv)) {
		return "NOT-DELIVERED [" & arguments.rv & "]";
	}
	return "NOT-A-QUERY";
}

// (1) green control: direct named attributes deliver into a pre-initialized
//     local variable -- this works on RustCFML too, isolating the gap to the
//     attributeCollection path.
function cfdirAcDirect(required string dir) {
	local.rv = "";
	try {
		cfdirectory(action = "list", directory = arguments.dir, filter = "*.txt", name = "rv");
	} catch (any e) {
		return "ERROR: " & e.message;
	}
	return cfdirAcShape(local.rv);
}
assert("direct named attrs: name= delivers the listing query",
	cfdirAcDirect(cfdirAcTmp), "QUERY rows=2");

// (2) attributeCollection passed as a struct value
function cfdirAcStruct(required string dir) {
	local.rv = "";
	local.args = {action: "list", directory: arguments.dir, filter: "*.txt", name: "rv"};
	try {
		cfdirectory(attributeCollection = local.args);
	} catch (any e) {
		return "ERROR: " & e.message;
	}
	return cfdirAcShape(local.rv);
}
assert("attributeCollection = struct: name= delivers the listing query",
	cfdirAcStruct(cfdirAcTmp), "QUERY rows=2");

// (3) the EXACT Wheels Global.cfc::$directory shape: arguments copied into a
//     plain struct, string-interpolated attributeCollection, result read
//     back from the pre-initialized local.rv
function cfdirAcWheels() {
	local.rv = "";
	arguments.name = "rv";
	local.args = {};
	for (local.key in arguments) {
		local.args[local.key] = arguments[local.key];
	}
	cfdirectory(attributeCollection = "#local.args#");
	return local.rv;
}
try {
	cfdirAcQ = cfdirAcWheels(action = "list", directory = cfdirAcTmp, filter = "*.txt", sort = "name asc");
} catch (any e) {
	cfdirAcQ = "ERROR: " & e.message;
}
assert("Wheels $directory shape (interpolated attributeCollection): returns the query",
	cfdirAcShape(cfdirAcQ), "QUERY rows=2");

// (4) the delivered listing must be usable downstream -- the Plugins.cfc
//     $folders() query-of-queries that is the first thing to die at Wheels
//     boot when the delivery is dropped.
function cfdirAcQoq(required any src) {
	try {
		local.q = queryExecute(
			"SELECT name FROM src WHERE name NOT LIKE '.%' ORDER BY name ASC",
			{},
			{dbtype: "query"}
		);
		return "rows=" & local.q.recordCount & " first=" & local.q.name[1];
	} catch (any e) {
		return "ERROR: " & e.message;
	}
}
assert("downstream QoQ over the delivered listing (Plugins.cfc $folders shape)",
	cfdirAcQoq(cfdirAcQ), "rows=2 first=alpha.txt");

// --- cleanup ---
directoryDelete(cfdirAcTmp, true);

suiteEnd();
</cfscript>
