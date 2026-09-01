<cfscript>
suiteBegin("An error in a pseudo-constructor is reported, not swallowed");

// The component body's error used to be DISCARDED (`let _ = execute(...)`), and
// the half-built component was then assembled from a corrupted stack: a nested
// `new` that threw inside a struct literal left ITS `__extends` behind, and the
// enclosing component silently acquired the inner one's parent. A missing Java
// algorithm inside a Preside module's init() surfaced three steps later as
// "invalid component definition, can't find component [Rsa]" — naming a
// component nobody had asked for.

function errOf( required string cfc, required string how ) {
	try {
		if ( arguments.how == "new" ) { createObject( "component", arguments.cfc ); }
		else { getComponentMetaData( arguments.cfc ); }
		return "no error";
	} catch( any e ) {
		return e.type & "|" & e.message;
	}
}

expected = "my.ctor.failure|pseudo-constructor dependency failed";

// Both the struct-literal and the plain-assignment shapes, through both the
// construction and the metadata paths.
assert( "constructing reports the pseudo-constructor's own error (struct literal)"
      , errOf( "oop.ctorfail.HolderStruct", "new" ), expected );
assert( "...and the metadata path reports it too"
      , errOf( "oop.ctorfail.HolderStruct", "meta" ), expected );
assert( "constructing reports it for a plain assignment as well"
      , errOf( "oop.ctorfail.HolderPlain", "new" ), expected );
assert( "...and its metadata path too"
      , errOf( "oop.ctorfail.HolderPlain", "meta" ), expected );

// The component that threw is itself reported faithfully. (`createObject`
// alone does NOT run init(), so reach it through `new`, as the holders do.)
function newThrower() {
	try { return new oop.ctorfail.sub.Thrower(); }
	catch( any e ) { return e.type & "|" & e.message; }
}
assert( "the failing component reports its own error directly", newThrower(), expected );
assertFalse( "createObject alone does not run init(), so it does not throw"
           , errOf( "oop.ctorfail.sub.Thrower", "new" ) != "no error" );

// A healthy sibling in the same package is unaffected.
base = createObject( "component", "oop.ctorfail.sub.Base" );
assert( "a component whose body does not throw still builds", base.baseFn(), "base" );

suiteEnd();
</cfscript>
