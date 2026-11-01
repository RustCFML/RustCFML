<cfscript>
// Runs ONLY the cfthread cap-eviction test, which needs RUSTCFML_TASKS_LIMIT=3
// in the engine's environment and so cannot run inside tests/runner.cfm.
// CI runs this page both from the CLI and in serve mode.
include "harness.cfm";
include "tags/test_cfthread_cap_eviction.cfm";
printSummary();
</cfscript>
