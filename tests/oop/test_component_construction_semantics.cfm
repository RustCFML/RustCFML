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
assert( "inherited methods present after single-run chain", leaf.rootMethod() & "|" & leaf.midMethod() & "|" & leaf.leafMethod(), "mid-override+root-method|mid-method|leaf-method" );
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

// --- a replayed construction (2nd+ of a class in the request) is identical ---
// The engine builds a class's method tables on its first construction and
// attaches them on every later one; nothing observable may differ between them.
function fnNames( md ) { local.n = []; for ( local.f in md.functions ) arrayAppend( local.n, local.f.name ); arraySort( local.n, "textnocase" ); return arrayToList( local.n ); }
first  = new oop.ctorsem.Leaf();
second = new oop.ctorsem.Leaf();
third  = createObject( "component", "oop.ctorsem.Leaf" ).init();
assert( "replay: getMetadata(...).functions identical", fnNames( getMetadata( second ) ), fnNames( getMetadata( first ) ) );
assert( "replay: leaf metadata lists ONLY own functions", fnNames( getComponentMetaData( "oop.ctorsem.Leaf" ) ), "inheritedOnThis,inheritedRefWorks,init,leafMethod,ownKeys,seenFromRoot,thisKeysInCtor,viaSuper" );
assert( "replay: public key list identical", listSort( second.ownKeys(), "textnocase" ), listSort( first.ownKeys(), "textnocase" ) );
assert( "replay: super dispatch through a 3-level chain", third.viaSuper(), "mid-method+mid-override+root-method" );
assert( "replay: overridden method resolves to the override, super to the parent", third.rootMethod(), "mid-override+root-method" );
// A chain declared with RELATIVE extends ("Mid", "Root") is package-qualified
// from the child that declared it, so the path-aware checks match Lucee.
assertTrue( "relative extends chain: isInstanceOf sees the qualified root", isInstanceOf( third, "oop.ctorsem.Root" ) );
assertTrue( "relative extends chain: isInstanceOf sees the qualified middle", isInstanceOf( first, "oop.ctorsem.Mid" ) );
assertFalse( "relative extends chain: a wrong package does not match", isInstanceOf( first, "wrong.ctorsem.Root" ) );
assert( "relative extends chain: metadata extends.name is qualified", getMetadata( first ).extends.name, "oop.ctorsem.Mid" );
assert( "relative extends chain: metadata extends.extends.name is qualified", getMetadata( first ).extends.extends.name, "oop.ctorsem.Root" );
// Inside the subclass pseudo-constructor the parent chain's methods are already
// on `this` (Lucee runs the parents first on the same `this`) — on the FIRST and
// on a replayed construction alike; the leaf metadata above still lists only own.
assert( "pseudo-ctor: inherited methods visible on this (first)", first.inheritedOnThis(), "true/true/true/false/true" );
assert( "pseudo-ctor: inherited methods visible on this (replay)", third.inheritedOnThis(), "true/true/true/false/true" );
assert( "pseudo-ctor: structKeyList(this) is the whole class, no engine keys", lCase( first.thisKeysInCtor() ), "inheritedonthis,inheritedrefworks,init,leafmethod,midmethod,ownkeys,rootmethod,seenfromroot,thiskeysinctor,viasuper" );
assertTrue( "pseudo-ctor: an inherited method reference reads through this", third.inheritedRefWorks() );
second.extra = "per-instance";
assertFalse( "replay: instances do not share public state", structKeyExists( first, "extra" ) );
assertFalse( "replay: no private-scope method leaks onto the public view", structKeyExists( first, "fromRoot" ) );

// --- an empty component is still a component ---------------------------------
e = new oop.ctorsem.Empty();
assertTrue( "empty component is an object", isObject( e ) );
assertTrue( "empty component has its metadata name", findNoCase( "Empty", getMetadata( e ).name ) GT 0 );
assert( "empty component has no public members", structCount( e ), 0 );

suiteEnd();
</cfscript>
