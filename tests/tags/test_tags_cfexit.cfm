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

<!--- With no method, <cfexit> means method="exittag", as on Lucee and ACF.
      It used to default to exittemplate, so a bare <cfexit> in a custom tag's
      start phase ran the caller's body and the end phase anyway. Every
      expectation below was measured on Lucee 7.0.4.34. --->
<cfset ltBareOut = "">
<cfset ltBareLog = "">
<cfsavecontent variable="ltBareOut"><cf_ltcfexitbare outVar="ltBareLog">BODY</cf_ltcfexitbare></cfsavecontent>
<cfscript>
    assert('bare cfexit in the start phase skips the caller body', trim(ltBareOut), "");
    assert('bare cfexit in the start phase skips the rest of the tag and its end phase', ltBareLog, "[start]");
</cfscript>

<cfimport taglib="cfexit_default_tags" prefix="ltx">
<cfset ltImportOut = "">
<cfset ltImportLog = "">
<cfsavecontent variable="ltImportOut"><ltx:exitbare outVar="ltImportLog">BODY</ltx:exitbare></cfsavecontent>
<cfscript>
    assert('bare cfexit in a cfimport-prefixed tag skips the caller body', trim(ltImportOut), "");
    assert('bare cfexit in a cfimport-prefixed tag skips its end phase', ltImportLog, "[start]");
</cfscript>

<cfset ltModuleOut = "">
<cfset ltModuleLog = "">
<cfsavecontent variable="ltModuleOut"><cfmodule template="ltcfexitbare.cfm" outVar="ltModuleLog">BODY</cfmodule></cfsavecontent>
<cfscript>
    assert('bare cfexit in a cfmodule tag skips the caller body', trim(ltModuleOut), "");
    assert('bare cfexit in a cfmodule tag skips its end phase', ltModuleLog, "[start]");
</cfscript>

<cfset ltScriptOut = "">
<cfset ltScriptLog = "">
<cfsavecontent variable="ltScriptOut"><ltx:scriptexit outVar="ltScriptLog">BODY</ltx:scriptexit></cfsavecontent>
<cfscript>
    assert('a bare exit; statement in a tag start phase skips the caller body', trim(ltScriptOut), "");
    assert('a bare exit; statement in a tag start phase skips the rest of the tag and its end phase', ltScriptLog, "[start]");

    // Outside a custom tag the default behaves as exittemplate did: the
    // included template stops and the includer carries on.
    request._cfexitDefaultInclude = "";
    include "cfexit_default_include.cfm";
    request._cfexitDefaultInclude &= "[includer-after]";
    assert('bare cfexit in an included template returns to the includer', request._cfexitDefaultInclude, "[inc-before][includer-after]");

    request._cfexitDefaultInclude = "";
    include "cfexit_default_script_include.cfm";
    request._cfexitDefaultInclude &= "[includer-after]";
    assert('a bare exit; statement in an included template returns to the includer', request._cfexitDefaultInclude, "[inc-before][includer-after]");

    // `exit` is a soft keyword: it is still an ordinary variable name.
    exit = 5;
    assert('exit is still usable as a variable name', exit, 5);
</cfscript>

<cfscript>suiteEnd();</cfscript>
