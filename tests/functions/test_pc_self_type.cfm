<!---
  A self-typed method called from a pseudo-constructor — docs/known-issues.md §35.

  `component { reset(); MyType function reset(){ return this; } }` threw
  "The function [reset] has an invalid return value , [Cannot cast Object type
  [Component Anonymous] to a value of type [MyType]]". A component's `this` carries
  the parser's "Anonymous" placeholder for as long as its pseudo-constructor body
  runs — the real name is stamped on afterwards — so the §29 return check had
  nothing to match. getMetadata() already compensated from the same
  in-construction stack; the type checker now does too.

  This is ColdBox's LogBoxConfig verbatim, and it stopped Preside booting on
  v0.557.0. Green on RustCFML and Lucee 7.
--->
<cfscript>
suiteBegin( "Self-typed method called from a pseudo-constructor (§35)" );

// Constructing at all is the regression: the PC calls reset(), whose declared
// return type is the component's own.
obj = new pc_self_type_target();
assert( "component constructs", obj.marker(), "constructed" );

// The same call once construction has finished must still pass — by then `this`
// can name itself, so this proves the normal path was not broken to fix the
// in-construction one.
assert( "self-typed method after construction", obj.reset().marker(), "constructed" );
assert( "nested self-typed call", obj.resetTwice().marker(), "constructed" );

// And the instance really is its declared type.
assertTrue( "instance is its own type", isInstanceOf( obj, "pc_self_type_target" ) );

suiteEnd();
</cfscript>
