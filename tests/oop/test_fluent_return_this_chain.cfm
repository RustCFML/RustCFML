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

suiteEnd();
</cfscript>
