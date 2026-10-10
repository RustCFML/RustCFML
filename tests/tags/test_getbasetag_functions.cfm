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
//     getBaseTagData("CFMODULE") cannot find them (leg E);
//   - a cfimport-prefixed tag is listed and found as CF_<NAME>, exactly like
//     its cf_ form: the prefix is not part of the name (legs F and G,
//     measured on Lucee 7.0.4.34 and Lucee 5.3.10, which agree).

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

// (legs F and G follow in tag syntax; cleanup is at the end)
</cfscript>

<!--- ── F: a cfimport-prefixed tag has the same identity as its cf_ form:
       it is listed as CF_<NAME> (the prefix is not part of the name, and the
       name is uppercased however it is written) and found by that name. --->
<cfimport taglib="." prefix="btl">
<cfset structDelete(request, "btimp") />
<cfset structDelete(request, "btimpcase") />
<cfset request.bt_imp_err = "" />
<cftry>
    <btl:basetag_probe marker="imp-1" report="btimp" />
    <btl:BaseTag_Probe marker="imp-case" report="btimpcase" />
    <cfcatch type="any"><cfset request.bt_imp_err = "THREW: " & cfcatch.message /></cfcatch>
</cftry>

<cfscript>
assert( "F: cfimport-prefixed probe completes", request.bt_imp_err EQ "" ? "ok" : request.bt_imp_err, "ok" );
impProbe = request.btimp ?: {};
assert( "F: a prefixed tag's own entry is CF_<NAME>, not CFMODULE", impProbe.first ?: "(missing)", "CF_BASETAG_PROBE" );
assert( "F: element 2 is still the suite runner", impProbe.parent_name ?: "(missing)", "CF_RUNTEST" );
assert( "F: a prefixed tag is found by CF_<NAME>", impProbe.self_marker ?: "(missing)", "imp-1" );
impCase = request.btimpcase ?: {};
assert( "F: the name is uppercased however it is written", impCase.first ?: "(missing)", "CF_BASETAG_PROBE" );
assert( "F: and the mixed-case tag is found by CF_<NAME> too", impCase.self_marker ?: "(missing)", "imp-case" );
</cfscript>

<!--- ── G: a cfimport-prefixed host is in its body tags' ancestry, and a
       body tag reads the host's attributes through getBaseTagData(). --->
<cfset structDelete(request, "btimpbody") />
<cfset request.bt_impbody_err = "" />
<cftry>
    <btl:basetag_outer marker="imp-outer"><btl:basetag_probe marker="imp-body" report="btimpbody" /></btl:basetag_outer>
    <cfcatch type="any"><cfset request.bt_impbody_err = "THREW: " & cfcatch.message /></cfcatch>
</cftry>

<cfscript>
assert( "G: prefixed host with a prefixed body tag completes", request.bt_impbody_err EQ "" ? "ok" : request.bt_impbody_err, "ok" );
impBody = request.btimpbody ?: {};
assert( "G: element 2 of the body tag is the prefixed host", impBody.parent_name ?: "(missing)", "CF_BASETAG_OUTER" );
assert( "G: the body tag reads the host's attributes via getBaseTagData", impBody.parent_marker ?: "(missing)", "imp-outer" );

// (legs H, I and J follow in tag syntax; cleanup is at the end)
</cfscript>

<!--- ── H: getBaseTagData() matches the TEMPLATE, not the listed name. The
       same ancestor resolves under CF_<NAME>, the bare name, and either in any
       case; `CFMODULE` resolves to nothing, because it is not a template stem.
       Also: getBaseTagList() honours a delimiter argument, and the not-found
       message echoes the requested name as written. All measured on Lucee
       7.1.0.204. --->
<cfset structDelete(request, "btlk") />
<cfset request.bt_lk_err = "" />
<cftry>
    <cf_basetag_lookup_probe marker="lk-direct" report="btlk" />
    <cfcatch type="any"><cfset request.bt_lk_err = "THREW: " & cfcatch.message /></cfcatch>
</cftry>

<cfscript>
assert( "H: lookup probe completes", request.bt_lk_err EQ "" ? "ok" : request.bt_lk_err, "ok" );
lk = request.btlk ?: {};
assert( "H: found by CF_<NAME>",            lk.lookup.prefixed ?: "(missing)", "lk-direct" );
assert( "H: found by the BARE name",        lk.lookup.bare     ?: "(missing)", "lk-direct" );
assert( "H: found lower-case",              lk.lookup.lower    ?: "(missing)", "lk-direct" );
assert( "H: found in mixed case",           lk.lookup.mixed    ?: "(missing)", "lk-direct" );
assert( "H: CFMODULE is not a lookup name", lk.lookup.module ?: "(missing)", "(threw)" );
assertTrue( "H: the not-found message keeps the requested name's case (saw: " & ( lk.missingMsg ?: "" ) & ")",
    find( "[CF_NoSuchTag]", lk.missingMsg ?: "" ) GT 0 );
</cfscript>

<!--- ── I: a cfmodule-invoked tag lists as CFMODULE but is still FOUND by its
       template's name — the list says how it was invoked, the lookup asks which
       template it is. --->
<cfset structDelete(request, "btlkmod") />
<cfset request.bt_lkmod_err = "" />
<cftry>
    <cfmodule template="basetag_lookup_probe.cfm" marker="lk-module" report="btlkmod" />
    <cfcatch type="any"><cfset request.bt_lkmod_err = "THREW: " & cfcatch.message /></cfcatch>
</cftry>

<cfscript>
assert( "I: module-invoked lookup probe completes", request.bt_lkmod_err EQ "" ? "ok" : request.bt_lkmod_err, "ok" );
lkMod = request.btlkmod ?: {};
assert( "I: a cfmodule-invoked tag still lists as CFMODULE", lkMod.first ?: "(missing)", "CFMODULE" );
assert( "I: but IS found by CF_<NAME>", lkMod.lookup.prefixed ?: "(missing)", "lk-module" );
assert( "I: and by the bare template name", lkMod.lookup.bare ?: "(missing)", "lk-module" );
assert( "I: while CFMODULE still finds nothing", lkMod.lookup.module ?: "(missing)", "(threw)" );
</cfscript>

<!--- ── J: getBaseTagList() honours its delimiter argument. Needs two
       ancestors for the delimiter to be observable at all. --->
<cfset structDelete(request, "btlknest") />
<cfset request.bt_lknest_err = "" />
<cftry>
    <cf_basetag_outer marker="lk-outer"><cf_basetag_lookup_probe marker="lk-nested" report="btlknest" /></cf_basetag_outer>
    <cfcatch type="any"><cfset request.bt_lknest_err = "THREW: " & cfcatch.message /></cfcatch>
</cftry>

<cfscript>
assert( "J: nested lookup probe completes", request.bt_lknest_err EQ "" ? "ok" : request.bt_lknest_err, "ok" );
lkNest = request.btlknest ?: {};
assert( "J: getBaseTagList( ';' ) joins on the delimiter given",
    lkNest.semi ?: "(missing)", "CF_BASETAG_LOOKUP_PROBE;CF_BASETAG_OUTER;CF_RUNTEST" );

structDelete(request, "btlk");
structDelete(request, "btlkmod");
structDelete(request, "btlknest");
structDelete(request, "bt_lk_err");
structDelete(request, "bt_lkmod_err");
structDelete(request, "bt_lknest_err");
structDelete(request, "btimp");
structDelete(request, "btimpcase");
structDelete(request, "btimpbody");
structDelete(request, "bt_imp_err");
structDelete(request, "bt_impbody_err");
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
