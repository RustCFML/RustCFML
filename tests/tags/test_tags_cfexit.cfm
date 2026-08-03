<cfscript>suiteBegin("Tags: cfexit");</cfscript>

<cfset ltExitOut = "">
<cfset ltExitLog = "">
<cfset ltExitError = "">

<cftry>
    <cfsavecontent variable="ltExitOut"><cf_ltcfexit outVar="ltExitLog">BODY</cf_ltcfexit></cfsavecontent>
    <cfcatch type="any">
        <cfset ltExitError = cfcatch.message>
    </cfcatch>
</cftry>

<cfscript>
    assert('cfexit method="exittag" does not throw', ltExitError, "");
    assert('start-phase cfexit method="exittag" skips caller body output', trim(ltExitOut), "");
    assert('start-phase cfexit method="exittag" skips remaining tag code and end phase', ltExitLog, "[start]");
</cfscript>

<!--- ---------------------------------------------------------------
      cfexit method="loop": the tag re-executes its own BODY.
      Reference behaviour measured on Lucee 7.0.4: the start phase runs
      once, the body is re-executed per iteration, thisTag state carries
      across iterations, and generatedContent holds only the CURRENT
      iteration's body output.
      --------------------------------------------------------------- --->
<cfset loopLog = "">
<cfset loopCount = 0>
<cfset loopBodyRuns = 0>
<cfset loopError = "">
<cfset loopOut = "">

<cftry>
    <cfsavecontent variable="loopOut"><cf_ltcfexitloop outVar="loopLog" countVar="loopCount" label="L1"><cfset loopBodyRuns = loopBodyRuns + 1><cfoutput>body#loopBodyRuns#</cfoutput></cf_ltcfexitloop></cfsavecontent>
    <cfcatch type="any">
        <cfset loopError = cfcatch.message>
    </cfcatch>
</cftry>

<cfscript>
    assert('cfexit method="loop" does not throw', loopError, "");
    assert('cfexit method="loop" re-executes the tag body', loopBodyRuns, 3);
    assert('cfexit method="loop" runs the end phase once per iteration', loopCount, 3);
    // generatedContent is per-iteration (never accumulated), thisTag members
    // carry across iterations, attributes and executionMode stay stable, and
    // the start phase appears exactly once.
    assert('cfexit method="loop" iteration detail',
        loopLog,
        "[start:L1]"
        & "[end1 gc=body1 carried=S0 mode=end attr=L1]"
        & "[end2 gc=body2 carried=S1 mode=end attr=L1]"
        & "[end3 gc=body3 carried=S2 mode=end attr=L1]");
</cfscript>

<!--- The point of the feature: the CALLER's page must receive every
      iteration's body output, in order, each followed by that iteration's
      end-phase output. This tag does not clear generatedContent, so the
      rendered result is asserted directly rather than via a side channel.
      An alias kept from the start phase must also observe the current phase
      (thisTag is refreshed in place, not replaced). --->
<cfset outCount = 0>
<cfset outRuns = 0>
<cfset aliasTag = "">
<cfset outRendered = "">
<cfsavecontent variable="outRendered"><cf_ltcfexitloopout countVar="outCount" aliasVar="aliasTag"><cfset outRuns = outRuns + 1><cfoutput>body#outRuns#</cfoutput></cf_ltcfexitloopout></cfsavecontent>

<cfscript>
    // Whitespace-stripped: the tag template's own newlines are irrelevant, the
    // point is that every iteration's body reached the page, in order, each
    // followed by that iteration's end-phase output.
    assert('cfexit method="loop" renders every iteration to the page',
        reReplace(outRendered, "[[:space:]]+", "", "all"),
        "body1(e1)body2(e2)body3(e3)");
    assert('cfexit method="loop" body ran once per iteration', outRuns, 3);
    // thisTag was refreshed in place, so an alias taken during the start
    // phase sees the end phase and the latest member value.
    assert("thisTag alias observes the current executionMode",
        aliasTag.executionMode, "end");
    assert("thisTag alias observes later member updates", aliasTag.carried, "S3");
</cfscript>

<!--- Self-closing tags take a different engine path (no start/end pair):
      a loop request there simply re-runs the end phase. --->
<cfset selfLog = "">
<cfset selfCount = 0>
<cfset selfError = "">
<cfset selfOut = "">

<cftry>
    <cfsavecontent variable="selfOut"><cf_ltcfexitloopself outVar="selfLog" countVar="selfCount" /><cf_ltcfexitplain outVar="selfLog">AFTER</cf_ltcfexitplain></cfsavecontent>
    <cfcatch type="any">
        <cfset selfError = cfcatch.message>
    </cfcatch>
</cftry>

<cfscript>
    assert('self-closing cfexit method="loop" does not throw', selfError, "");
    assert('self-closing cfexit method="loop" repeats the end phase', selfCount, 3);
    assert('self-closing loop keeps thisTag state and lets the next tag run',
        selfLog,
        "[start][end1 carried=S0][end2 carried=S1][end3 carried=S2]"
        & "[second-start][second-end:AFTER]");
</cfscript>

<!--- Nested body-mode tags, BOTH looping: the rewind target is per-tag
      state, so an inner loop must not disturb the outer one. --->
<cfset nestLog = "">
<cfset nestOuter = 0>
<cfset nestInner = 0>
<cfset nestError = "">
<cfset nestOut = "">

<cftry>
    <cfsavecontent variable="nestOut"><cf_ltcfexitloop outVar="nestLog" countVar="nestOuter" label="OUT"><cf_ltinnerloop outVar="nestLog" countVar="nestInner"></cf_ltinnerloop></cf_ltcfexitloop></cfsavecontent>
    <cfcatch type="any">
        <cfset nestError = cfcatch.message>
    </cfcatch>
</cftry>

<cfscript>
    assert('nested cfexit method="loop" does not throw', nestError, "");
    assert('nested loop: outer tag iterates 3 times', nestOuter, 3);
    // The inner tag loops while its own count is odd: 1->loop->2 stop,
    // 3->loop->4 stop, 5->loop->6 stop. Three outer iterations, two inner
    // runs each.
    assert('nested loop: inner tag iterates independently', nestInner, 6);
</cfscript>

<!--- The operand stack must stay balanced across a rewind: ordinary
      expressions after a looping tag still evaluate correctly. --->
<cfset stackLog = "">
<cfset stackCount = 0>
<cfset stackOut = "">
<cfsavecontent variable="stackOut"><cf_ltcfexitloop outVar="stackLog" countVar="stackCount" label="ST">x</cf_ltcfexitloop></cfsavecontent>
<cfset stackMath = (1 + 2) * 3>
<cfset stackList = listLen("a,b,c")>

<cfscript>
    assert("operand stack stays balanced after a looping tag (arithmetic)", stackMath, 9);
    assert("operand stack stays balanced after a looping tag (function call)", stackList, 3);
</cfscript>

<!--- A loop request from the START phase is invalid (Lucee throws). --->
<cfset startLoopError = "">
<cfset startLoopOut = "">
<cftry>
    <cfsavecontent variable="startLoopOut"><cf_ltcfexitloopstart>BODY</cf_ltcfexitloopstart></cfsavecontent>
    <cfcatch type="any">
        <cfset startLoopError = cfcatch.message>
    </cfcatch>
</cftry>

<cfscript>
    assertTrue('start-phase cfexit method="loop" is rejected', len(startLoopError) GT 0);
</cfscript>

<!--- An exit fired in an END phase must not leak into the next, unrelated
      tag: that tag's body and end phase still run. --->
<cfset leakLog = "">
<cfset leakError = "">
<cfset leakOut = "">
<cftry>
    <cfsavecontent variable="leakOut"><cf_ltcfexitendexit outVar="leakLog">FIRST</cf_ltcfexitendexit><cf_ltcfexitplain outVar="leakLog">SECOND</cf_ltcfexitplain></cfsavecontent>
    <cfcatch type="any">
        <cfset leakError = cfcatch.message>
    </cfcatch>
</cftry>

<cfscript>
    assert("end-phase cfexit does not throw", leakError, "");
    assert("end-phase cfexit does not leak into the next tag",
        leakLog, "[first-end][second-start][second-end:SECOND]");
</cfscript>

<cfscript>suiteEnd();</cfscript>
