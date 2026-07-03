<cfscript>
suiteBegin("OOP: injected method binds to receiver scope, not its own (GH ##235)");

// GH #235: MockBox's MockGenerator copies its own `$include` method onto the
// target being mocked and invokes it there (targetObject.$include = ...; call;
// structDelete). That method carries MockGenerator's private `variables.instance`
// struct. When invoked as a method on the target, the receiver's `variables`
// must win — otherwise the generator's `variables.instance` overwrites the
// target's own `variables.instance`, wiping any state the target keeps in it
// (WireBox's Binder stores ALL config there → getProperties()/getCustomDSL()
// return null). This is the injected-method receiver-scope isolation guarantee.
// Reproduced self-contained (Injector235 mimics MockGenerator's `instance`).

t = new oop.mb235.Target235();
assert("target instance before injection", t.instanceKeys(), "customDSL,properties");

inj = new oop.mb235.Injector235();
inj.injectInto( t );

// the injected method ran bound to the target (set its marker on the target)
assertTrue("injected method executed against the target", t.wasInjected());
// ...but the injector's own variables.instance did NOT clobber the target's
assert("target instance survives injection", t.instanceKeys(), "customDSL,properties");
assertFalse("getProperties still works after injection", isNull( t.getProperties() ));
assertFalse("getCustomDSL still works after injection", isNull( t.getCustomDSL() ));
assert("getProperties value intact", t.getProperties().seeded, true);

// --------------------------------------------------------------------------
// GH #235 (reopened): the narrower case above (injectInto runs on its OWN home
// object) was already fixed, but the ACTUAL MockBox `$` path is different: the
// OUTER method is ITSELF injected onto the target and, while running there,
// makes a NESTED scope-prefixed call back into MockBox (`this.mbox.getX()`).
// The nested call's variables-writeback used to be trimmed to the `this` scope
// root and merged MockBox's own `variables.instance` onto the target — wiping
// the target's private state. Reproduce the real path.
t2 = new oop.mb235.Target235();
assert("real path: target instance before mock", t2.instanceKeys(), "customDSL,properties");

mb = new oop.mb235.MockBoxLike235().init();
mb.decorate( t2 );                                   // copies mb.$ onto t2, sets t2.mbox
assert("real path: instance intact after decorate", t2.instanceKeys(), "customDSL,properties");

r = t2.$( "processMappings" );                       // injected $, nested this.mbox call
assert("real path: mock invoked", r, "mocked processMappings via gen-object");
assert("real path: target instance survives .$()", t2.instanceKeys(), "customDSL,properties");
assertFalse("real path: getProperties still works after .$()", isNull( t2.getProperties() ));
assertFalse("real path: getCustomDSL still works after .$()", isNull( t2.getCustomDSL() ));
assert("real path: getProperties value intact", t2.getProperties().seeded, true);

suiteEnd();
</cfscript>
