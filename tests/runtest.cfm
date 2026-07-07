<!---
  Per-test isolation custom tag.

  <cf_runtest file="category/test_x.cfm">

  Runs one test file in the tag's OWN variables scope, so a test's unscoped
  page-level writes (e.g. `thread = "x"`, stray globals) cannot leak into the
  runner's page scope and pollute later tests. Shared test state (the pass/fail
  counters) lives in the `request` scope, which DOES cross the tag boundary, so
  totals still accumulate across every test.

  harness.cfm is re-included here so assert()/suiteBegin()/suiteEnd() are visible
  in this isolated scope; it is idempotent (counters init once per request), so
  re-including never resets the running totals.
--->
<cfif thisTag.executionMode eq "start">
    <cfinclude template="harness.cfm">
    <cftry>
        <cfinclude template="#attributes.file#">
        <cfcatch type="any"><cfset suiteAbort(attributes.file, cfcatch.message)></cfcatch>
    </cftry>
</cfif>
