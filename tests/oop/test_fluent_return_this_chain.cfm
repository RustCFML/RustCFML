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

suiteEnd();
</cfscript>
