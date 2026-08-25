<!--- Fixture for tests/tags/test_customtag_hyphenated_attributes.cfm: record
      the attribute names (sorted) and the value of the hyphenated ones in
      request scope. Emits nothing. Lives at the custom-tag-path ROOT so it
      resolves on both engines without deep search. --->
<cfif thisTag.executionMode eq "start">
    <cfset request.ctpathAttrKeys = listSort(structKeyList(attributes), "textnocase") />
    <cfset request.ctpathAttrXref = structKeyExists(attributes, "x-ref") ? attributes["x-ref"] : "(absent)" />
    <cfset request.ctpathAttrData = structKeyExists(attributes, "data-foo") ? attributes["data-foo"] : "(absent)" />
</cfif>
