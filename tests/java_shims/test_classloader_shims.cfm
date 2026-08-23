<cfscript>
suiteBegin("Java classloader shims (cbjavaloader boot path)");

// --- Unsupported java class now THROWS loudly (was a silent NULL no-op,
//     which surfaced downstream as a confusing "Variable X is undefined").
assertThrows("createObject java unknown class throws", function() {
	return createObject("java", "com.totally.Made.Up.Class");
});

// --- java.lang.Class + Class.forName ---
clazz = createObject("java", "java.lang.Class");
assertTrue("java.lang.Class shim is object", isObject(clazz));
urlClass = clazz.forName("java.net.URL");
assert("Class.forName().getName()", urlClass.getName(), "java.net.URL");
assert("Class.forName().getSimpleName()", urlClass.getSimpleName(), "URL");

// --- java.lang.reflect.Array (newInstance / set / get / getLength) ---
// On a real JVM newInstance(URL.class, 3) makes a TYPED URL[3] and storing a
// String into it throws "array element type mismatch". This engine's array is
// untyped, so the whole block is a RustCFML superset.
if ( isRustCFML() ) {
arrUtil = createObject("java", "java.lang.reflect.Array");
holder = arrUtil.newInstance(urlClass, 3);
assert("reflect.Array.newInstance length", arrayLen(holder), 3);
assertTrue("reflect.Array.newInstance is nulls", isNull(holder[1]));
arrUtil.set(holder, 0, "file:/a.jar");
arrUtil.set(holder, 2, "file:/c.jar");
assert("reflect.Array.set index 0 -> [1]", holder[1], "file:/a.jar");
assert("reflect.Array.set index 2 -> [3]", holder[3], "file:/c.jar");
assert("reflect.Array.get", arrUtil.get(holder, 0), "file:/a.jar");
assert("reflect.Array.getLength", arrUtil.getLength(holder), 3);

} // end isRustCFML() — untyped reflect.Array

// --- array.iterator() / hasNext() / next() (java.util.List passthrough) ---
items = ["one", "two", "three"];
it = items.iterator();
collected = "";
while (it.hasNext()) {
	collected = listAppend(collected, it.next());
}
assert("array.iterator() walks all elements", collected, "one,two,three");
assertFalse("iterator exhausted hasNext()=false", it.hasNext());

emptyIt = [].iterator();
assertFalse("empty array iterator hasNext()=false", emptyIt.hasNext());

// --- java.io.File.toURL() ---
assert("File.toURL()", createObject("java", "java.io.File").init("/tmp/x.jar").toURL(), "file:/tmp/x.jar");

// --- java.net.URLClassLoader deferred object: classloader plumbing succeeds,
//     but invoking a class it "loads" throws (no JVM). ---
// RustCFML-only by construction: it takes the untyped array built above, and
// "loads a class that then throws" is precisely the no-JVM behaviour.
if ( isRustCFML() ) {
ucl = createObject("java", "java.net.URLClassLoader").init(holder);
assertTrue("URLClassLoader.init() returns object", isObject(ucl));
loaded = ucl.loadClass("com.compoundtheory.classloader.NetworkClassLoader");
assert("URLClassLoader.loadClass().getName()", loaded.getName(), "com.compoundtheory.classloader.NetworkClassLoader");
assertTrue("URLClassLoader.addURL() is a no-op (non-null)", ucl.addURL("file:/d.jar"));
assertThrows("deferred java object throws on real method use", function() {
	return ucl.invokeSomethingThatNeedsAJvm();
});

} // end isRustCFML() — URLClassLoader over the untyped array

// --- coldfusion.runtime.java.JavaProxy path (kept on the java path because the
//     shim does NOT throw, avoiding cbjavaloader's heavy CFC-reflection fallback). ---
jp = createObject("java", "coldfusion.runtime.java.JavaProxy");
assertTrue("JavaProxy shim is object", isObject(jp));
// Proxies the class the deferred URLClassLoader above "loaded", so this is
// RustCFML-only for the same reason.
if ( isRustCFML() ) {
    proxy = jp.init(loaded);
    instance = proxy.init();
    assertTrue("NetworkClassLoader proxy addUrl() no-op", instance.addUrl("file:/e.jar"));
}

suiteEnd();
</cfscript>
