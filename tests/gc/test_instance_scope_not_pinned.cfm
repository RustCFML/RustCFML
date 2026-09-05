<cfscript>
suiteBegin( "Cycle GC: a component's live variables scope is not a pinned root" );

// A component that defines a closure keeps its LIVE `variables` scope (the
// closure captured it) rather than the partitioned copy, so that scope is a
// TRACKED cycle-GC node. The Instance's reference to it must be counted as an
// internal edge; when it was not, the scope's external count read 1, every such
// instance became a pinned root, and its whole object graph was marked live —
// a complete generation stranded per framework reload.
//
// Behavioural proxy for the collector's arithmetic: build a generation, drop it,
// and require the instances to be genuinely unreachable afterwards. The memory
// assertion itself lives in the Rust suite; this pins the SEMANTICS the fix
// must not break — the scope must stay alive and correct while it IS reachable.

obj = new gc.ClosureHolder( "a" );
assert( "closure sees its captured scope", obj.readViaClosure(), "a" );

obj.setPeer( new gc.ClosureHolder( "b" ) );
assert( "peer reachable through the live scope", obj.readPeer(), "b" );

// Mutating through the closure must reach the same live scope, not a copy.
obj.writeViaClosure( "c" );
assert( "closure writes through to the live scope", obj.readId(), "c" );
assert( "and the closure re-reads it", obj.readViaClosure(), "c" );

suiteEnd();
</cfscript>
