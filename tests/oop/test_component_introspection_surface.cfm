<cfscript>
// Phase C.3 Slice-6 hardening: the standard-library / VM introspection surface
// must treat a COMPONENT uniformly regardless of backing (marker struct OR the
// `component-instance` flyweight). Each behaviour below silently mis-fired on a
// flyweight instance before the audit fix (the `_ => false`/`Struct`-only match
// arms that never matched a `CfmlValue::Instance`) — the same class of bug as the
// `IsDefined()` regression. Every assertion is identical on both RustCFML backings
// and on Lucee, so this file guards the whole surface in one place.
suiteBegin( "Component introspection / struct-BIF surface (flyweight parity)" );

o = new oop.FlyweightSurfaceProbe();

// --- type checks + isDefined through a component (the original IsDefined bug) ---
assert( "isObject(component)",              isObject( o ),                true );
assert( "structKeyExists public member",    structKeyExists( o, "pubData" ), true );
assert( "isDefined(comp.method)",           isDefined( "o.greet" ),       true );
assert( "isDefined(comp.dataMember)",       isDefined( "o.pubData" ),     true );
assert( "isDefined(comp.missing) is false", isDefined( "o.nope" ),        false );

// --- structDelete on a component (delete_scope_path + fn_struct_delete) ---
o.temp = "t";
assert( "member present before delete", structKeyExists( o, "temp" ), true );
structDelete( o, "temp" );
assert( "structDelete removes member",  structKeyExists( o, "temp" ), false );

// --- structInsert / structUpdate write in place (fn_struct_insert) ---
structUpdate( o, "pubData", "upd" );
assert( "structUpdate writes to component", o.pubData, "upd" );

// --- structCopy carries public members (fn_struct_copy) ---
cp = structCopy( o );
assert( "structCopy carries public member", structKeyExists( cp, "pubData" ), true );

// --- structFindKey / structFindValue (fn_struct_find_key/value) ---
o.uniqueMarker = "FINDME_UNIQUE_7";
assert( "structFindKey finds a component key",   arrayLen( structFindKey( o, "uniqueMarker" ) ) gt 0, true );
assert( "structFindValue finds a component val", arrayLen( structFindValue( o, "FINDME_UNIQUE_7" ) ) gt 0, true );

// --- structFind returns the member value (fn_struct_find) ---
assert( "structFind returns member value", structFind( o, "pubData" ), "upd" );

// --- structAppend into a component ---
structAppend( o, { appendedKey = "yes" } );
assert( "structAppend adds member", o.appendedKey, "yes" );

// --- serializeJSON / serialize include component data ---
assert( "serializeJSON includes data", serializeJSON( o ) contains "pubData", true );

// --- identity: === / !== and arrayFind compare components by REFERENCE ---
y = o;
assert( "=== same reference is true",       ( o === y ),                     true );
assert( "arrayFind finds same instance",    arrayFind( [ o ], o ),           1 );

// --- duplicate() is independent ---
d = duplicate( o );
d.pubData = "changed-copy";
assert( "duplicate is independent",  o.pubData,   "upd" );
assert( "duplicate keeps behaviour", d.greet(),   "hi" );

// --- inheritance-aware surface ---
c = new oop.FlyweightSurfaceChild();
assert( "isInstanceOf(child, parent)",  isInstanceOf( c, "FlyweightSurfaceProbe" ), true );
assert( "child inherits parent method", c.greet(),      "hi" );
assert( "child own method",             c.childOnly(),  "child" );

// --- getMetadata carries a name ---
assert( "getMetadata(component).name present", structKeyExists( getMetadata( o ), "name" ), true );

suiteEnd();
</cfscript>
