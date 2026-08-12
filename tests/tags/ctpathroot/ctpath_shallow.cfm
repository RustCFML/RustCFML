<!---
    Control custom tag: lives at the ROOT of the this.customtagpaths directory
    declared in tests/Application.cfc. RustCFML already resolves this, so it
    guards the custom-tag-path wiring for test_customtag_path_deep_search.cfm.
--->
<cfif thisTag.executionMode eq "start">
    <cfoutput>shallow-hello #attributes.name#</cfoutput>
</cfif>
