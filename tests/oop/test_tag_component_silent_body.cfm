<cfscript>
suiteBegin("tag component bodies: output=false text is a no-op, output=true still emits");

// The tag preprocessor turns template text between <cffunction> tags into output
// statements in the pseudo-constructor. Under output="false" they emit nothing, and
// RustCFML now drops them at compile time so the class can be built from its
// prototype. Behaviour must be unchanged.
savecontent variable="silentOut" {
	a = new oop.tagbody.SilentTag( "first" );
	b = new oop.tagbody.SilentTag( "second" );
}
assert( "output=false tag component emits nothing when constructed", trim( silentOut ), "" );
assert( "first instance keeps its own data", a.getLabel(), "first" );
assert( "second instance keeps its own data", b.getLabel(), "second" );
a.addItem( 1 ).addItem( 2 );
assert( "mutating one instance's array", a.itemCount(), 2 );
assert( "does not touch the other instance's array", b.itemCount(), 0 );

fns = {};
for ( f in getMetaData( a ).functions ) { fns[ f.name ] = f; }
assert( "function hints survive on the instance metadata", fns.getLabel.hint, "returns the label" );
assert( "custom function annotations survive (WireBox reads provider/inject this way)", fns.itemCount.myannotation, "hello" );
assert( "component hint survives", getMetaData( a ).hint, "a tag component that must stay silent" );

savecontent variable="loudOut" {
	l1 = new oop.tagbody.LoudTag();
	l2 = new oop.tagbody.LoudTag();
}
assert( "output=true tag component emits its text on every construction", loudOut, "[LOUD][LOUD]" );
assert( "and its methods still work", l2.ping(), "pong" );

suiteEnd();
</cfscript>
