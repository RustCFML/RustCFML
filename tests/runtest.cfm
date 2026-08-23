<!---
  Per-test isolation custom tag.

  <cf_runtest file="category/test_x.cfm">
  <cf_runtest file="category/test_x.cfm" rustcfmlOnly="true">

  Runs one test file in the tag's OWN variables scope, so a test's unscoped
  page-level writes (e.g. `thread = "x"`, stray globals) cannot leak into the
  runner's page scope and pollute later tests. Shared test state (the pass/fail
  counters) lives in the `request` scope, which DOES cross the tag boundary, so
  totals still accumulate across every test.

  harness.cfm is re-included here so assert()/suiteBegin()/suiteEnd() are visible
  in this isolated scope; it is idempotent (counters init once per request), so
  re-including never resets the running totals.

  Add `why="..."` to say WHY in the SKIPPED block; without it the reason is
  the generic "RustCFML-only".

  `rustcfmlOnly="true"` marks a file that exercises a RustCFML extension,
  superset, or syntax Lucee's parser rejects. It runs normally here and is
  SKIPPED on other engines. The skip has to happen at this level rather than
  inside the file: a file Lucee cannot even compile is past saving by the time
  an `if (isRustCFML())` inside it would run.
--->
<cfif thisTag.executionMode eq "start">
    <cfinclude template="harness.cfm">
    <cfif structKeyExists(attributes, "rustcfmlOnly")
          AND attributes.rustcfmlOnly
          AND NOT isRustCFML()>
        <cfset suiteSkip( attributes.file
              , structKeyExists( attributes, "why" ) ? attributes.why
                                                     : "RustCFML-only; not applicable to this engine" )>
    <cfelse>
        <cftry>
            <cfinclude template="#attributes.file#">
            <cfcatch type="any"><cfset suiteAbort(attributes.file, cfcatch.message)></cfcatch>
        </cftry>
    </cfif>
</cfif>
