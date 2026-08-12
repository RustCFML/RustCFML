<cfscript>
// getBaseTagList() / getBaseTagData(): the standard custom-tag ancestry API
// (Lucee and ACF both ship them). They are how nested custom tags find and
// talk to their ancestors — the fragment/slot pattern deposits content into a
// parent's attributes through the getBaseTagData() reference, and layout/
// modal/panel tag libraries are built on it.
//
// RustCFML implements neither: the first call resolves as a variable read and
// throws "Variable 'GetBaseTagList' is undefined", so every template in such
// a tag library dies on its first line. Repro class: titan (Moopa) on
// v0.575.0 — /sales died in its slot tag; the whole mechanism (136 call
// sites: every modal, layout and admin grid) had to be rewritten onto a
// hand-rolled request-scope stack to run at all.
//
// Expected values measured on Lucee 7.0 (docker lucee/lucee:7.0, repo as
// webroot, suite via tests/runner.cfm). Two measured subtleties pinned here
// so an implementation gets them right:
//   - a cf_-invoked host IS in the ancestry of tags placed in its body
//     (leg D) — but a cfmodule-invoked host is NOT (its body runs in the
//     caller's context with no CFMODULE ancestor added);
//   - cfmodule-invoked tags appear in getBaseTagList() as CFMODULE but
//     getBaseTagData("CFMODULE") cannot find them (leg E).

suiteBegin("getBaseTagList()/getBaseTagData(): custom-tag ancestry built-ins");

// ── A: direct calls from this page (which runs inside cf_runtest) ──
aList = "(threw)";
try { aList = getBaseTagList(); } catch (any e) { aList = "THREW: " & e.message; }
assertTrue( "A: getBaseTagList() from an in-tag page returns the ancestry (saw: " & aList & ")",
    listFindNoCase( aList, "CF_RUNTEST" ) GT 0 );

aFile = "(threw)";
try { aFile = getBaseTagData( "CF_RUNTEST" ).attributes.file ?: "(missing)"; } catch (any e) { aFile = "THREW: " & e.message; }
assert( "A: getBaseTagData() by name reads that ancestor's attributes", aFile, "tags/test_getbasetag_functions.cfm" );
</cfscript>

<!--- ── B: leaf probe invoked as a cf_ tag directly from the test file ── --->
<cfset structDelete(request, "btprobe") />
<cfset request.bt_leaf_err = "" />
<cftry>
    <cf_basetag_probe marker="leaf-1" report="btprobe" />
    <cfcatch type="any"><cfset request.bt_leaf_err = "THREW: " & cfcatch.message /></cfcatch>
</cftry>

<cfscript>
assert( "B: probe using the ancestry built-ins completes", request.bt_leaf_err EQ "" ? "ok" : request.bt_leaf_err, "ok" );
probe = request.btprobe ?: {};
assert( "B: first element of getBaseTagList() is the tag's own entry", probe.first ?: "(missing)", "CF_BASETAG_PROBE" );
assert( "B: element 2 is the nearest enclosing tag (the suite runner)", probe.parent_name ?: "(missing)", "CF_RUNTEST" );
assert( "B: self lookup by own tag name reaches this instance", probe.self_marker ?: "(missing)", "leaf-1" );
assert( "B: CF_RUNTEST.attributes.file readable via getBaseTagData", probe.runtest_file ?: "(missing)", "tags/test_getbasetag_functions.cfm" );
</cfscript>

<!--- ── C: genuine template nesting — the outer host invokes the probe from
       its OWN template (how a modal tag renders its fragment/slot children);
       the probe reaches the outer tag's attributes and mutates them through
       the getBaseTagData() reference. --->
<cfset structDelete(request, "btnested") />
<cfset structDelete(request, "btouter") />
<cfset request.bt_nested_err = "" />
<cftry>
    <cf_basetag_outer marker="outer-1"></cf_basetag_outer>
    <cfcatch type="any"><cfset request.bt_nested_err = "THREW: " & cfcatch.message /></cfcatch>
</cftry>

<cfscript>
assert( "C: outer host invoking the probe from its own template completes", request.bt_nested_err EQ "" ? "ok" : request.bt_nested_err, "ok" );
probe = request.btnested ?: {};
assertTrue( "C: nested ancestry has at least 3 entries (probe, outer, runner) (saw: " & ( probe.list ?: "(none)" ) & ")",
    ( probe.len ?: 0 ) GTE 3 );
assert( "C: element 2 is the outer host", probe.parent_name ?: "(missing)", "CF_BASETAG_OUTER" );
assert( "C: getBaseTagData(parent) reads the outer host's attributes", probe.parent_marker ?: "(missing)", "outer-1" );
assert( "C: self lookup still reaches the probe itself", probe.self_marker ?: "(missing)", "inner-1" );
assert(
    "C: slot deposited through the getBaseTagData() reference is visible to the host",
    ( request.btouter.slots_report ?: "(no report)" ),
    "actions=deposited-by-probe"
);
</cfscript>

<!--- ── D: body placement (Lucee-measured): a tag invoked in a cf_ host's
       BODY sees the host in its ancestry — element 2 is the host, and its
       attributes are readable there. (Contrast: a cfmodule-invoked host does
       NOT appear in its body tags' ancestry.) --->
<cfset structDelete(request, "btbody") />
<cfset request.bt_body_err = "" />
<cftry>
    <cf_basetag_outer marker="outer-2"><cf_basetag_probe marker="body-1" report="btbody" /></cf_basetag_outer>
    <cfcatch type="any"><cfset request.bt_body_err = "THREW: " & cfcatch.message /></cfcatch>
</cftry>

<cfscript>
assert( "D: body-nested probe completes", request.bt_body_err EQ "" ? "ok" : request.bt_body_err, "ok" );
assert( "D: a cf_ host IS in its body tags' ancestry (element 2)",
    ( request.btbody.parent_name ?: "(missing)" ), "CF_BASETAG_OUTER" );
assert( "D: the body tag reads the host's attributes via getBaseTagData",
    ( request.btbody.parent_marker ?: "(missing)" ), "outer-2" );
</cfscript>

<!--- ── E: cfmodule limitation (Lucee-measured): a module-invoked tag appears
       in getBaseTagList() as CFMODULE, but getBaseTagData("CFMODULE") throws
       ("can't find base tag with name [CFMODULE]") — module entries are not
       findable by name. --->
<cfset structDelete(request, "btmod") />
<cfset request.bt_mod_err = "" />
<cftry>
    <cfmodule template="basetag_probe.cfm" marker="mod-1" report="btmod">
    <cfcatch type="any"><cfset request.bt_mod_err = "THREW: " & cfcatch.message /></cfcatch>
</cftry>

<cfscript>
assert( "E: module-invoked probe completes", request.bt_mod_err EQ "" ? "ok" : request.bt_mod_err, "ok" );
modProbe = request.btmod ?: {};
assert( "E: module-invoked tag's own getBaseTagList() entry is CFMODULE", modProbe.first ?: "(missing)", "CFMODULE" );
assertTrue( "E: getBaseTagData('CFMODULE') cannot find module entries (saw: " & ( modProbe.cfmodule_lookup ?: "(missing)" ) & ")",
    findNoCase( "(threw:", modProbe.cfmodule_lookup ?: "" ) GT 0 );

structDelete(request, "btprobe");
structDelete(request, "btnested");
structDelete(request, "btbody");
structDelete(request, "btmod");
structDelete(request, "btouter");
structDelete(request, "bt_leaf_err");
structDelete(request, "bt_nested_err");
structDelete(request, "bt_body_err");
structDelete(request, "bt_mod_err");

suiteEnd();
</cfscript>
