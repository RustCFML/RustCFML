<cfscript>
suiteBegin("Self-named default argument reads the enclosing variable (GitHub ##240)");

// A default-value expression that references a variable of the SAME NAME as the
// parameter must read that variable from the ENCLOSING (variables/local) scope,
// not the parameter's own not-yet-initialized slot. Pre-v0.423 the omitted
// param was pre-seeded as Null, which shadowed the outer variable so the default
// resolved to null. Lucee 5.4 is the oracle.
variables.sessionStorage = "OUTER-SS";
variables.other          = "OUTER-OTHER";

function sameName( sessionStorage = sessionStorage )           { return arguments.sessionStorage ?: "<null>"; }
function diffName( sessionStorage = other )                    { return arguments.sessionStorage ?: "<null>"; }
function scoped(   sessionStorage = variables.sessionStorage ) { return arguments.sessionStorage ?: "<null>"; }

assert("same-name default reads outer var",        sameName(), "OUTER-SS");
assert("diff-name default reads other outer var",  diffName(), "OUTER-OTHER");
assert("explicit variables.-qualified default",    scoped(),   "OUTER-SS");

// A supplied argument must still win over the default.
assert("supplied arg wins over self-named default", sameName("PASSED"), "PASSED");

// Cross-param defaults are unaffected: a later param's default may reference an
// earlier param by name (that earlier slot IS initialized by the time it runs).
function twoParams( a = "A", b = a & "-derived" ) { return arguments.b; }
assert("later default reads earlier param", twoParams(), "A-derived");
assert("later default with earlier supplied", twoParams("Z"), "Z-derived");

// The CFC-method form — the concrete Preside CsrfProtectionServiceTest idiom:
// setup() seeds variables.X, then a helper defaults `arg = X` and forwards the
// whole arguments struct via argumentCollection.
svc = new core.SelfNamedDefaultFixture();
assert("cfc self-named default forwards outer var", svc.run(), "SS-OBJ|5|false");
assert("cfc self-named default, one supplied",      svc.run(sessionStorage="OVERRIDE"), "OVERRIDE|5|false");

suiteEnd();
</cfscript>
