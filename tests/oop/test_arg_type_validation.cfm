<cfscript>
suiteBegin("Argument type validation for component/interface-typed params (Lucee parity)");

// Lucee throws `expression` when an argument passed to a param declared with a
// component/interface type is not an instance of that type. Primitive types
// (string/numeric/struct/...) stay leniently coerced — only CFC/interface types
// are validated. cfflow's WorkflowImplementationFactory relies on this:
// `registerScheduler( required IWorkflowScheduler implementation )` must reject a
// non-conforming object with an `expression` error.

reg    = new oop.argtype.Registry();
circle = new oop.argtype.Circle();   // implements IShape
square = new oop.argtype.Square();   // does NOT implement IShape

// A conforming instance is accepted (matched by unqualified interface name)...
assert( "conforming instance accepted", reg.register( circle ), "ok:3" );
// ...and by the fully-qualified interface type.
assert( "conforming instance accepted (FQN param type)", reg.registerFqn( circle ), "ok:3" );

// A component that does NOT implement the interface throws `expression`.
assertThrows( "non-conforming component rejected", function(){ reg.register( square ); } );

// A non-component value (plain struct / string) also throws.
assertThrows( "plain struct rejected", function(){ reg.register( { a = 1 } ); } );
assertThrows( "string rejected",       function(){ reg.register( "notashape" ); } );

suiteEnd();
</cfscript>
