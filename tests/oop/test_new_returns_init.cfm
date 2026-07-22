<cfscript>
// `new X()` desugars to `createObject("component","X").init()` and must return
// init()'s RETURN VALUE when non-null (Lucee/ACF parity, verified against Lucee
// 7.0.4). This unblocked FW/1's frameworkFacadeTest — the facade's init() returns
// `request._fw1.theFramework`, so `new framework.facade()` must yield the
// framework object (which has getBeanFactory), not an empty facade.
suiteBegin( "new X() returns init()'s return value (Lucee parity)" );

// 1. init() returns a DIFFERENT object → new X() hands that object back.
other = new oop.NRIOther();
assert( "new returns init's foreign object", isStruct( other ) AND structKeyExists( other, "marker" ), true );
assert( "foreign object value is intact", other.marker, "FROM_INIT" );

// 2. init() returns `this` after mutating this/variables/accessors → all survive
// (result shares the instance's Arc; the common case is unchanged).
t = new oop.NRIThis();
assert( "return this: public member",  t.getPub(),  "PUB" );
assert( "return this: private member", t.getPriv(), "PRIV" );
assert( "return this: accessor value", t.getTag(),  "TAG" );

// 3. void init() (no explicit return) → the instance is returned, with its
// variables-scope mutations intact.
v = new oop.NRIVoid();
assert( "void init returns the instance", v.getSeed(), "SEED" );

suiteEnd();
</cfscript>
