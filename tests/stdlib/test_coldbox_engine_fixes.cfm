<cfscript>
suiteBegin("Engine fixes surfaced by booting ColdBox (Lucee-parity)");

// ---- Case-insensitive local variables ----
// CFML identifiers are case-insensitive: a `var flashPath` then a later
// `flashpath = …` must update the SAME local, not fork a second variable or
// leak to the variables scope. (ColdBox's RequestService relies on exactly this
// in buildFlashScope.)
ciLocal = function(){
	var flashPath = "";
	flashpath     = "session";
	return flashPath;
};
assert( "var X then x= updates the same local (case-insensitive)", ciLocal(), "session" );

foo = "";
FOO = "bar";
assert( "page var foo/FOO are the same variable", foo, "bar" );

// ---- structAppend from a component must not turn the dest into an object ----
// structAppend( struct, cfc, true ) folds the component's public members into
// the struct (ColdBox's Controller.loadColdBoxSettings does this). The result
// must remain a plain struct — copying the engine-internal component markers
// made it report isObject()=true, so a later struct member call (keyExists)
// dispatched as a component method and threw.
probe    = new concurrenttest.SampleCallable();
settings = { x = 1 };
structAppend( settings, probe, true );
assertFalse( "structAppend(struct, component) leaves dest NOT an object", isObject( settings ) );
assertTrue( "dest is still a struct", isStruct( settings ) );
assertTrue( "dest.keyExists() works after append-from-component", settings.keyExists( "x" ) );

// ---- Components inherit Object.hashCode()/equals() ----
// Every CFC inherits java.lang.Object, so these resolve even when undeclared.
// (ColdBox's async BaseProxy calls hashCode() on proxied components.)
o1 = new concurrenttest.SampleCallable();
o2 = new concurrenttest.SampleCallable();
// RustCFML gives every CFC the java.lang.Object methods; Lucee does not, so
// these two are supersets. The rest of the suite is cross-engine.
if ( isRustCFML() ) {
    assertTrue( "component hashCode() is numeric", isNumeric( o1.hashCode() ) );
    assertTrue( "component equals() itself", o1.equals( o1 ) );
    assertFalse( "component not equals a different instance", o1.equals( o2 ) );
}

// ---- java.time shim basics ----
dur = createObject( "java", "java.time.Duration" ).ofSeconds( 5 );
assert( "Duration.ofSeconds(5).toMillis()", dur.toMillis(), 5000 );
assert( "Duration.ofMinutes(2).getSeconds()", createObject( "java", "java.time.Duration" ).ofMinutes( 2 ).getSeconds(), 120 );
zone = createObject( "java", "java.time.ZoneId" ).of( "UTC" );
assert( "ZoneId.of('UTC').toString()", zone.toString(), "UTC" );

suiteEnd();
</cfscript>
