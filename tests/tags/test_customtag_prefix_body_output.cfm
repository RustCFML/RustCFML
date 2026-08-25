<cfscript>
suiteBegin("Tags: a prefix-invoked custom tag body inherits the caller's output context");
</cfscript>

<!---
    ============================================================
    Background
    ============================================================
    The body of a custom tag is CFML executed in the CALLER's context: if the
    caller is inside <cfoutput>, or inside a <cffunction output="true">, then
    #expressions# in the body are evaluated before the tag sees them as
    thisTag.generatedContent. Lucee 7.0.5 does this identically for all three
    ways of invoking the tag: <cf_name>, <cfmodule name="name"> and
    <cfmodule template="...">.

    RustCFML evaluates the body for the two cfmodule forms but NOT for the
    prefix form: <cf_name>#x#</cf_name> hands the tag the literal text "#x#",
    which then reaches the browser. An inner <cfoutput> inside the body
    works around it, so the gap is specifically "the prefix path does not
    inherit the enclosing output context" — not "prefix tags can't output".

    Repro class (titan/moopa): a form control tag wraps its Alpine component
    script in <cf_once> (dedupe-once-per-request tag) inside the control's
    own <cfoutput>; the script builds image URLs from
    #server.system.environment.TWIC_PICS_URL#. On RustCFML every image link
    became https://host/page#server.system.environment.TWIC_PICS_URL/... —
    broken thumbnails and a "click to open" that navigated to a fragment.
    Same for every page whose <cffunction output="true"> body is a
    <cf_layout_default>...</cf_layout_default> block: any #expr# directly in
    the layout body renders literally. 19 sites in that one app.

    Fixture: tests/tags/ctpathroot/ctpath_body_echo.cfm (at the custom-tag-
    path ROOT declared in tests/Application.cfc) records what it received in
    request.ctpathBodyEcho and emits nothing, so the assertions need no
    output capture. Every leg is under cftry so a throw is asserted as a
    value rather than aborting the file.
    ============================================================
--->

<cfset probeValue = "VALUE">

<cffunction name="echoResult" output="false">
    <cfreturn structKeyExists(request, "ctpathBodyEcho") ? request.ctpathBodyEcho : "(tag did not run)">
</cffunction>

<!--- Control 1: <cfmodule name=> inside the caller's cfoutput. --->
<cfset structDelete(request, "ctpathBodyEcho")>
<cftry>
    <cfoutput><cfmodule name="ctpath_body_echo">#probeValue#</cfmodule></cfoutput>
    <cfcatch type="any"><cfset request.ctpathBodyEcho = "THREW: " & cfcatch.message></cfcatch>
</cftry>
<cfscript>
assert("control: cfmodule name= body evaluated inside caller's cfoutput", echoResult(), "VALUE");
</cfscript>

<!--- Control 2: <cfmodule template=> — same. --->
<cfset structDelete(request, "ctpathBodyEcho")>
<cftry>
    <cfoutput><cfmodule template="ctpathroot/ctpath_body_echo.cfm">#probeValue#</cfmodule></cfoutput>
    <cfcatch type="any"><cfset request.ctpathBodyEcho = "THREW: " & cfcatch.message></cfcatch>
</cftry>
<cfscript>
assert("control: cfmodule template= body evaluated inside caller's cfoutput", echoResult(), "VALUE");
</cfscript>

<!--- Control 3: prefix form with its OWN inner cfoutput — proves the prefix
      path can evaluate a body; only inheritance is at stake in the gap legs. --->
<cfset structDelete(request, "ctpathBodyEcho")>
<cftry>
    <cf_ctpath_body_echo><cfoutput>#probeValue#</cfoutput></cf_ctpath_body_echo>
    <cfcatch type="any"><cfset request.ctpathBodyEcho = "THREW: " & cfcatch.message></cfcatch>
</cftry>
<cfscript>
assert("control: <cf_name> body with its own inner cfoutput", echoResult(), "VALUE");
</cfscript>

<!--- GAP 1: prefix form inside the caller's <cfoutput>. --->
<cfset structDelete(request, "ctpathBodyEcho")>
<cftry>
    <cfoutput><cf_ctpath_body_echo>#probeValue#</cf_ctpath_body_echo></cfoutput>
    <cfcatch type="any"><cfset request.ctpathBodyEcho = "THREW: " & cfcatch.message></cfcatch>
</cftry>
<cfscript>
assert("<cf_name> body inherits the caller's enclosing cfoutput", echoResult(), "VALUE");
</cfscript>

<!--- GAP 2: prefix form inside a <cffunction output="true"> body (implicit
      output context — no cfoutput tag anywhere). --->
<cffunction name="renderViaPrefixTag" output="true"><cf_ctpath_body_echo>#probeValue#</cf_ctpath_body_echo></cffunction>
<cfset structDelete(request, "ctpathBodyEcho")>
<cftry>
    <cfset renderViaPrefixTag()>
    <cfcatch type="any"><cfset request.ctpathBodyEcho = "THREW: " & cfcatch.message></cfcatch>
</cftry>
<cfscript>
assert("<cf_name> body inherits a cffunction output=true context", echoResult(), "VALUE");
suiteEnd();
</cfscript>
