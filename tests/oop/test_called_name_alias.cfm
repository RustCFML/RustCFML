<cfscript>
// getFunctionCalledName() must report the ALIAS a method was invoked under when
// the method slot was replaced in-scope by a differently-named stub (FW/1
// beanProxy / AOP). This unblocked FW/1's CombinedInterceptorsTest / Issue518
// TestInitMethods ("Unable to locate method ($callPublicMethod)"): the proxy stub
// read its own declared name instead of the intercepted method's, so runStacks()
// looked up a non-existent "$callPublicMethod" and threw.
suiteBegin( "getFunctionCalledName() through in-scope method aliasing" );

p = new oop.CalledNameAliasProbe();
p.install();

// 1. Public method replaced in `this` and called directly through its alias slot.
assert( "direct call reports the alias (public slot)", p.pubAlias(), "pubAlias" );

// 2. Unqualified internal call to a private method replaced in `variables`.
assert( "single unqualified aliased call reports alias", p.callSingle(), "inner1" );

// 3. NESTED: outer1( inner1() ) — the inner (argument) call must not steal the
// outer call's alias. Before the fix, the outer call reported the stub's declared
// name because the inner call drained the pending-name channel first.
assert( "nested outer aliased call reports its own alias", p.callNested(), "outer1" );

suiteEnd();
</cfscript>
