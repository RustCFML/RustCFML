<cfscript>
suiteBegin("Static blocks and static scope");

// --- static scope reads inside instance methods -------------------------
a = new oop.StaticConsole();
assert("static scalar read", a.greet(), "hello");
assert("static struct dot read", a.colorRed(), chr(27) & "[31m");
assert("static struct dynamic key read", a.colorByKey("green"), chr(27) & "[32m");

// --- static scope is shared per type ------------------------------------
// Use RELATIVE assertions so the test is agnostic to whether statics persist
// for the application lifetime (Lucee/BoxLang) or per request.
before = a.getCount();
a.bump();
assert("bump increments shared counter", a.getCount(), before + 1);
b = new oop.StaticConsole();
assert("new instance sees shared count", b.getCount(), before + 1);
b.bump();
assert("mutation through one instance visible on another", a.getCount(), before + 2);

// --- static functions ---------------------------------------------------
// Callable on an instance, reading static scope.
assert("static fn on instance", a.wrap("green", "X"),
    chr(27) & "[32m" & "X" & chr(27) & "[0m");
// Callable via the `::` operator without an instance.
assert("static fn via ::", oop.StaticConsole::wrap("red", "Y"),
    chr(27) & "[31m" & "Y" & chr(27) & "[0m");

// --- `::` static member access (no instance) ----------------------------
assert("static var via ::", oop.StaticConsole::GREETING, "hello");

// --- getComponentStaticScope() ------------------------------------------
// The name-string form is the Lucee-documented signature (portable).
s2 = getComponentStaticScope("oop.StaticConsole");
assert("getComponentStaticScope by name", s2.GREETING, "hello");
// Passing a component instance is a RustCFML convenience (Lucee accepts a
// name string only), so guard it to keep the suite green on Lucee.
if (isRustCFML()) {
    s1 = getComponentStaticScope(a);
    assert("getComponentStaticScope on instance", s1.GREETING, "hello");
}

// --- <cfstatic> tag form ------------------------------------------------
t = new oop.StaticTagForm();
assert("cfstatic scoped write", t.scoped(), "from-cfstatic");
assert("cfstatic unscoped write", t.plainVal(), 7);

// --- a static scope exists WITHOUT a static block (GH #347) --------------
// Lucee: every component has a static scope. Writing to it from a method
// persists and is shared by every instance of that type, whether or not the
// component declares `static { }`. Gating the scope's existence on the
// declaration made the write silently vanish — `setIt()` returned "set-ok" and
// the value was gone, which is the damaging shape: nothing throws, so a static
// counter or cache built this way just never accumulates.
noneA = new oop.StaticNone();
assert("write to an undeclared static scope reports success", noneA.setIt(), "set-ok");
assert("...and the value is still there", noneA.getIt(), "v");
assert("...and a FRESH instance sees it (per-type, not per-instance)", new oop.StaticNone().getIt(), "v");
assertTrue("static key list contains X", listFindNoCase( noneA.keyList(), "X" ) gt 0);
// A counter is the case that made the loss visible: it must accumulate across
// instances rather than restart at 1 every call. Measured as a DELTA, not an
// absolute — a static scope outlives the request, so the starting value depends
// on how many times this suite has already run in this server (Lucee and our own
// serve mode both persist it; only a fresh CLI process starts at zero).
bumpBase = new oop.StaticNone().bump();
new oop.StaticNone().bump();
assert("static counter accumulates across instances", new oop.StaticNone().bump(), bumpBase + 2);

// An EMPTY block is not a declaration of any key, and behaves the same way.
emptyA = new oop.StaticEmptyBlock();
assert("write to an empty static block reports success", emptyA.setIt(), "set-ok");
assert("...and the value is still there", emptyA.getIt(), "v");

// A scope written through `static.X` in the block must not end up containing a
// self-referential `static` entry — `for ( k in static )` iterated a phantom key.
seeded = new oop.StaticSeeded();
assert("seeded static block still works", seeded.setIt(), "set-ok");
assert("seeded static value readable", seeded.getIt(), "v");
assertTrue("seeded scope keeps its declared key", listFindNoCase( seeded.keyList(), "Seed" ) gt 0);
assertTrue("seeded scope keeps the written key", listFindNoCase( seeded.keyList(), "X" ) gt 0);
assertFalse("static scope has no phantom 'static' key",
            listFindNoCase( seeded.keyList(), "static" ) gt 0);

// --- static inheritance -------------------------------------------------
kid = new oop.StaticKid();
assert("child reads own static", kid.ownValue(), "kid-only");
assert("child reads inherited static", kid.inheritedGreeting(), "hello");
assert("inherited static via ::", oop.StaticKid::GREETING, "hello");

// --- GH #353: a write from the PSEUDO-CONSTRUCTOR ------------------------
// `static.X = v` in the component body is an ordinary statement, not a member
// modifier. It used to be swallowed by the parser: `static` was consumed as a
// modifier whatever followed, so the write vanished silently and the read came
// back null. Both the dotted and the bracket spelling must land, and every
// legal use of `static` as a real modifier must keep working.
pseudoOnly = new oop.StaticPseudoCtorOnly();
assert("pseudo-constructor static write persists", pseudoOnly.get(), "ctor");
// Shared across instances, like any other static member.
assert("...and is shared with a second instance", new oop.StaticPseudoCtorOnly().get(), "ctor");

pseudo = new oop.StaticPseudoCtor();
assert("body write alongside a static block", pseudo.readCtor(), "ctor");
assert("bracket-form body write", pseudo.readBracket(), "bracket");
assert("the static block still runs", pseudo.readBlock(), "block");
assert("static modifiers still parse in every position",
       pseudo.callModifiers(), "sf/stf/psf");

suiteEnd();
</cfscript>
