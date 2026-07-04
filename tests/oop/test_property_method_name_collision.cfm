<cfscript>
suiteBegin("Property / same-named-method collision (WireBox Binder shape)");

// Regression (WireBox port, Phase-5 providers blocker). A CFC with both a
// `property name="config"` accessor and a same-named method `config()`, whose
// pseudo-constructor assigns `variables.config = {...}` (via reset()). Two engine
// bugs combined:
//   #1 VM component assembly let the same-named method clobber the
//      pseudo-constructor's `variables.config` assignment (method install ran
//      AFTER the pseudo-constructor instead of before, Lucee-style).
//   #2 the auto-generated getter read `this.config` — where the PUBLIC method
//      lives — instead of the variables backing, so getConfig() returned the
//      method (empty) even once #1 was fixed.
// This is exactly coldbox Binder.scopeRegistration: getScopeRegistration()
// returned {} on RustCFML vs the populated defaults on Lucee, which silently
// disabled scope registration and broke provider:/provider-method resolution.

obj = new PropMethodCollision();

// getter reads the variables backing, populated by the pseudo-constructor
cfg = obj.getConfig();
assertTrue( "getConfig() returns a struct", isStruct( cfg ) );
assertTrue( "getConfig() has the pseudo-ctor 'enabled' key", structKeyExists( cfg, "enabled" ) );
assert(     "getConfig().enabled", cfg.enabled, true );
assert(     "getConfig().mode",    cfg.mode, "live" );

// the same-named method must still be callable
obj.config( "alpha" );
assert( "method config() mutated the backing", obj.getConfig().lastKey, "alpha" );

suiteEnd();

// ---------------------------------------------------------------------------
suiteBegin("Property / method collision survives inheritance");

child = new PropMethodCollisionChild();
childCfg = child.getConfig();
assertTrue( "child getConfig() is a struct", isStruct( childCfg ) );
assert(     "child getConfig().enabled", childCfg.enabled, true );
assert(     "child getConfig().mode",    childCfg.mode, "live" );
child.config( "beta" );
assert( "child method config() callable", child.getConfig().lastKey, "beta" );

suiteEnd();

// ---------------------------------------------------------------------------
suiteBegin("Property / method collision — generated setter must not clobber method");

// Regression (Preside/Sticker util/Asset.cfc). A CFC with `property name="after"`
// AND a method `after()`; the value is written via the AUTO-GENERATED `setAfter()`
// at runtime. The generated setter used to write `this.after = value`, clobbering
// the same-named method so `this.after()` / bare `after()` / external `obj.after()`
// all threw "Component has no function with name [after]". Fix: the generated
// setter writes ONLY the `variables` backing when a same-named method exists; the
// method stays callable, getAfter() returns the value. (Lucee parity.)

a = new PropMethodSetterCollision();  // init() calls setAfter([]) (runtime setter)

// external method call still works after the runtime setter ran
a.after( "x" );
assertTrue( "getAfter() returns an array (value, not the method)", isArray( a.getAfter() ) );
assert(     "external after('x') mutated backing", a.getAfter()[ 1 ], "x" );

// internal this.method() call (Asset.dependsOn shape)
a.callInternal( "y" );
assert( "this.after('y') callable internally", a.getAfter()[ 2 ], "y" );
assert( "getAfter() length after two appends", arrayLen( a.getAfter() ), 2 );

suiteEnd();
</cfscript>
