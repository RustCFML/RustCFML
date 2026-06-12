<cfscript>
suiteBegin("Tags: cffile script-call form");

// ============================================================
// Background
// ============================================================
// CFML exposes built-in tags as script-callable functions. Inside a function,
//
//     cffile(action = "read", file = path, variable = "local.content");
//     cffile(attributeCollection = "#local.args#");
//
// must behave exactly like the corresponding <cffile> tag. On RustCFML the
// script-call form of cffile is not implemented at all: the call site is
// treated as an undefined identifier and throws "Variable 'cffile' is
// undefined" at runtime — identically for the named-attribute form and the
// attributeCollection form, for every action. The TAG form <cffile> works
// (control B below) and the fileRead()/fileWrite() BIFs work (control A),
// so only the script-call lowering is missing.
//
// Cross-engine: Lucee (and Adobe CF) execute every shape below. RustCFML
// 0.130.0 fails assertions (1)-(4) with "Variable 'cffile' is undefined"
// while both controls pass.
//
// All scratch files live under getTempDirectory() and are deleted on both
// the pass and the fail path, so the test is self-contained and runner-safe.
// ============================================================

// ---- Control A: file BIFs round-trip (works on both engines) ----
function zzcffilesfBifControl() {
	var p = getTempDirectory() & "zzcffilesf_bif_" & createUUID() & ".txt";
	var result = "";
	try {
		fileWrite(p, "bif-ok");
		result = fileRead(p);
	} catch (any e) {
		result = "ERROR: " & e.message;
	}
	if (fileExists(p)) fileDelete(p);
	return result;
}
assert("control: fileWrite()/fileRead() BIFs round-trip", zzcffilesfBifControl(), "bif-ok");
</cfscript>

<!--- ---- Control B: TAG form of the same tag (works on both engines).
      Tag syntax cannot appear inside a cfscript block, so this control runs
      as a tag island — same mixed tag/script pattern as test_tags_include.cfm.
      The tag form passing while assertions (1)-(4) fail pins the gap to the
      script-call lowering, not to the tag's file machinery. --->
<cfset request._zzcffilesf_tagPath = getTempDirectory() & "zzcffilesf_tag_" & createUUID() & ".txt">
<cfset request._zzcffilesf_tag = "">
<cftry>
	<cffile action="write" file="#request._zzcffilesf_tagPath#" output="tag-roundtrip">
	<cffile action="read" file="#request._zzcffilesf_tagPath#" variable="request._zzcffilesf_tag">
	<cfcatch type="any">
		<cfset request._zzcffilesf_tag = "ERROR: " & cfcatch.message>
	</cfcatch>
</cftry>
<cfif fileExists(request._zzcffilesf_tagPath)>
	<cfset fileDelete(request._zzcffilesf_tagPath)>
</cfif>

<cfscript>
// trim(): action="write" may add a trailing newline depending on the engine's
// addNewLine default; the contract under test is delivery, not newline policy.
assert("control: tag form write+read round-trips", trim(request._zzcffilesf_tag), "tag-roundtrip");

// ---- (1) script-call form, action="write", named attributes ----
function zzcffilesfWrite() {
	var p = getTempDirectory() & "zzcffilesf_w_" & createUUID() & ".txt";
	var result = "";
	try {
		cffile(action = "write", file = p, output = "written-via-script-call");
		result = fileExists(p) ? trim(fileRead(p)) : "no-file-created";
	} catch (any e) {
		result = "ERROR: " & e.message;
	}
	if (fileExists(p)) fileDelete(p);
	return result;
}
assert("(1) cffile(action='write', file=, output=) writes the file",
	zzcffilesfWrite(), "written-via-script-call");

// ---- (2) script-call form, action="read", variable="local.content" ----
// This is the exact shape Wheels' Global.cfc helpers use inside functions.
function zzcffilesfRead() {
	var p = getTempDirectory() & "zzcffilesf_r_" & createUUID() & ".txt";
	var result = "";
	local.content = "";
	try {
		fileWrite(p, "read-me-back");
		cffile(action = "read", file = p, variable = "local.content");
		result = local.content;
	} catch (any e) {
		result = "ERROR: " & e.message;
	}
	if (fileExists(p)) fileDelete(p);
	return result;
}
assert("(2) cffile(action='read', file=, variable='local.content') delivers the content",
	zzcffilesfRead(), "read-me-back");

// ---- (3) script-call form, action="append" ----
// Shape used by the Wheels migrator to emit SQL files (Base.cfc $file(action="append")).
function zzcffilesfAppend() {
	var p = getTempDirectory() & "zzcffilesf_app_" & createUUID() & ".txt";
	var result = "";
	try {
		fileWrite(p, "line1");
		cffile(action = "append", file = p, output = "line2", addnewline = "no");
		result = fileRead(p);
	} catch (any e) {
		result = "ERROR: " & e.message;
	}
	if (fileExists(p)) fileDelete(p);
	return result;
}
assert("(3) cffile(action='append', ..., addnewline='no') appends to the file",
	zzcffilesfAppend(), "line1line2");

// ---- (4) attributeCollection form ----
// Byte-for-byte the body of Wheels' Global.cfc $file(): copy arguments into a
// plain struct, then cffile(attributeCollection = "#local.args#").
function zzcffilesfAcWrite() {
	var p = getTempDirectory() & "zzcffilesf_ac_" & createUUID() & ".txt";
	var result = "";
	try {
		local.args = {action: "write", file: p, output: "ac-write-ok"};
		cffile(attributeCollection = "#local.args#");
		result = fileExists(p) ? trim(fileRead(p)) : "no-file-created";
	} catch (any e) {
		result = "ERROR: " & e.message;
	}
	if (fileExists(p)) fileDelete(p);
	return result;
}
assert("(4) cffile(attributeCollection='##local.args##') executes the collected attributes",
	zzcffilesfAcWrite(), "ac-write-ok");

suiteEnd();
</cfscript>
