<cfscript>
// Phase C.2.2 component-instance flyweight prototype.
//
// This is an ordinary component test: it passes on the default (marker-struct)
// build AND — run with a feature-`component-instance` binary plus
// RUSTCFML_INSTANCE_CLASSES=FlyweightProbe — against the compact flyweight
// backing. A green run in BOTH configurations is the prototype's correctness
// bar (see COMPONENT_MODEL_PHASE_C2_PROTOTYPE.md, measurement step 3).
suiteBegin("Flyweight Instance (C.2.2 prototype)");

obj = new oop.FlyweightProbe( "world" );

// --- construction: this + variables members set in the pseudo-constructor ---
assert("init returned this (greeting set)", obj.greeting, "hi world");
assert("public field direct read",          obj.publicField, "hello");
assert("public field read via method",      obj.readPublic(), "hello");

// --- method dispatch, incl. a private method called from a public one ---
assert("getName (variables read)", obj.getName(), "world");
assert("greet (private call)",     obj.greet(), "Hello, world");

// --- mutation of the variables scope through a method persists ---
assert("bump 1", obj.bump(), 1);
assert("bump 2", obj.bump(), 2);
assert("bump 3", obj.bump(), 3);

// --- mutation of the this scope through a method persists (both read paths) ---
obj.setField("changed");
assert("field after setField (method read)", obj.readPublic(), "changed");
assert("field after setField (direct read)", obj.publicField, "changed");

// --- direct this write persists (both read paths) ---
obj.publicField = "direct";
assert("field after direct write (method read)", obj.readPublic(), "direct");
assert("field after direct write (direct read)", obj.publicField, "direct");

// --- bracket-notation member read ---
assert("bracket read", obj["publicField"], "direct");

// --- fluent return-this chain still dispatches ---
assert("fluent chain", obj.setField("fluent").readPublic(), "fluent");

// --- introspection ---
assertTrue("isObject", isObject(obj));

// --- a second instance is independent (no shared data via the blueprint) ---
other = new oop.FlyweightProbe( "other" );
assert("second instance name",  other.getName(), "other");
assert("second instance counter fresh", other.bump(), 1);
assert("first instance counter untouched", obj.bump(), 4);

suiteEnd();
</cfscript>
