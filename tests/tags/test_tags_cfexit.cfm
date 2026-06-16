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

<cfscript>suiteEnd();</cfscript>
