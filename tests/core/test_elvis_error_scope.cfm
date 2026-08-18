<cfscript>
// Elvis (?:) error scope — a PARITY PIN, currently green on both engines.
//
// On Lucee the left side of `?:` is evaluated in a swallow-everything scope:
// not just null/undefined, but a genuinely THROWN exception anywhere in the
// chain yields the default. Measured identical on RustCFML v0.607.0 — but it
// has not always been: at v0.595.1 a thrown left side PROPAGATED here
// (observed during the getBaseTagData work, PR #323's measurement footnote:
// `getBaseTagData(...).attributes.marker ?: "x"` threw on RustCFML while
// Lucee returned "x"). The behaviour converged silently somewhere in
// v0.596–v0.607; this suite pins it so any future change is deliberate.
//
// Real-world weight: defensive one-liners lean on this scope constantly —
// `cfg().setting ?: default` is written assuming a failing cfg() means
// "use the default", not "500".

suiteBegin("elvis (?:) swallows thrown left sides, not just null/undefined (Lucee parity)");

function boomFn() { throw(message="boom"); }

r1 = "(propagated)";
try { r1 = boomFn() ?: "dflt"; } catch (any e) { r1 = "THREW: " & e.message; }
assert( "a thrown function call yields the default", r1, "dflt" );

r2 = "(propagated)";
try { r2 = boomFn().member ?: "dflt"; } catch (any e) { r2 = "THREW: " & e.message; }
assert( "a throw inside a call-then-member chain yields the default", r2, "dflt" );

emptySt = {};
r3 = "(propagated)";
try { r3 = emptySt.a.b ?: "dflt"; } catch (any e) { r3 = "THREW: " & e.message; }
assert( "an undefined deep path yields the default (the documented case)", r3, "dflt" );

r4 = "(propagated)";
try { r4 = totally_unknown_fn_xyz() ?: "dflt"; } catch (any e) { r4 = "THREW: " & e.message; }
assert( "an unknown function name yields the default", r4, "dflt" );

suiteEnd();
</cfscript>
