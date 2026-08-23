<cfscript>
// Engine fixes surfaced while booting ColdBox / Preside on RustCFML. Every
// assertion here is cross-engine verified against Lucee 7 — the mapping test
// deliberately uses the `application action="update"` path (the one ColdBox's
// LuceeMappingHelper actually uses and that Lucee supports), NOT the ACF-only
// `getApplicationMetadata().mappings[x]=y` mutation which Lucee ignores.
suiteBegin("ColdBox/Preside boot engine fixes (Lucee-parity)");

// ---- comma-less month-name date parsing --------------------------------
// cbsecurity's JwtService builds its epoch base with "January 1 1970 00:00"
// (no comma). Lucee/ACF accept month-name dates with or without the comma.
d = parseDateTime("January 1 1970 00:00");
assert("parseDateTime('January 1 1970 00:00') year",  year(d),  1970);
assert("parseDateTime('January 1 1970 00:00') month", month(d), 1);
assert("parseDateTime('January 1 1970 00:00') day",   day(d),   1);
assert("parseDateTime abbrev comma-less 'Jan 1 1970 00:00:00'", year(parseDateTime("Jan 1 1970 00:00:00")), 1970);
dOnly = parseDateTime("January 1 1970");
assert("parseDateTime date-only comma-less year", year(dOnly), 1970);
assert("parseDateTime date-only comma-less day",  day(dOnly),  1);

// ---- stackTrace key on every exception ---------------------------------
// Lucee/ACF expose e.stackTrace (a string). ColdBox's ModuleService/RestHandler
// read it directly; a missing member would throw "stackTrace is undefined".
stackOk = false;
try { throw(type="Custom", message="boom-marker"); }
catch(any e){ stackOk = isSimpleValue(e.stackTrace) && findNoCase("boom-marker", e.stackTrace) GT 0; }
assertTrue("caught exception exposes a stackTrace string containing the message", stackOk);

// ---- array argumentCollection is positional ----------------------------
// invoke(obj, "setEmail", [value]) must pass the array element as a single
// positional arg (ColdBox BeanPopulator), NOT store the whole array.
bean = new bootfix.BeanTarget();
invoke(bean, "setEmail", ["alex@example.com"]);
invoke(bean, "setAge", [42]);
assertTrue("array argColl -> setEmail stored a scalar", isSimpleValue(bean.getEmail()));
assert("array argColl -> setEmail value", bean.getEmail(), "alex@example.com");
assert("array argColl -> setAge value", bean.getAge(), 42);

// ---- string-list HOF member methods ------------------------------------
// Member forms route through builtins (which can't run a closure), so the VM
// handles them inline. ColdBox's RoutingService uses list.listFilter(cb).
lst = "a,b,c,d";
assert("list.listFilter", lst.listFilter(function(i){ return i != "b"; }), "a,c,d");
assert("list.listMap",    lst.listMap(function(i){ return ucase(i); }),     "A,B,C,D");
assert("list.listReduce", lst.listReduce(function(acc,i){ return acc & i; }, ""), "abcd");
listEachCount = 0;
lst.listEach(function(i){ listEachCount++; });
assert("list.listEach ran once per item", listEachCount, 4);
assertTrue("list.listEvery all len==1", lst.listEvery(function(i){ return len(i) == 1; }));
assertFalse("list.listEvery detects a violation", lst.listEvery(function(i){ return i == "a"; }));
assertTrue("list.listSome finds a match", lst.listSome(function(i){ return i == "c"; }));
assertFalse("list.listSome none match", lst.listSome(function(i){ return i == "z"; }));

// ---- injected-method receiver binding (WireBox virtual inheritance) ----
// A method member-extracted from Origin and injected into Host, then called by
// bare name as a member of Host, binds this to HOST (call-site binding) and
// getFunctionCalledName() reports the injected name ("newInstance").
host = new bootfix.Host();
host.setup();
injectedResult = host.run();
assert("injected method binds this to the receiver (Host)", listFirst(injectedResult, ":"), "HOST");
assertTrue("injected method getFunctionCalledName is the injected name",
    compareNoCase(listRest(injectedResult, ":"), "newInstance") == 0);

// ---- runtime CF mapping via application action=update ------------------
// (cross-engine path). Register a module mapping at runtime and confirm it
// resolves for both createObject and expandPath.
modDir = getDirectoryFromPath(getCurrentTemplatePath()) & "bootfix/mod";

// action="update" REPLACES the application's mapping set; it does NOT merge
// into it. That is asserted here rather than assumed, because it is the reason
// the merge idiom below is mandatory rather than stylistic — a bare update
// drops every mapping the application declared, and nothing warns you.
//
// This suite used to do exactly that. It was invisible while this engine
// merged, and on Lucee it silently took out /oop, /core and /tags for the rest
// of the request: 67 later test files aborted with "can't find component".
priorMaps = getApplicationMetadata().mappings;
application action="update" mappings="#{ '/bootfixmod' = modDir }#";
assertFalse("action=update REPLACES the set: a mapping not passed stops resolving",
    directoryExists(expandPath("/wheelsmapprobe/")));
assertTrue("action=update leaves the server-level webroot mapping alone",
    directoryExists(expandPath("/")));

// Restore by merging, which is what real code has to do — and what ColdBox's
// LuceeMappingHelper.addMapping does: read the current set, add to it, write
// the whole thing back.
priorMaps[ "/bootfixmod" ] = modDir;
application action="update" mappings="#priorMaps#";
assertTrue("the merge idiom brings a previously declared mapping back",
    directoryExists(expandPath("/wheelsmapprobe/")));

// An update that does not mention mappings at all must leave them alone —
// otherwise the replace above would make every unrelated action=update
// destructive. Verified on Lucee 7.1.0.204.
application action="update" sessionTimeout="#createTimeSpan(0,0,45,0)#";
assertTrue("action=update without a mappings attribute leaves mappings alone",
    directoryExists(expandPath("/wheelsmapprobe/")));

mapWidget = createObject("component", "bootfixmod.Widget");
assert("runtime mapping resolves createObject", mapWidget.hello(), "widget-hello");
assertTrue("runtime mapping resolves expandPath", fileExists(expandPath("/bootfixmod/Widget.cfc")));

// Lucee parity: mutating the struct returned by getApplicationMetadata() must
// NOT register a live mapping. Lucee returns a detached copy, so this mutation
// is a no-op — expandPath of the fake mapping must not resolve. (RustCFML used
// to make this persist, ACF-style; that divergence was removed.)
md = getApplicationMetadata();
md.mappings["/bootfixghost"] = modDir;
assertFalse("getApplicationMetadata().mappings mutation does NOT register (Lucee parity)",
    fileExists(expandPath("/bootfixghost/Widget.cfc")));

// ---- relative file BIF falls back to the executing CFC's directory ------
// A relative fileRead("./x") resolves against the BASE template first, then
// falls back to the currently-executing template's OWN directory when the file
// isn't at the base (verified on Lucee 7). Preside's PasswordStrengthAnalyzer
// reads "./symbolClasses.json" sitting next to the CFC exactly this way — the
// admin console 500'd without this. relprobe.json exists only next to the CFC,
// not at the runner's base dir.
relReader = new bootfix.reldir.RelReader();
assert("relative fileRead falls back to executing CFC dir (Lucee parity)",
    relReader.read(), "CFC_ADJACENT");

// ---- break/continue inside a closure (no enclosing loop) ----------------
// `break` inside an `.each()`/closure callback ends the current invocation and
// iteration continues (Lucee parity) — it must NOT compile to a jump-to-0 that
// loops the closure forever. Preside's admin Layout.localePicker does exactly
// `locales.each(function(l){ if (...) { ...; break; } })`; the old behavior
// pinned a CPU core at 100% and hung the admin console indefinitely.
breakOut = "";
[1,2,3,4].each(function(x){
    if (x == 2) { break; }
    breakOut &= x;
});
assert("break inside .each() ends the callback, iteration continues", breakOut, "134");
contOut = "";
[1,2,3,4].each(function(x){
    if (x == 3) { continue; }
    contOut &= x;
});
assert("continue inside .each() ends the callback, iteration continues", contOut, "124");
// break used to find a match then stop appending — the localePicker pattern.
selected = "";
["a","b","c"].each(function(item){
    if (item == "b") { selected = item; break; }
});
assert("break inside .each() captures the match (localePicker pattern)", selected, "b");

// ---- directoryList filter semantics (compiled once per call) ------------
// The filter is compiled once per directoryList invocation (a wildcard-free
// name is an exact case-insensitive match; a glob compiles to one regex),
// instead of recompiling a regex for every directory entry. Preside's Sticker
// resolves each asset with an exact-name filter over a ~1000-file icon dir
// hundreds of times per request — the per-entry recompile pinned /admin at
// ~100s of CPU. Semantics must stay identical to Lucee.
bfDir = getDirectoryFromPath(getCurrentTemplatePath()) & "bootfix";
exactHit = directoryList(bfDir, false, "name", "Host.cfc");
assertTrue("directoryList exact-name filter matches one file",
    arrayLen(exactHit) == 1 && exactHit[1] == "Host.cfc");
globHit = directoryList(bfDir, false, "name", "*.cfc");
assertTrue("directoryList glob '*.cfc' finds all components (>=3)", arrayLen(globHit) GTE 3);
assert("directoryList non-matching filter returns empty",
    arrayLen(directoryList(bfDir, false, "name", "nope.xyz")), 0);

suiteEnd();
</cfscript>
