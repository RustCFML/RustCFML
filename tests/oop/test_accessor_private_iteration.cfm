<cfscript>
suiteBegin("Accessor properties are private to introspection (Lucee parity)");

// Lucee stores accessor-`property` VALUES (set via the implicit accessor ctor or
// a generated setX) in the private `variables` scope: invisible to
// structKeyList/Count/Exists/for-in, but still returned by getX()/serializeJSON.
// This engine materialises them at the struct top level, so a per-instance marker
// hides them from introspection. Without this, TestBox's deep-compare `equalize`
// descended into cfflow mock instances whose accessor refs pointed back at them —
// an unbounded cycle (depth-256 guard). An explicit `this.x=` stays public.

o = new AccessorPrivateFixture();
o.setRef( { nested = "data" } );

// Accessor-set property (via generated setRef): hidden from introspection...
assertFalse("structKeyExists hides accessor prop 'ref'", structKeyExists( o, "ref" ));
keys = structKeyList( o );
assertFalse("structKeyList excludes 'ref'", listFindNoCase( keys, "ref" ) GT 0);

// ...but STILL readable via getX() and serializeJSON (value lives at the
// top level; only introspection/for-in consult the accessor-private marker).
assert("getRef reads accessor value", serializeJSON( o.getRef() ), '{"nested":"data"}');
assertTrue("serializeJSON still includes accessor 'ref'", findNoCase( "nested", serializeJSON( o ) ) GT 0);

// Explicit `this.x=` is a genuine public member — stays visible.
assertTrue("structKeyExists shows explicit this.publicFlag", structKeyExists( o, "publicFlag" ));

// for-in over the component yields only public methods + explicit this members.
forInKeys = "";
for ( k in o ) { forInKeys = listAppend( forInKeys, k ); }
assertFalse("for-in excludes accessor prop 'ref'", listFindNoCase( forInKeys, "ref" ) GT 0);
assertTrue("for-in includes explicit this.publicFlag", listFindNoCase( forInKeys, "publicFlag" ) GT 0);
assertTrue("for-in includes public method getRef", listFindNoCase( forInKeys, "getRef" ) GT 0);

// Cycle safety: an accessor ref pointing back at the owner must not make a
// deep walk (for-in) recurse — the property is not iterated.
o.setRef( o );
cycleKeys = "";
for ( k in o ) { cycleKeys = listAppend( cycleKeys, k ); }
assertFalse("cyclic accessor ref not iterated", listFindNoCase( cycleKeys, "ref" ) GT 0);

suiteEnd();
</cfscript>
