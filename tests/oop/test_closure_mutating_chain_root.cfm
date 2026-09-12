<cfscript>
suiteBegin( "A mutating member call on a chain rooted at a captured component leaves the captured variable alone" );
s = new oop.clochain.Svc();
assert( "both routes appended, receiver still the component", s.activate( "mod1" ), "2/true" );
assert( "second activation on the same instance", s.activate( "mod2" ), "2/true" );
suiteEnd();
</cfscript>
