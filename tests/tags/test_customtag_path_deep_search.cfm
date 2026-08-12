<cfscript>
suiteBegin("Tags: custom tag path deep search (tag in a subdirectory)");
</cfscript>

<!---
    ============================================================
    Background
    ============================================================
    A custom tag file in a SUBDIRECTORY of a custom tag path must resolve:
    with this.customtagpaths pointing at dir X and the tag file at
    X/nested/hello.cfm, <cf_hello> executes. Lucee searches the declared
    custom tag paths RECURSIVELY when deep search is enabled
    (customTagDeepSearch=true in the engine config; the per-application
    this.customTagDeepSearch spelling is inert on Lucee 7 -- verified). Deep
    search is OFF in a stock Lucee install, but enabling it is how Lucee-based
    projects ship (e.g. titan's config.json sets "customTagDeepSearch":"true"
    and keeps app tags in nested per-app directories), so the cross-engine
    run for this suite needs that engine flag on.

    RustCFML resolves a tag at the ROOT of this.customtagpaths (the control
    below passes today) but never descends into subdirectories: the nested
    tag throws "Custom tag 'cf_ctpath_deep' not found". The throw is
    catchable, so the gap is pinned inline under cftry and registration is
    runner-safe.

    Fixtures: tests/Application.cfc declares
        this.customtagpaths = <tests dir> & "tags/ctpathroot/"
    with the control tag at ctpathroot/ctpath_shallow.cfm and the gap tag at
    ctpathroot/nested/ctpath_deep.cfm. The test file itself lives in
    tests/tags/, so neither tag is findable by caller-relative search --
    resolution can only come through the declared custom tag path.

    Reduced from the titan (Moopa) codebase port: its custom tag path points
    at an app's tags/ dir whose tags are organised in subdirectories.
    ============================================================
--->

<!--- Control: a tag at the custom-tag-path ROOT resolves today. --->
<cfset shallowOut = "">
<cfset shallowErr = "">
<cftry>
    <cfsavecontent variable="shallowOut"><cf_ctpath_shallow name="World"></cfsavecontent>
    <cfcatch type="any"><cfset shallowErr = cfcatch.message></cfcatch>
</cftry>
<cfscript>
assert("control: tag at the custom-tag-path root resolves", shallowErr, "");
assertTrue("control: root tag executed", findNoCase("shallow-hello World", shallowOut) GT 0);
</cfscript>

<!--- Gap: the same path with the tag one subdirectory down. --->
<cfset deepOut = "">
<cfset deepErr = "">
<cftry>
    <cfsavecontent variable="deepOut"><cf_ctpath_deep name="World"></cfsavecontent>
    <cfcatch type="any"><cfset deepErr = cfcatch.message></cfcatch>
</cftry>
<cfscript>
assert("tag in a custom-tag-path subdirectory resolves", deepErr, "");
assertTrue("subdirectory tag executed", findNoCase("deep-hello World", deepOut) GT 0);

suiteEnd();
</cfscript>
