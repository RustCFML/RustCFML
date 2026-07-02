<cfscript>
suiteBegin("OOP: runtime-added instance methods via structAppend are callable (GH ##230)");

// GH #230: a Function assigned onto a component's public scope AFTER construction
// (via structAppend(instance, {name: closure}, true)) must be callable as a
// method. The reported break was TestBox's addAssertions(): a spec's
//   variables.$assert = this.$assert = new Assertion();   // chained assign in a PARENT
// then structAppend(this.$assert, customAssertions, true), invoked later via the
// unqualified $assert (i.e. variables.$assert). This works once the chained
// assignment shares ONE reference across this/variables (GH #227, v0.382.0) AND
// method dispatch treats a runtime-added Function member as callable.

// 1) direct: append a closure onto a fresh instance, then call it -------------
obj = new oop.pkg229.Widget229();
structAppend( obj, { isAwesome: function( required expected ){ return "yes:" & expected; } }, true );
assert("runtime-added method is present", structKeyExists( obj, "isAwesome" ), true);
assert("runtime-added method is callable", obj.isAwesome( "x" ), "yes:x");

// 2) TestBox shape: chained-assign in PARENT + inherited addAssertions +
//    unqualified read of the shared member in a child method --------------------
spec = new oop.pkg229.ChildSpec230();
spec.beforeTests();
assert("append via this.member is visible through the chained-assigned variables alias",
    spec.testIt(), "yes:test");

suiteEnd();
</cfscript>
