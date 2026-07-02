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

// network I/O explicitly throws (no JVM)
assertThrows("openStream() throws (no JVM)", function(){
    createObject("java","java.net.URL").init("http://example.com").openStream();
});

suiteEnd();
</cfscript>
