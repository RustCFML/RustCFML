<!---
	<cfexit method="loop"> — a custom tag re-executing its own body (GH #310).

	Every expectation here was measured against Lucee 7.0.4.34 before the engine
	side was written; the suite is intended to pass unchanged on both engines.

	Whitespace inside a custom tag template is engine-visible, so output is
	compared with all whitespace stripped rather than byte-for-byte.
--->
<cfimport taglib="cfexit_loop_tags" prefix="x">
<cfscript>
	suiteBegin("Tags: cfexit method=loop (custom tag body iteration)");

	function celSquash(required string s) {
		return reReplace(arguments.s, "[\s]", "", "all");
	}
</cfscript>

<!--- The body re-executes once per loop request; the START phase runs ONCE. --->
<cfset request.cel_start = 0>
<cfset request.cel_end = "">
<cfset request.cel_gc = "">
<cfset celRow = "none">
<cfsavecontent variable="celA"><x:looper><cfoutput>(#celRow#)</cfoutput></x:looper></cfsavecontent>
<cfscript>
	assert("body re-executes once per loop request", celSquash(celA), "(none)(r1)(r2)");
	assert("start phase runs exactly once", request.cel_start, 1);
	assert("end phase runs once per iteration", request.cel_end, "123");
</cfscript>

<!---
	generatedContent holds THIS iteration's body output only — it does not
	accumulate — and the tag's `variables` scope persists across iterations
	(the counter above reached 3), while caller writes made by one end phase
	are visible to the next body pass (the (r1)/(r2) above).
--->
<cfscript>
	assert("generatedContent is per-iteration, not cumulative", celSquash(request.cel_gc), "[(none)][(r1)][(r2)]");
	assert("caller write from the final end phase persists", celRow, "r3");
</cfscript>

<!--- Emission order per iteration: generatedContent, then the end phase's own output. --->
<cfset request.cel_g = 0>
<cfsavecontent variable="celB"><x:rewrite>B</x:rewrite></cfsavecontent>
<cfscript>
	assert("rewritten generatedContent precedes end-phase output, per iteration",
	       celSquash(celB), "<B>{e}<B>{e}");
</cfscript>

<!--- thisTag survives the phase flip, and an alias to it observes the current phase. --->
<cfset request.cel_mine = "">
<cfset request.cel_alias = "">
<cfsavecontent variable="celC"><x:thistag>b</x:thistag></cfsavecontent>
<cfscript>
	assert("a member the tag set on thisTag survives start -> end", request.cel_mine, "S0");
	assert("an alias kept to thisTag observes the current phase", request.cel_alias, "end");
</cfscript>

<!--- method="loop" is only legal in an end phase. --->
<cfset request.cel_sloop = "">
<cfset celThrew = false>
<cfset celMsg = "">
<cftry>
	<x:badloop>b</x:badloop>
	<cfcatch type="any">
		<cfset celThrew = true>
		<cfset celMsg = cfcatch.message>
	</cfcatch>
</cftry>
<cfscript>
	assert("cfexit method=loop in a start phase throws", celThrew, true);
	assert("...with Lucee's message", celMsg contains "method loop can only be used", true);
	assert("the start phase did run before throwing", request.cel_sloop, "startphase");
</cfscript>

<!---
	A <cfbreak> in a tag body binds to the CALLER's loop, not the tag: the
	captured body content is discarded, the end phase never runs, and — the
	regression this guards — the page keeps rendering afterwards. Before the
	fix the tag's capture buffer leaked and silently swallowed the rest of the
	page.
--->
<cfset request.cel_brkend = "">
<cfsavecontent variable="celE"><cfloop from="1" to="3" index="celN"><x:plain>(body#celN#)<cfbreak></x:plain></cfloop>TAIL</cfsavecontent>
<cfscript>
	assert("cfbreak in a tag body discards the body content", celSquash(celE), "TAIL");
	assert("cfbreak in a tag body skips the end phase", request.cel_brkend, "");
	assert("cfbreak in a tag body leaves the page rendering", celE contains "TAIL", true);

	suiteEnd();
</cfscript>
