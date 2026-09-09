<cfscript>
// Pseudo-constructor semantics pinned against Lucee 7.1 (probed 2026-09-09).
// Each of these was a divergence in the construction path before v0.658.0:
//   * a parent pseudo-constructor ran 2^depth times per instantiation
//     (the parent was resolved once for the child's injected scope and again
//     in the inheritance merge, at every level);
//   * the finalize deep-copied constructor-assigned values, so a
//     `variables.cfg = request.cfg` handed the instance a private copy;
//   * a `component name="X"` file left its template in page globals — the next
//     `new X()` skipped the constructor and shared `this` state, and a page
//     variable called X was clobbered;
//   * the body's class-name local leaked into `variables` as a self-reference.
suiteBegin( "Component construction semantics (constructor once per level, live references)" );

// --- each pseudo-constructor runs exactly once, root first --------------------
request.ctorsemLog = "";
leaf = new oop.ctorsem.Leaf();
assert( "new: every level runs once, root first", request.ctorsemLog, "Root,Mid,Leaf" );
assert( "inherited methods present after single-run chain", leaf.rootMethod() & "|" & leaf.midMethod() & "|" & leaf.leafMethod(), "root-method|mid-method|leaf-method" );
assert( "root ctor's variables write visible to the leaf", leaf.seenFromRoot(), "root" );

request.ctorsemLog = "";
leaf2 = createObject( "component", "oop.ctorsem.Leaf" ).init();
assert( "createObject: every level runs once, root first", request.ctorsemLog, "Root,Mid,Leaf" );

request.ctorsemLog = "";
leaf3 = new oop.ctorsem.Leaf();
assert( "second instantiation of the same class: still once per level", request.ctorsemLog, "Root,Mid,Leaf" );

request.ctorsemLog = "";
mid = new oop.ctorsem.Mid();
assert( "two-level class: once per level", request.ctorsemLog, "Root,Mid" );

// --- constructor-assigned values are live references -------------------------
request.ctorsemCfg = {};
r = new oop.ctorsem.RefSem();
z = r.mutate();
assertTrue( "variables.cfg = request.cfg stays a reference (write reaches request)", structKeyExists( request.ctorsemCfg, "x" ) );
assert( "…and carries the written value", structKeyExists( request.ctorsemCfg, "x" ) ? request.ctorsemCfg.x : "MISSING", 2 );
assertTrue( "this.cfg2 = request.cfg stays a reference too", structKeyExists( request.ctorsemCfg, "y" ) );
assert( "variables.both = this.both = {} is ONE struct", z, 4 );

// --- the class-name local does not leak into the variables scope -------------
assertFalse( "no `Anonymous` key in variables", listFindNoCase( r.varKeys(), "Anonymous" ) GT 0 );
assertFalse( "no class-name key in variables", listFindNoCase( r.varKeys(), "RefSem" ) GT 0 );
assertTrue( "real ctor variables still present", listFindNoCase( r.varKeys(), "cfg" ) GT 0 );

// --- `component name="X"`: constructor runs every time, page globals untouched -
request.ctorsemNamedRuns = 0;
NamedCtor = "page variable, must survive";
n1 = new oop.ctorsem.NamedCtor();
n1.x = 99;
n2 = new oop.ctorsem.NamedCtor();
assert( "named component: constructor ran for BOTH instantiations", request.ctorsemNamedRuns, 2 );
assert( "named component: instances do not share this-state", n2.x, 1 );
assert( "named component: same-named page variable is not clobbered", NamedCtor, "page variable, must survive" );

// --- an empty component is still a component ---------------------------------
e = new oop.ctorsem.Empty();
assertTrue( "empty component is an object", isObject( e ) );
assertTrue( "empty component has its metadata name", findNoCase( "Empty", getMetadata( e ).name ) GT 0 );
assert( "empty component has no public members", structCount( e ), 0 );

suiteEnd();
</cfscript>
