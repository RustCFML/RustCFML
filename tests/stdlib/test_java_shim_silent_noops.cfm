<cfscript>
suiteBegin("Java shims: silently-dropped operations");

// Every case below used to return null and DO NOTHING — a mutation that
// reported success while the target was untouched. They came from the java-shim
// dispatch convention where "handler returned Ok(Null)" meant "method not
// recognised", so any unimplemented method silently became a no-op.
//
// Expectations verified against Lucee 7.0.4 (a real JVM) except where noted.

// ---- StringBuilder mutators (buffer was left silently intact) -------------
function sb(s) { return createObject("java", "java.lang.StringBuilder").init(s); }

b = sb("world");        b.insert(0, "hello ");
assert("StringBuilder.insert mutates the buffer", b.toString(), "hello world");

b = sb("hello world");  b.delete(5, 11);
assert("StringBuilder.delete removes the range", b.toString(), "hello");

b = sb("abc");          b.deleteCharAt(1);
assert("StringBuilder.deleteCharAt removes one char", b.toString(), "ac");

b = sb("abcdef");       b.setLength(3);
assert("StringBuilder.setLength truncates", b.toString(), "abc");

b = sb("hello world");  b.replace(0, 5, "howdy");
assert("StringBuilder.replace swaps the range", b.toString(), "howdy world");

b = sb("abc");          b.reverse();
assert("StringBuilder.reverse reverses in place", b.toString(), "cba");

b = sb("b");            b.insert(0, "a").append("c");
assert("StringBuilder mutators are chainable and return this", b.toString(), "abc");

// Java clamps `end` to the buffer length rather than throwing.
b = sb("abc");          b.delete(1, 99);
assert("StringBuilder.delete clamps end to length", b.toString(), "a");

// A builder passed INTO a function must be mutated for the caller too — the
// shim is a reference type. (This is what MockBox's stub generation relies on.)
function mutate(x) { x.insert(0, "P-"); }
b = sb("body"); mutate(b);
assert("StringBuilder mutation is visible to the caller", b.toString(), "P-body");

caught = "";
try { sb("ab").insert(9, "x"); } catch (any e) { caught = e.type; }
assert("StringBuilder.insert past the end throws",
	caught, "java.lang.StringIndexOutOfBoundsException");

// ---- ConcurrentHashMap writes (dropped, old value retained) --------------
m = createObject("java", "java.util.concurrent.ConcurrentHashMap").init();
m.put("k", "old");

prev = m.replace("k", "new");
assert("CHM.replace returns the previous value", prev, "old");
assert("CHM.replace actually replaces", m.get("k"), "new");

r = m.replace("absent", "v");
assertTrue("CHM.replace on a missing key returns null", isNull(r));
assertTrue("CHM.replace on a missing key does not insert", !m.containsKey("absent"));

m.put("d", "x");
assert("CHM.remove returns the removed value", m.remove("d"), "x");
assertTrue("CHM.remove actually removes", !m.containsKey("d"));

// getOrDefault was never implemented: it returned null, so the caller's default
// was discarded and a miss was indistinguishable from a stored null.
assert("CHM.getOrDefault returns the default on a miss",
	m.getOrDefault("nope", "dflt"), "dflt");
assert("CHM.getOrDefault returns the value on a hit",
	m.getOrDefault("k", "dflt"), "new");

// A genuine null from a map getter must still be believed (this is what the
// deleted `map_getter_owns_null` allowlist used to guarantee by hand).
assertTrue("CHM.get on a missing key is null", isNull(m.get("no-such-key")));

// ---- Collections.sort (sorted numbers lexicographically) ------------------
nums = [10, 9, 2];
createObject("java", "java.util.Collections").sort(nums);
assert("Collections.sort orders numbers numerically, not as strings",
	arrayToList(nums), "2,9,10");

strs = ["pear", "apple", "fig"];
createObject("java", "java.util.Collections").sort(strs);
assert("Collections.sort still orders strings lexicographically",
	arrayToList(strs), "apple,fig,pear");

// ---- TimeZone offsets (were hardcoded 0 for every zone) ------------------
tzc = createObject("java", "java.util.TimeZone");
assert("TimeZone.getRawOffset is the real standard offset",
	tzc.getTimeZone("America/New_York").getRawOffset(), -18000000);
assert("TimeZone.getRawOffset handles east-of-UTC zones",
	tzc.getTimeZone("Asia/Tokyo").getRawOffset(), 32400000);
assert("TimeZone.getRawOffset is 0 for UTC",
	tzc.getTimeZone("UTC").getRawOffset(), 0);
assertTrue("TimeZone.useDaylightTime is true for a DST zone",
	tzc.getTimeZone("America/New_York").useDaylightTime());
assertTrue("TimeZone.useDaylightTime is false for a non-DST zone",
	!tzc.getTimeZone("Asia/Tokyo").useDaylightTime());
// DST actually shifts the offset: Jan vs Jul 2026 in New York.
assert("TimeZone.getOffset reflects standard time in winter",
	tzc.getTimeZone("America/New_York").getOffset(1768478400000), -18000000);
assert("TimeZone.getOffset reflects daylight time in summer",
	tzc.getTimeZone("America/New_York").getOffset(1784116800000), -14400000);

// ---- java.util.Date comparisons (returned null, i.e. always falsy) -------
d1 = createObject("java", "java.util.Date").init(1000);
d2 = createObject("java", "java.util.Date").init(2000);
assertTrue("Date.before compares", d1.before(d2));
assertTrue("Date.after compares", !d1.after(d2));
assertTrue("Date.equals compares", d1.equals(d1));
assert("Date.compareTo orders", d1.compareTo(d2), -1);

// ---- File.renameTo (returned null, file never moved) ---------------------
tmp = getTempDirectory();
srcPath = tmp & "rustcfml_renameto_src.txt";
dstPath = tmp & "rustcfml_renameto_dst.txt";
fileWrite(srcPath, "payload");
if (fileExists(dstPath)) { fileDelete(dstPath); }
srcFile = createObject("java", "java.io.File").init(srcPath);
dstFile = createObject("java", "java.io.File").init(dstPath);
assertTrue("File.renameTo reports success", srcFile.renameTo(dstFile));
assertTrue("File.renameTo actually moves the file",
	fileExists(dstPath) AND NOT fileExists(srcPath));
assert("File.renameTo preserves content", fileRead(dstPath), "payload");
if (fileExists(dstPath)) { fileDelete(dstPath); }
if (fileExists(srcPath)) { fileDelete(srcPath); }

suiteEnd();
</cfscript>
