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

// ---- Optional.orElse family (were missing, so they returned null) --------
// NOTE: only `orElse` is comparable on Lucee — a CFML closure is not a
// java.util.function.Supplier there, so Lucee throws NoSuchMethodException for
// orElseGet/orElseThrow(supplier). Our shim accepts them, which is additive.
Opt = createObject("java", "java.util.Optional");
some = Opt.of("v");
none = Opt.empty();
assert("Optional.orElse returns the value when present", some.orElse("fb"), "v");
assert("Optional.orElse returns the fallback when empty", none.orElse("fb"), "fb");

// ---- java.nio.file.Files (returned null and performed no I/O) ------------
Files = createObject("java", "java.nio.file.Files");
tdir = getTempDirectory();
fw = tdir & "rustcfml_nio_write.txt";
fc = tdir & "rustcfml_nio_copy.txt";
fdir = tdir & "rustcfml_nio_dir/sub/deep";
if (fileExists(fw)) { fileDelete(fw); }
if (fileExists(fc)) { fileDelete(fc); }

// Capability guard: a real JVM's Files.write takes (Path, byte[]) and rejects
// (string, string) with NoSuchMethodException — Lucee cannot express this call
// with CFML types at all. Our shim accepts plain paths and strings, which is
// additive, so probe first and skip rather than fail on the reference engine.
nioTakesStrings = true;
try { Files.write(fw, "nio-payload"); }
catch (any e) { nioTakesStrings = false; }

if (nioTakesStrings) {
	assert("Files.write actually writes the file", fileRead(fw), "nio-payload");

	Files.copy(fw, fc);
	assert("Files.copy actually copies", fileRead(fc), "nio-payload");

	Files.createDirectories(fdir);
	assertTrue("Files.createDirectories creates the whole chain", directoryExists(fdir));

	// readAllBytes returns the same "native byte[]" shape as String.getBytes():
	// a CFML array of signed ints, so arrayLen() works on it (GH #271).
	assert("Files.readAllBytes returns an indexable byte array",
		arrayLen(Files.readAllBytes(fw)), 11);

	// A failed write must throw, not silently do nothing.
	caught = "";
	try { Files.write(tdir & "rustcfml_no_such_dir_xyz/f.txt", "x"); }
	catch (any e) { caught = e.type; }
	assert("Files.write on an unwritable path throws IOException",
		caught, "java.io.IOException");
} else {
	assertTrue("java.nio.file.Files skipped: engine requires real Path/byte[] args",
		true);
}

if (fileExists(fw)) { fileDelete(fw); }
if (fileExists(fc)) { fileDelete(fc); }
if (directoryExists(tdir & "rustcfml_nio_dir")) {
	directoryDelete(tdir & "rustcfml_nio_dir", true);
}

// ---- fileExists() must not go stale after a shim mutation ---------------
// The engine keeps a request-scoped POSITIVE existence memo. The BIF write path
// clears it (native fileDelete/fileMove were always correct), but java-shim
// mutations bypassed that dispatch entirely — so after File.delete() or
// Files.move(), fileExists() kept answering "true" for the rest of the request.
// A silently wrong answer, not a crash, which is why it went unnoticed.
// directoryList() is the ground truth here: it hits the filesystem directly.
staleDir = getTempDirectory();
staleFile = staleDir & "rustcfml_stale_probe.txt";
fileWrite(staleFile, "x");

// Prime the memo — this is what made the answer stick.
assertTrue("the probe file exists before deletion", fileExists(staleFile));

createObject("java", "java.io.File").init(staleFile).delete();

assertTrue("directoryList confirms File.delete removed it",
	arrayLen(directoryList(staleDir, false, "name", "rustcfml_stale_probe.txt")) EQ 0);
assertTrue("fileExists agrees with the filesystem after File.delete",
	NOT fileExists(staleFile));

// Same again for a rename, where the memo must clear for BOTH paths.
// File.renameTo (rather than Files.move) so this runs on the reference engine
// too — Lucee cannot call Files.move with plain strings.
moveFrom = staleDir & "rustcfml_stale_from.txt";
moveTo   = staleDir & "rustcfml_stale_to.txt";
fileWrite(moveFrom, "y");
if (fileExists(moveTo)) { fileDelete(moveTo); }
assertTrue("the move source exists before the move", fileExists(moveFrom));
assertTrue("the move target does not exist before the move", NOT fileExists(moveTo));

createObject("java", "java.io.File").init(moveFrom)
	.renameTo(createObject("java", "java.io.File").init(moveTo));

assertTrue("fileExists sees the move source is gone", NOT fileExists(moveFrom));
assertTrue("fileExists sees the move target arrived", fileExists(moveTo));
if (fileExists(moveTo)) { fileDelete(moveTo); }

suiteEnd();
</cfscript>
