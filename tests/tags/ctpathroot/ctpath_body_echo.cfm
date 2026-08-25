<!--- Fixture for tests/tags/test_customtag_prefix_body_output.cfm: record
      exactly what the tag received as generatedContent in
      request.ctpathBodyEcho and emit nothing (the caller asserts on the
      request key, so no output capture is involved — cfsavecontent around
      a custom tag inside a function drops the tag's output on BOTH engines,
      which would mask the result). Lives at the custom-tag-path ROOT so it
      resolves on both engines without deep search. --->
<cfif thisTag.executionMode eq "end">
    <cfset request.ctpathBodyEcho = trim(thisTag.generatedContent) />
    <cfset thisTag.generatedContent = "" />
</cfif>
