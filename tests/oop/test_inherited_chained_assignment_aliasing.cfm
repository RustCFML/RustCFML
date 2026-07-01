<cfscript>
suiteBegin( "Inherited chained assignment aliasing: parent-body this.x=variables.x shares in subclass (##227)" );

// Direct instantiation of the parent already shared correctly (##221 baseline).
p = createObject( "component", "oop.ChainAliasParent" );
p.inject();
assert( "direct parent: this/variables share", p.probe(), "true/true" );

// Instantiated through a SUBCLASS: the chained assignment ran in the PARENT
// pseudo-constructor, but both names must STILL be one shared reference.
child = createObject( "component", "oop.ChainAliasChild" );
child.inject();
assert( "subclass: this/variables share", child.probe(), "true/true" );
assert( "subclass: unscoped name reaches shared ref", child.callUnscoped(), "hi" );

// Multi-level inheritance must behave the same.
gc = createObject( "component", "oop.ChainAliasGrandChild" );
gc.inject();
assert( "grandchild: this/variables share", gc.probe(), "true/true" );

// Two SEPARATE objects in this.distinct / variables.distinct must STAY
// distinct through a subclass (fix must not over-share by name).
child2 = createObject( "component", "oop.ChainAliasChild" );
child2.injectDistinct();
assert( "subclass: distinct objects stay distinct", child2.probeDistinct(), "true/false" );

// Per-instance isolation: mutating one subclass instance must not affect another.
a = createObject( "component", "oop.ChainAliasChild" );
b = createObject( "component", "oop.ChainAliasChild" );
a.inject();
assert( "subclass instances are isolated", structKeyExists( b.obj, "added" ), false );

suiteEnd();
</cfscript>
