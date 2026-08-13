<cfscript>
// getComponentMetaData() on a dotted PATH is memoized per request (the instance
// form has always been cached on the class blueprint; the path form used to redo
// the whole derivation — resolve template, execute the pseudo-constructor,
// resolve inheritance, walk the chain again — on EVERY call, roughly D²
// pseudo-constructor executions for a chain of depth D. ColdBox/WireBox's
// Mapping.cfc calls this form with its own metadata cache disabled.)
//
// These assertions pin the OBSERVABLE contract the memo must not change:
// repeated calls are identical, they agree with getMetadata(instance), and a
// caller that MUTATES the struct it is handed cannot corrupt what the next
// caller sees (ColdBox's Util.getInheritedMetaData edits the struct in place).
suiteBegin("getComponentMetaData path-form metadata cache");

function fnNames( required array functions ) {
	local.out = [];
	for ( local.f in arguments.functions ) { arrayAppend( local.out, local.f.name ); }
	arraySort( local.out, "text" );
	return arrayToList( local.out );
}

first = getComponentMetaData( "oop.GcmCacheL1" );

// --- shape: own functions only, recursive `extends` ---
assert( "L1 name", first.name, "oop.GcmCacheL1" );
assert( "L1 type", first.type, "component" );
assert( "L1 own functions", fnNames( first.functions ), "init,l1One,l1Two" );
assert( "L2 name via extends", first.extends.name, "oop.GcmCacheL2" );
assert( "L2 own functions", fnNames( first.extends.functions ), "l2One,l2Two" );
assert( "L3 name via extends.extends", first.extends.extends.name, "oop.GcmCacheL3" );
assert( "L3 own functions", fnNames( first.extends.extends.functions ), "l3One,l3Two" );
assertFalse( "L3 is the top of the chain", structKeyExists( first.extends.extends, "extends" ) );

// --- repeated calls are identical ---
second = getComponentMetaData( "oop.GcmCacheL1" );
assert( "2nd call same name", second.name, first.name );
assert( "2nd call same functions", fnNames( second.functions ), fnNames( first.functions ) );
assert( "2nd call same parent name", second.extends.name, first.extends.name );
assert( "2nd call same parent functions",
	fnNames( second.extends.functions ), fnNames( first.extends.functions ) );
assert( "2nd call same grandparent functions",
	fnNames( second.extends.extends.functions ), fnNames( first.extends.extends.functions ) );

// --- each call hands back an INDEPENDENT struct (callers mutate it) ---
second.injectedByCaller = "mutated";
second.functions = [];
structDelete( second, "extends" );
third = getComponentMetaData( "oop.GcmCacheL1" );
assertFalse( "caller mutation does not leak into the next call",
	structKeyExists( third, "injectedByCaller" ) );
assert( "functions survive a caller emptying its own copy",
	fnNames( third.functions ), "init,l1One,l1Two" );
assertTrue( "extends survives a caller deleting it from its own copy",
	structKeyExists( third, "extends" ) );
assert( "extends still resolves the chain", third.extends.extends.name, "oop.GcmCacheL3" );

// mutating a NESTED level must not leak either
third.extends.name = "clobbered";
fourth = getComponentMetaData( "oop.GcmCacheL1" );
assert( "nested mutation does not leak", fourth.extends.name, "oop.GcmCacheL2" );

// --- the instance form (blueprint-cached) still agrees with the path form ---
inst = createObject( "component", "oop.GcmCacheL1" );
instMd = getMetadata( inst );
assert( "getMetadata(instance) name", instMd.name, first.name );
assert( "getMetadata(instance) own functions",
	fnNames( instMd.functions ), fnNames( first.functions ) );
assert( "getMetadata(instance) parent name", instMd.extends.name, first.extends.name );
assert( "getMetadata(instance) grandparent name",
	instMd.extends.extends.name, first.extends.extends.name );

// --- a different path must not be answered from another path's entry ---
l2 = getComponentMetaData( "oop.GcmCacheL2" );
assert( "sibling path resolves its own component", l2.name, "oop.GcmCacheL2" );
assert( "sibling path own functions", fnNames( l2.functions ), "l2One,l2Two" );
assert( "sibling path parent", l2.extends.name, "oop.GcmCacheL3" );

// --- a differently-CASED path keeps its own casing in `name` ---
// Resolution is case-insensitive, but the returned `name`/`fullname` echo the path
// AS WRITTEN, so the memo is keyed case-sensitively. A CI key would hand the
// second call the FIRST caller's casing.
upper = getComponentMetaData( "OOP.GCMCACHEL1" );
assert( "upper-cased path keeps its own casing", upper.name, "OOP.GCMCACHEL1" );
assert( "upper-cased path resolves the same component",
	fnNames( upper.functions ), "init,l1One,l1Two" );
assert( "upper-cased path resolves the same chain", upper.extends.extends.name, "oop.GcmCacheL3" );
mixedAgain = getComponentMetaData( "oop.GcmCacheL1" );
assert( "original casing is not overwritten by the upper-cased call",
	mixedAgain.name, "oop.GcmCacheL1" );

// --- a name shadowed by a LOCAL variable is never served from the memo ---
// (resolve_component_template checks locals before anything else.)
function shadowedLookup() {
	var oop = "not a component";
	// still a dotted path, and `oop.GcmCacheL1` is not a local key
	return getComponentMetaData( "oop.GcmCacheL1" ).name;
}
assert( "dotted lookup inside a function", shadowedLookup(), "oop.GcmCacheL1" );

suiteEnd();
</cfscript>
