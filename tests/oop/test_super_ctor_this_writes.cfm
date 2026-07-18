<cfscript>
suiteBegin("super.method() this-writes in pseudo-constructor (Preside Application.cfc pattern)");

c = new SuperCtorChild();

// this.* written by super.setupIt() during construction persist on the instance
assert("super-set this member (direct read)", c.fromSuper, "SUPER_C");
// child's own this.num = 99 wins over the parent's this.num = 7 (parent runs first)
assert("child override wins over parent this member", c.num, 99);

// ...and are visible to a method called later on the same instance
r = c.readAll();
assert("super-set this member visible in method", r.fromSuper, "SUPER_C");
assert("overridden this member visible in method", r.num, 99);

suiteEnd();
</cfscript>
