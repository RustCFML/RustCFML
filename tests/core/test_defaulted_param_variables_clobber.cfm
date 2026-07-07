<cfscript>
suiteBegin("Core: defaulted param must not clobber same-named variables entry");

// Regression: a component method with a defaulted parameter whose name collides
// with another component method. Calling it with the arg OMITTED runs the
// default-value preamble, which in classic localmode must seed the param into
// local/arguments — NOT the component `variables` scope. Before the fix the
// preamble's bare store landed in `variables`, permanently overwriting the
// same-named method; every later bare call to it program-wide then threw
// "Variable is not a function" (this broke every Wheels POST redirect via the
// `$args` trampoline). Confirmed against Lucee 7 (all assertions pass there).

probe = createObject("component", "DefaultParamClobberProbe");

// Sanity: bare call resolves before the collision is triggered.
assert("method resolves before collision", probe.callHelperBare(), "HELPER_METHOD");

// The defaulted param is visible as its own data inside the method.
assert("omitted defaulted param takes its default", probe.useHelper(), "defaulted");
assert("supplied param overrides default", probe.useHelper("passed"), "passed");

// THE FIX: running useHelper() (which applied the `helper` default) must not
// have clobbered the `helper` component method in the variables scope.
assert("method survives after defaulted-param call", probe.callHelperBare(), "HELPER_METHOD");

// Idempotent under repetition (the clobber, if present, was permanent).
probe.useHelper();
probe.useHelper();
assert("method still resolves after repeated calls", probe.callHelperBare(), "HELPER_METHOD");

suiteEnd();
</cfscript>
