<cfscript>suiteBegin("Tags: cfhtmlhead / cfhtmlbody");</cfscript>

<cfset htmlInjectionMarker = "">
<cfset htmlInjectionError = "">

<cftry>
    <cf_lthtmlinjection outVar="htmlInjectionMarker" />
    <cfcatch type="any">
        <cfset htmlInjectionError = cfcatch.message>
    </cfcatch>
</cftry>

<cfscript>
    assert("cfhtmlhead/cfhtmlbody do not throw", htmlInjectionError, "");
    assert("cfhtmlhead/cfhtmlbody continue executing the current template", htmlInjectionMarker, "ran");
</cfscript>

<cfscript>suiteEnd();</cfscript>
