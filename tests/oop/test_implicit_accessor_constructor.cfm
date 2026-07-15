<cfscript>
suiteBegin( "Implicit accessor constructor (accessors=true, no init)" );

// Named args populate declared properties via the generated getter backing.
named = new oop.AccessorDto( api="/v1", uri="/some/test/", verb="OPTIONS" );
assert( "named arg -> getApi", named.getApi(), "/v1" );
assert( "named arg -> getUri", named.getUri(), "/some/test/" );
assert( "named arg -> getVerb", named.getVerb(), "OPTIONS" );

// Unprovided properties keep their declared defaults.
partial = new oop.AccessorDto( api="/only-api" );
assert( "provided prop set", partial.getApi(), "/only-api" );
assert( "unprovided prop keeps default", partial.getUri(), "/" );

// argumentCollection spread populates the same way.
ac = new oop.AccessorDto( argumentCollection={ api="/v2", uri="/another/" } );
assert( "argumentCollection -> getApi", ac.getApi(), "/v2" );
assert( "argumentCollection -> getUri", ac.getUri(), "/another/" );

// Positional args are NOT mapped to properties (Lucee 7 verified).
positional = new oop.AccessorDto( "/posApi", "/posUri", "POSV" );
assert( "positional does not populate", positional.getApi(), "/" );

// A property whose name collides with a method: the property value is set
// (getter returns it) but the method stays callable.
collide = new oop.AccessorDto( flag=true );
assert( "colliding property getter", collide.getFlag(), true );
assert( "colliding method still callable", collide.flag(), "FLAG-METHOD" );

// An explicit init() takes over entirely — no implicit population.
withInit = new oop.AccessorInit( api="/should-be-ignored" );
assert( "explicit init wins over implicit population", withInit.getApi(), "from-init" );

// No accessors -> no implicit population; the default stands.
plain = new oop.PlainDto( api="/v1" );
assert( "non-accessor component is not populated", plain.readApi(), "/" );

// GH #266: the implicit accessor constructor maps INHERITED (parent-declared)
// properties too, not just the component's own.
child = new oop.AccessorInheritChild( foo="cf", bar="cb", baz="cz" );
assert( "inherited property populated (foo)", child.getFoo(), "cf" );
assert( "inherited property populated (bar)", child.getBar(), "cb" );
assert( "own property populated (baz)",       child.getBaz(), "cz" );
// Inherited default still applies when not passed.
childDef = new oop.AccessorInheritChild( baz="only" );
assert( "inherited default retained", childDef.getFoo(), "D" );

// GH #267: serializeJSON includes accessor-property values that live only in
// the private variables scope — inherited and default-only properties.
j = deserializeJSON( serializeJSON( child ) );
assert( "serializeJSON includes inherited foo", j.foo, "cf" );
assert( "serializeJSON includes inherited bar", j.bar, "cb" );
assert( "serializeJSON includes own baz",       j.baz, "cz" );
// Default-only inherited property (never explicitly set) is serialized too.
jd = deserializeJSON( serializeJSON( childDef ) );
assert( "serializeJSON includes default-only foo", jd.foo, "D" );

suiteEnd();
</cfscript>
