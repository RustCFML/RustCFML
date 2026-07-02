<cfscript>
suiteBegin("Core: session scope holds a live component (GH ##236)");

// GH #236: the default in-memory session store keeps live object references, so
// storing a CFC (or closure) in `session` is allowed and reads back as a live
// object — matching Lucee/ACF. The data-only rule (issue #88) still applies to
// SERIALIZING stores (memcached/datasource/KV/cluster), which genuinely cannot
// round-trip a live object; that is covered by the Rust unit tests. Green on
// both engines with an in-memory session.

session.bean236 = new oop.SessBean236();
assertTrue("component stored in session is a live object", isObject( session.bean236 ));
assert("its methods still work after storing in session", session.bean236.hello(), "hi");

// closures are fine in an in-memory session too
session.cb236 = function(){ return 42; };
assert("closure stored in session is callable", session.cb236(), 42);

// plain data of course still round-trips
session.data236 = { a = 1, b = [ 2, 3 ] };
assert("plain data in session", session.data236.b[2], 3);

structDelete( session, "bean236" );
structDelete( session, "cb236" );
structDelete( session, "data236" );

suiteEnd();
</cfscript>
