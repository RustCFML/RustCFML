<cfscript>
suiteBegin("Tags: hyphenated attribute names on a custom tag are attribute keys, not expressions");
</cfscript>

<!---
    ============================================================
    Background
    ============================================================
    <cf_name id="m" x-ref="r1"> passes an attribute literally named "x-ref"
    (attributes["x-ref"] = "r1") on Lucee 7.0.5 — for the prefix form and
    for <cfmodule name=> alike. HTML-flavoured names (x-ref, x-data,
    data-foo, aria-label) are common on tags that wrap markup, and the tag
    reads them with attributes["x-data"] / cfparam name="attributes['x-data']".

    RustCFML parses the name as an expression: `x-ref` becomes `x - ref` and
    the call throws "Variable 'x' is undefined" before the tag runs. The
    throw is catchable, so each leg is pinned under cftry.

    Repro class (titan/moopa): <cf_modal id="modal_cost_line" title="Cost
    Line" x-ref="modal_cost_line"> — the whole route 500s on RustCFML with
    a stack line that points nowhere near the attribute.

    Fixture: tests/tags/ctpathroot/ctpath_attr_echo.cfm (at the custom-tag-
    path ROOT declared in tests/Application.cfc) records the attribute names
    and the hyphenated values in request scope; no output capture involved.
    ============================================================
--->

<cffunction name="attrResult" output="false">
    <cfreturn structKeyExists(request, "ctpathAttrKeys")
        ? request.ctpathAttrKeys & " | x-ref=" & request.ctpathAttrXref & " | data-foo=" & request.ctpathAttrData
        : "(tag did not run)">
</cffunction>
<cffunction name="resetAttrResult" output="false">
    <cfset structDelete(request, "ctpathAttrKeys")>
    <cfset structDelete(request, "ctpathAttrXref")>
    <cfset structDelete(request, "ctpathAttrData")>
</cffunction>

<!--- Control: plain attribute names only — the fixture and the path resolve. --->
<cfset resetAttrResult()>
<cftry>
    <cf_ctpath_attr_echo id="m" title="Plain" />
    <cfcatch type="any"><cfset request.ctpathAttrKeys = "THREW: " & cfcatch.message><cfset request.ctpathAttrXref = ""><cfset request.ctpathAttrData = ""></cfcatch>
</cftry>
<cfscript>
assert("control: plain attribute names reach the tag", attrResult(), "id,title | x-ref=(absent) | data-foo=(absent)");
</cfscript>

<!--- GAP 1: x-ref on the prefix form. --->
<cfset resetAttrResult()>
<cftry>
    <cf_ctpath_attr_echo id="m" x-ref="r1" />
    <cfcatch type="any"><cfset request.ctpathAttrKeys = "THREW: " & cfcatch.message><cfset request.ctpathAttrXref = ""><cfset request.ctpathAttrData = ""></cfcatch>
</cftry>
<cfscript>
assert("<cf_name x-ref=...>: hyphenated name is an attribute key, value intact", attrResult(), "id,x-ref | x-ref=r1 | data-foo=(absent)");
</cfscript>

<!--- GAP 2: same through <cfmodule name=>. --->
<cfset resetAttrResult()>
<cftry>
    <cfmodule name="ctpath_attr_echo" id="m" x-ref="r2" />
    <cfcatch type="any"><cfset request.ctpathAttrKeys = "THREW: " & cfcatch.message><cfset request.ctpathAttrXref = ""><cfset request.ctpathAttrData = ""></cfcatch>
</cftry>
<cfscript>
assert("<cfmodule name= x-ref=...>: hyphenated name is an attribute key, value intact", attrResult(), "id,x-ref | x-ref=r2 | data-foo=(absent)");
</cfscript>

<!--- GAP 3: a data-* name — not an Alpine special case, any hyphen. --->
<cfset resetAttrResult()>
<cftry>
    <cf_ctpath_attr_echo id="m" data-foo="d" />
    <cfcatch type="any"><cfset request.ctpathAttrKeys = "THREW: " & cfcatch.message><cfset request.ctpathAttrXref = ""><cfset request.ctpathAttrData = ""></cfcatch>
</cftry>
<cfscript>
assert("<cf_name data-foo=...>: any hyphenated name, not just x-*", attrResult(), "data-foo,id | x-ref=(absent) | data-foo=d");
suiteEnd();
</cfscript>
