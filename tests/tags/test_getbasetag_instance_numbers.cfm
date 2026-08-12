<cfscript>
// getBaseTagData(name, instancenumber) semantics — measured IDENTICAL on
// Lucee 7.0 and RustCFML v0.591.0, pinned here so they stay that way:
//
//   - the instance number is PER NAME: "the Nth ancestor called CF_X",
//     not "the Nth ancestor overall";
//   - it is 1-based, defaulting to 1; instance 0 is clamped to 1;
//   - ordering is NEAREST-first (instance 1 = closest enclosing instance);
//   - out-of-range throws "can't find base tag with name [X]" — the same
//     message on both engines.
//
// Repro class: titan (Moopa) walked its ancestor list passing one GLOBAL
// counter as the instance number — instance 0 of the first CF_ tag, instance
// N of the Nth regardless of name. The 0 was clamped, the walk usually broke
// on the nearest slot host before going out of range, and the bug survived
// for years — until a page where no slot matched early walked on to request
// instance 3 of a once-present layout tag and threw. These semantics are
// load-bearing for any fragment/slot tag library; an engine relaxing or
// tightening them changes real ancestor walks.
//
// Measurement footnote (observed while writing this, NOT pinned here): the
// throw is invisible inside a chained expression on Lucee —
// `getBaseTagData(...).attributes.marker ?: "x"` yields "x" (the elvis
// swallows the error), while RustCFML propagates it. That elvis-scope
// difference is a separate compatibility topic.

suiteBegin("getBaseTagData() instance numbers: per-name, 1-based, nearest-first (Lucee-measured)");
</cfscript>

<!--- ── A: single occurrence of the target name in the ancestry ── --->
<cfset structDelete(request, "btinst_dup") />
<cfset request.bti_err = "" />
<cftry>
    <cf_basetag_nest marker="solo-1" inner="true"></cf_basetag_nest>
    <cfcatch type="any"><cfset request.bti_err = "THREW: " & cfcatch.message /></cfcatch>
</cftry>

<cfscript>
assert( "A: probe completes", request.bti_err EQ "" ? "ok" : request.bti_err, "ok" );
p = request.btinst_dup ?: {};
assert( "A: default instance resolves the (single) nearest instance", p.inst_default ?: "(missing)", "solo-1" );
assert( "A: instance 0 is clamped to 1", p.inst_0 ?: "(missing)", "solo-1" );
assert( "A: instance 1 explicit", p.inst_1 ?: "(missing)", "solo-1" );
assertTrue( "A: instance 2 of a once-present name throws can't-find (saw: " & ( p.inst_2 ?: "(missing)" ) & ")",
    findNoCase( "can't find base tag", p.inst_2 ?: "" ) GT 0 );
</cfscript>

<!--- ── B: the target name appears TWICE (nested same-name hosts) ── --->
<cfset structDelete(request, "btinst_dup") />
<cfset request.bti_err2 = "" />
<cftry>
    <cf_basetag_nest marker="nest-outer"></cf_basetag_nest>
    <cfcatch type="any"><cfset request.bti_err2 = "THREW: " & cfcatch.message /></cfcatch>
</cftry>

<cfscript>
assert( "B: nested probe completes", request.bti_err2 EQ "" ? "ok" : request.bti_err2, "ok" );
p = request.btinst_dup ?: {};
dupCount = listValueCountNoCase( p.list ?: "", "CF_BASETAG_NEST" );
assertTrue( "B: ancestry contains the nest tag twice (saw: " & ( p.list ?: "(none)" ) & ")", dupCount EQ 2 );
assert( "B: instance 1 is the NEAREST instance", p.inst_1 ?: "(missing)", "nest-inner" );
assert( "B: default = instance 1 (nearest)", p.inst_default ?: "(missing)", "nest-inner" );
assert( "B: instance 2 is the next one out", p.inst_2 ?: "(missing)", "nest-outer" );
assertTrue( "B: instance 3 of a twice-present name throws can't-find (saw: " & ( p.inst_3 ?: "(missing)" ) & ")",
    findNoCase( "can't find base tag", p.inst_3 ?: "" ) GT 0 );

structDelete(request, "btinst_dup");
structDelete(request, "bti_err");
structDelete(request, "bti_err2");

suiteEnd();
</cfscript>
