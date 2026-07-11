<cfscript>
suiteBegin("Fluent return-this chain installs every step (GH ##260)");

// GH ##260: a fluent chain of `return this` calls dropped every step after the
// first. Root cause: components have value semantics, so `return this` yields a
// copy on a fresh Arc; the method-call write-back's chained-CFC identity guard
// used Arc `ptr_eq` to tell "the same instance returned via `this`" from "a
// getter that returned a *different* CFC", and the copy defeated it. A stable
// per-instance `__instance_id` now makes that distinction, so every step lands.

// (A) plain value members set through a fluent chain — every step must land.
a = new FluentReturnThis();
a.set( "x", 1 ).set( "y", 2 ).set( "z", 3 );
assert("chain set x", a.x, 1);
assert("chain set y", a.y, 2);   // 2nd+ steps were dropped before the fix
assert("chain set z", a.z, 3);

// (B) MockBox-shaped: a value stub chained into a further value stub. This is
//     the shape of TestBox MockBox's `obj.$( "m1", r1 ).$( "m2", r2 )` value
//     stubs (WireBox BuilderTest), which drove the report.
b = new FluentReturnThis();
b.stub( "m1", "STUB-M1" ).stub( "m2", "STUB-M2" );
assert("chained 1st value stub", b.m1, "STUB-M1");
assert("chained 2nd value stub", b.m2, "STUB-M2");   // the dropped one

// (C) longer chain — 4 steps, all must land.
c = new FluentReturnThis();
c.set( "p", "P" ).set( "q", "Q" ).set( "r", "R" ).set( "s", "S" );
assert("chain step 4 p", c.p, "P");
assert("chain step 4 q", c.q, "Q");
assert("chain step 4 r", c.r, "R");
assert("chain step 4 s", c.s, "S");

// (D) the OPPOSITE case must still hold: a chained call whose inner method
//     returns a DIFFERENT CFC must NOT clobber the base, and must not be
//     treated as the same instance by the identity guard.
d = new FluentReturnThis();
d.getDep().set( "mark", "X" );
assert("base keeps identity after chained foreign-CFC mutate", d.whoAmI(), "root");
assert("foreign-CFC chained mutation persists on the shared dep", d.getDep().mark, "X");

// (E) GH ##261: a FUNCTION/closure member injected onto `this` through the
//     2nd-or-later step of a fluent chain used to be silently dropped, so a
//     later call dispatched the ORIGINAL method. Value members already worked
//     (cases A–C); only closures chained after a prior step were lost. Root
//     cause: since #260 `return this` copies onto a fresh Arc, but the closure
//     captured the receiver BEFORE injection, so the shared closure env held a
//     stale snapshot; the post-call env-reconcile then pulled that snapshot
//     back over the just-written receiver. Validated as a real divergence vs
//     Lucee 7 (which installs the closure).
e = new FluentReturnThis();
e.stub( "v", "V" ).stub( "m2", ()=>"STUB-M2" );
assert("chained value member still lands", e.v, "V");
assert("chained closure overrides the real method", e.m2("k"), "STUB-M2");

// (F) noop() first, then inject a BRAND-NEW closure member (not overriding an
//     existing method). The new member must exist and dispatch to the closure.
f = new FluentReturnThis();
f.noop().stub( "brandNew", ()=>"HELLO" );
assertTrue("brand-new chained closure member exists", structKeyExists( f, "brandNew" ));
assert("brand-new chained closure dispatches", f.brandNew(), "HELLO");

// (G) control: a STRUCT value injected the same way must still land (never
//     regressed, but locks the value-member path alongside the closure fix).
g = new FluentReturnThis();
g.noop().stub( "cfg", { a: 1 } );
assert("chained struct member lands", g.cfg.a, 1);

// (H) the injected closure must survive a subsequent sibling call that takes
//     the receiver as a plain argument (`structKeyExists(e, …)` is a bare Call
//     whose own env-reconcile must not revert the receiver).
assertTrue("closure survives an intervening bare Call on the receiver",
	structKeyExists( e, "m2" ) && e.m2("k") == "STUB-M2");

// (I) GH ##263: a reference captured BEFORE a chained stub install must see
//     EVERY chained step — not just the first. `return this` copies onto a
//     fresh Arc since #260, and the write-back used to re-point the direct
//     variable at that copy, orphaning any other reference that still held the
//     original shared Arc (a Holder that captured the mock before stubbing).
//     The 1st chained stub (mutated in place on the shared Arc) stayed visible
//     to the alias, but every 2nd-and-later stub landed only on the copy and
//     fell through to the real method through the alias. This is exactly how
//     ColdBox BuilderTest's `mockInjector.$("getInstance",…).$("containsInstance")`
//     lost `containsInstance` → real `Injector.containsInstance` → undefined
//     `binder`. The write-back now merges same-instance return-this copies IN
//     PLACE into the shared Arc, so the held alias sees all steps.
//     The stubs must be FUNCTIONS (as MockBox's generator injects), so a call
//     through the alias dispatches to the stub rather than the real method.
i = new FluentReturnThis();
hi = new FluentAliasHolder().init( i );        // alias captured BEFORE the chain
i.stub( "m1", ()=>"STUB-M1" ).stub( "m2", ( key )=>"STUB-M2" );
assert("alias sees 1st chained stub",  hi.callM1(), "STUB-M1");
assert("alias sees 2nd chained stub",  hi.callM2("k"), "STUB-M2");   // was REAL-M2

// (J) control: two stubs as SEPARATE statements were already visible to a
//     pre-captured alias — lock that it stays so.
j = new FluentReturnThis();
hj = new FluentAliasHolder().init( j );
j.stub( "m1", ()=>"STUB-M1" );
j.stub( "m2", ( key )=>"STUB-M2" );
assert("alias sees 2nd separate-statement stub", hj.callM2("k"), "STUB-M2");

// (K) control: alias captured AFTER the chain must also see both.
k = new FluentReturnThis();
k.stub( "m1", ()=>"STUB-M1" ).stub( "m2", ( key )=>"STUB-M2" );
hk = new FluentAliasHolder().init( k );
assert("alias-after-chain sees 2nd stub", hk.callM2("k"), "STUB-M2");

suiteEnd();
</cfscript>
