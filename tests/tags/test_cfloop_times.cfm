<cfscript>
suiteBegin("Tags: cfloop times");

// <cfloop times="N"> is a Lucee 5+ form. It was implemented in the SCRIPT-tag
// lowering (parser.rs) but NOT in the tag preprocessor, so the tag form fell
// through to the `while (true)` fallback and hung — no output, unbounded
// output-buffer growth (12 GB observed before the process was killed).
// These assertions pin both the counted behaviour and the edge cases.

out = "";
</cfscript>

<cfloop times="3"><cfset out = out & "X"></cfloop>
<cfscript>
assert("times=3 runs the body three times", out, "XXX");
</cfscript>

<cfset out = "">
<cfloop times="1"><cfset out = out & "X"></cfloop>
<cfscript>
assert("times=1 runs the body once", out, "X");
</cfscript>

<cfset out = "">
<cfloop times="0"><cfset out = out & "X"></cfloop>
<cfscript>
assert("times=0 does not run the body at all", out, "");
</cfscript>

<cfset out = "">
<cfset n = 4>
<cfloop times="#n#"><cfset out = out & "X"></cfloop>
<cfscript>
assert("times accepts a dynamic value", out, "XXXX");
</cfscript>

<cfset out = "">
<cfset n = 2>
<cfloop times="#n * 2#"><cfset out = out & "X"></cfloop>
<cfscript>
assert("times accepts an expression", out, "XXXX");
</cfscript>

<cfset out = "">
<cfloop times="2"><cfloop times="3"><cfset out = out & "X"></cfloop><cfset out = out & "|"></cfloop>
<cfscript>
assert("nested times loops keep independent counters", out, "XXX|XXX|");
</cfscript>

<cfset out = "">
<cfloop times="-2"><cfset out = out & "X"></cfloop>
<cfscript>
assert("a negative times runs the body zero times", out, "");
</cfscript>

<cfset out = "">
<cfloop times="2.7"><cfset out = out & "X"></cfloop>
<cfscript>
assert("a fractional times truncates toward zero", out, "XX");
</cfscript>

<cfscript>
// The bound is evaluated once, up front — not re-read on every iteration.
// Verified against Lucee 7.0.4: it hoists too, so mutating the source variable
// inside the body does not change the trip count on either engine.
out = "";
n = 3;
</cfscript>
<cfloop times="#n#"><cfset out = out & "X"><cfset n = 99></cfloop>
<cfscript>
assert("times bound is hoisted, not re-evaluated per iteration", out, "XXX");

suiteEnd();
</cfscript>
