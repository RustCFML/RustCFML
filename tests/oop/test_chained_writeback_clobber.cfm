<cfscript>
suiteBegin("Chained method call does not clobber base variable");

// Regression: `outer.getDep().mutate()` — a chained call whose OUTER mutating
// method runs on a different CFC than the base variable `outer` — must not
// write that inner CFC back onto `outer`. Codegen propagates the writeback
// path (["outer"]) to both calls in the chain; a single-segment path bypassed
// the chained-CFC identity guard, so `outer` was clobbered and became the
// inner CFC. (Sibling of the deep-path bug #15.)

o = new ChainClobberOuter();
assert("base is Outer before chain", o.whoAmI(), "Outer");

o.getDep().setMark( "X" );

// base variable keeps its identity
assert("base still Outer after chained mutate", o.whoAmI(), "Outer");
// the mutation landed on the shared inner instance (reference semantics)
assert("chained mutation persists on shared dep", o.getDep().getMark(), "X");

// Java-shim chaining must STILL propagate (the guard only gates CFC<->CFC):
sb = createObject( "java", "java.lang.StringBuilder" ).init( "" );
sb.append( "a" ).append( "b" ).append( "c" );
assert("java shim chained mutation still works", sb.toString(), "abc");

// Deep-path (2-segment base) variant: `variables.inner.getStore().put(k,v)` —
// `put` is a mutating method returning a foreign CFC, and the deep
// result-writeback would clobber variables.inner. This is the exact shape that
// broke WireBox app/CF scopes (injector.getScopeStorage().put(...) overwrote
// variables.injector with the ScopeStorage).
dh = new DeepHolder();
dh.setInner( new DeepInner() );
dh.getInner().setStore( new DeepStore() );
assert("deep chained mutating call does not clobber the 2-segment base", dh.probe(), "Inner");

// GH #219: deep chained call where the source-text scope path casing differs
// from the canonical stored key (`variables.Inner` vs `inner`). The identity
// guard walked the write-back path case-sensitively, missed the leaf, and let
// the chained call's foreign return value clobber the base. (TestBox BDDRunner
// async path: `variables.testBox.getUtility().slugify(...)`, stored `testbox`.)
dh2 = new DeepHolder();
dh2.setInner( new DeepInner() );
dh2.getInner().setStore( new DeepStore() );
assert("mixed-case deep chained call does not clobber the base", dh2.probeMixedCase(), "Inner");

// In-place array member fn chained on a method that RETURNS AN ARRAY:
// `o.getItems().sort()` sorts the returned array, but codegen propagates the
// writeback path (["o"]) to the outer .sort(), so the sorted array was written
// back onto `o` — clobbering the component with an array (so the next `o.X()`
// call failed). The guard now skips the writeback when the base holds a CFC and
// the result isn't that same instance. (Surfaced via `new SocketIoServer()`'s
// `arrayToList(io.getRegisteredNamespaces().sort(...))`.)
o2 = new ChainClobberOuter();
sorted = o2.getItems().sort( "textnocase" );
assert("chained array sort returns the sorted array", arrayToList( sorted ), "a,b,c");
assert("base still a component after chained array sort", isObject( o2 ), true);
assert("base method still callable after chained array sort", o2.whoAmI(), "Outer");

// ColdBox blocker #8: single-segment chained call where the base function-local
// is REFERENCED with different casing than it was DECLARED
// (`var cbController` … `cbcontroller.getRenderer().layout()`). The write-back
// guard's scope_aware_load was case-sensitive while scope_aware_store was
// case-insensitive, so the guard missed the local and the chained call's foreign
// `this` overwrote the base. See ChainClobberOuter.probeCaseMismatch().
mm = new ChainClobberOuter();
assert("mixed-case single-segment chained call does not clobber base local", mm.probeCaseMismatch(), "Outer");

suiteEnd();
</cfscript>
