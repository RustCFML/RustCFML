<cfscript>suiteBegin("Tags: cfsleep");</cfscript>

<cfset sleepMarker = "">
<cfset sleepError = "">

<cftry>
    <cf_ltsleep time="1" outVar="sleepMarker" />
    <cfcatch type="any">
        <cfset sleepError = cfcatch.message>
    </cfcatch>
</cftry>

<cfscript>
    assert("cfsleep does not throw", sleepError, "");
    assert("cfsleep continues executing the current template", sleepMarker, "slept");
</cfscript>

<cfscript>suiteEnd();</cfscript>
