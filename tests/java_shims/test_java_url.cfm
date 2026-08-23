<cfscript>
suiteBegin("Java shim: java.net.URL (GH ##231)");

// createObject("java","java.net.URL") is a no-JVM structural parse of a URL —
// no network I/O, just the accessor surface CFML code uses to pull apart URLs.

u = createObject("java","java.net.URL").init("https://user@example.com:8443/a/b?x=1&y=2##frag");
assert("getProtocol", u.getProtocol(), "https");
assert("getHost", u.getHost(), "example.com");
assert("getPort (explicit)", u.getPort(), 8443);
assert("getDefaultPort", u.getDefaultPort(), 443);
assert("getPath", u.getPath(), "/a/b");
assert("getQuery", u.getQuery(), "x=1&y=2");
assert("getRef", u.getRef(), "frag");
assert("getFile (path+query)", u.getFile(), "/a/b?x=1&y=2");
assert("getAuthority", u.getAuthority(), "user@example.com:8443");
assert("getUserInfo", u.getUserInfo(), "user");
assert("toString round-trips", u.toString(), "https://user@example.com:8443/a/b?x=1&y=2##frag");
assert("toExternalForm", u.toExternalForm(), "https://user@example.com:8443/a/b?x=1&y=2##frag");

// port omitted -> getPort() is -1 (Java sentinel), default port still known
u2 = createObject("java","java.net.URL").init("http://example.com/plain");
assert("getPort (omitted) is -1", u2.getPort(), -1);
assert("getDefaultPort http", u2.getDefaultPort(), 80);
assertTrue("getQuery is null when absent", isNull(u2.getQuery()));
assertTrue("getRef is null when absent", isNull(u2.getRef()));

// multi-arg constructor: URL(protocol, host, port, file)
u3 = createObject("java","java.net.URL").init("http", "localhost", 9000, "/api/v1");
assert("4-arg host", u3.getHost(), "localhost");
assert("4-arg port", u3.getPort(), 9000);
assert("4-arg path", u3.getPath(), "/api/v1");

// NB: network I/O (openStream/openConnection) throws on RustCFML because there
// is no JVM — not asserted here since real Lucee/ACF would attempt live I/O, so
// the behaviour is intentionally engine-specific and outside the parity bar.

// GH #238: URL equality. Two URLs built from the same spec are equal; different
// specs are not. Previously `eq` compared struct identity (always false) and
// TestBox's equalize() saw all URL shims as interchangeable empty structs
// (isStruct was true but the value lived in hidden __keys).
urlA = createObject("java","java.net.URL").init("http://www.luismajano.com");
urlB = createObject("java","java.net.URL").init("http://www.luismajano.com");
urlC = createObject("java","java.net.URL").init("http://www.ortussolutions.com");
// `eq` between two Java objects works here; Lucee refuses to compare complex
// types as simple values. The .equals() legs below are cross-engine.
if ( isRustCFML() ) {
    assertTrue("same-spec URLs are eq", urlA eq urlB);
    assertFalse("different-spec URLs are not eq", urlA eq urlC);
}
assertTrue("same-spec URLs .equals()", urlA.equals(urlB));
assertFalse("different-spec URLs .equals()", urlA.equals(urlC));
// A URL is an object, not a struct (Lucee parity) — this is what lets TestBox's
// equalize() reach .equals() instead of an empty-struct key walk.
assertFalse("isStruct(URL) is false", isStruct(urlA));
// default-port equivalence: explicit :80 equals the implied http default
uP1 = createObject("java","java.net.URL").init("http://h.com/a");
uP2 = createObject("java","java.net.URL").init("http://h.com:80/a");
// `eq` again — cross-engine via .equals(), which agrees on both.
assertTrue("explicit default port equals implied", uP1.equals( uP2 ));

suiteEnd();
</cfscript>
