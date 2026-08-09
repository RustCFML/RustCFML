<!---
    Gap custom tag: lives in a SUBDIRECTORY of the this.customtagpaths
    directory declared in tests/Application.cfc. Lucee resolves it when
    custom-tag deep search is on (engine config customTagDeepSearch=true --
    see test_customtag_path_deep_search.cfm); RustCFML searches only the
    path root and throws "Custom tag 'cf_ctpath_deep' not found".
--->
<cfif thisTag.executionMode eq "start">
    <cfoutput>deep-hello #attributes.name#</cfoutput>
</cfif>
