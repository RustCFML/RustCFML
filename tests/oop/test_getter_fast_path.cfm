<cfscript>
// One-expression getters (`return x;`, `return variables.x;`, `return a.b;`,
// `return variables.x ?: y;`) are answered without building a call frame.
// These pin that every observable result is the one the full call produces:
// hits, misses and their errors, return-type checks, access rules, inheritance,
// a caller's same-named locals, and values changed after the first call.
suiteBegin("Getter fast path");

c = new oop.getter.GetterChild();

assert("bare sibling calls, every getter shape",
	serializeJSON( c.bareShapes() ), serializeJSON( [ 1, "rs", "inst", "inst", "base", "fallback" ] ));
assert("this.getter() calls",
	serializeJSON( c.memberShapes() ), serializeJSON( [ "rs", "inst", "base", "fallback" ] ));
assert("external member call", c.getV(), "inst");
assert("external member call, property path", c.getRequestService(), "rs");
assert("inherited getter, external call", c.getBaseVal(), "base");
assert("elvis getter, miss returns the fallback", c.getMissingElvis(), "fallback");

// A caller's own locals are not the callee's variables.
assert("caller locals do not shadow the getter's variables",
	serializeJSON( c.bareWithShadowingLocal() ), serializeJSON( [ 1, "inst", "inst" ] ));

// The value is read live, not remembered from an earlier call.
c.setInstance( "changed" );
assert("getter sees a later write (member call)", c.getV(), "changed");
assert("getter sees a later write (bare call)", c.bareShapes()[ 3 ], "changed");
c.clearInstance();
assertThrows("variables.x getter throws once the key is gone", function() { c.getV(); });
assert("elvis getter falls back once the key is gone", c.elvisInstance(), "none");

// Misses and type checks still raise.
assertThrows("declared return type is still enforced", function() { c.getWrongType(); });
assertThrows("undefined bare name still throws", function() { c.getUndefined(); });
assertThrows("missing property still throws", function() { c.getMissingKey(); });
assertThrows("private getter is not callable from outside", function() { c._getObjects(); });
assert("private getter is callable from inside", c.privateGetterFromInside().a, 1);

// A getter returning a function value hands back a callable function.
f = c.getFn();
assert("getter returning a function value", f(), "called");

// Extra arguments do not change the answer.
c2 = new oop.getter.GetterChild();
assert("getter called with an extra positional argument", c2.getV( "ignored" ), "inst");

// A page-level UDF getter reads (and keeps reading) the page's variables.
pageGetterValue = "page";
function pageGetter() { return pageGetterValue; }
function pageGetterScoped() { return variables.pageGetterValue; }
assert("page UDF getter", pageGetter(), "page");
assert("page UDF variables.x getter", pageGetterScoped(), "page");
pageGetterValue = "page2";
assert("page UDF getter sees a later write", pageGetter(), "page2");

// `return request.x ?: <fallback>` — the fallback runs once, then the hit.
structDelete( request, "__getterTestCtx" );
rc1 = c2.getRequestCtx();
assertTrue("request elvis: first call runs the fallback", rc1.made);
rc1.marker = 1;
assert("request elvis: later calls read request scope (member)", c2.getRequestCtx().marker, 1);
assert("request elvis: later calls read request scope (bare)", c2.requestCtxBare().marker, 1);
request.__getterTestCtx = { replaced = true };
assertTrue("request elvis: sees a replaced value", c2.getRequestCtx().replaced);
structDelete( request, "__getterTestCtx" );

// Engine-generated accessors take the same path.
a = new oop.getter.GetterAccessors();
assert("generated getter, member call", a.getLevelMax(), 4);
assert("generated getter, bare call", a.readBare(), 4);
a.setLevelMax( 7 );
assert("generated getter sees the setter", a.getLevelMax(), 7);
assertTrue("generated getter of a never-set property returns null", isNull( a.getLabel() ));
a.setLabel( "x" );
assert("generated getter after set", a.getLabel(), "x");

suiteEnd();
</cfscript>
