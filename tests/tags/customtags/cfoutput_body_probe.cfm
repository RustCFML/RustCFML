<!--- Fixture for GH #339. A minimal body-executing custom tag: the end phase
      simply re-emits whatever the body generated, so a test can assert on
      whether the body's #expr# were interpolated by the CALLER's cfoutput. --->
<cfif thisTag.executionMode EQ "end"><cfset thisTag.generatedContent = trim( thisTag.generatedContent )></cfif>
