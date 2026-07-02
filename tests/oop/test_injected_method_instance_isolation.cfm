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

suiteEnd();
</cfscript>
