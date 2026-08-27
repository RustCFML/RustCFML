<cfscript>
suiteBegin("Metadata of a relatively-instantiated component");

// `new algorithms.Rsa()` inside another CFC records the name AS WRITTEN, and
// re-resolving that relative name against the component's OWN directory looks
// for algorithms/algorithms/Rsa.cfc and fails. The metadata then collapsed to
// bare { name } — no path, no type, no functions.
//
// That is not cosmetic: WireBox reads `md.path` as an object's identity
// (Injector.autowire → registerNewInstance( targetID, md.path )), so every
// relatively instantiated model got a mapping whose path was a bare name, and
// resolving THAT later threw "can't find component [Rsa]" during boot.

wrapper = createObject( "component", "oop.relmeta.Wrapper" );

for ( howMade in [ "viaNew", "viaCreateObject" ] ) {
	obj = Invoke( wrapper, howMade );
	md  = getMetaData( obj );

	assertTrue( "#howMade#: metadata carries a path", Len( md.path ?: "" ) > 0 );
	assertTrue( "#howMade#: the path is the component's own file"
	          , ( md.path ?: "" ) contains "algorithms" & ( server.separator.file ?: "/" ) & "Rsa.cfc"
	         || ( md.path ?: "" ) contains "algorithms/Rsa.cfc" );
	assert( "#howMade#: type is component", md.type ?: "", "component" );
	assertTrue( "#howMade#: the declared functions are visible", ArrayLen( md.functions ?: [] ) >= 2 );
	assertTrue( "#howMade#: name is the full dotted path, not the relative one"
	          , ( md.name ?: "" ) contains "oop.relmeta.algorithms.Rsa" );

	// And the object itself still works.
	assert( "#howMade#: the instance is usable", obj.sign( "x" ), "signed:x" );
}

suiteEnd();
</cfscript>
