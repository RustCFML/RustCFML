<cfscript>
suiteBegin( "Injected method dispatch: scope binding survives the owned-frame fast path (v0.663.0)" );
// An instance-method dispatch hands a PLAIN class method its frame ready-made.
// A method injected from another CFC (TestBox custom matchers, Wheels
// controller mixins) must keep its Lucee semantics: a plain UDF binds to the
// component it is INVOKED on, a closure keeps what it captured. Expectations
// verified on Lucee 7.1.
p = new oop.injfr.Provider();
t = new oop.injfr.Target();
t.inject( "injected", p.getMatcher() );
t.inject( "clo", p.getClosure() );
assert( "plain own method", t.own( 1 ), "own:1:from-target" );
assert( "injected UDF binds to the target it is invoked on", t.injected( "a" ), "from-target:a:1" );
assert( "injected UDF via this.x() inside a method", t.callInjectedThis( "b" ), "from-target:b:2" );
assert( "injected UDF via bare call inside a method", t.callInjectedBare( "c" ), "from-target:c:3" );
assert( "the target's private state was the one mutated", t.getCalls(), 3 );
assert( "injected closure keeps its captured local", listFirst( t.clo( "d" ), ":" ), "from-provider" );
assert( "injected closure via this.x() keeps its captured local", listFirst( t.callClosureThis( "e" ), ":" ), "from-provider" );
assert( "injected closure via bare call keeps its captured local", listFirst( t.callClosureBare( "f" ), ":" ), "from-provider" );
// NOTE (pre-existing, both engines probed 2026-09-11): `variables.x` INSIDE an
// injected closure is the DEFINING component's scope on Lucee ("from-provider")
// and the invoking target's here ("from-target"). Not asserted; tracked in
// docs/known-issues.md §95.
assert( "own after injected", t.own( 2 ), "own:2:from-target" );
suiteEnd();
</cfscript>
