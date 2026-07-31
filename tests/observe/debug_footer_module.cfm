<!--- Fixture for tests/observe/test_debug_footer.cfm: a <cfmodule>/custom-tag
      target whose execution must surface as a `pages` (Execution Time) row.
      Lucee's Execution Time section covers "templates, includes, modules,
      custom tags, and component method calls"; RustCFML recorded every category
      except modules/custom tags until this was instrumented. --->
<cfoutput>[module ran]</cfoutput>
