/**
 * Fixture for getFunctionCalledName() when a component method is REPLACED in-scope
 * by a differently-named stub — the FW/1 beanProxy / AOP shape. beanProxy stashes
 * the original method under a `___`-prefixed key and overwrites the public/private
 * method slot with a shared stub (`$callPublicMethod`/`$callPrivateMethod`); the
 * stub calls getFunctionCalledName() to recover which method was actually invoked
 * and dispatch the interceptor stack for it. RustCFML must report the ALIAS the
 * function was called under (the storage key), not the stub's declared name —
 * INCLUDING when the aliased call is nested inside another aliased call's argument
 * (the callee is loaded before its args, so the naive pending-name channel would
 * be drained by the inner call before the outer call consumes it). Closures keep
 * their generated name (their prefix drives live-capture refresh), so this fix is
 * scoped to genuine named methods.
 */
component {
    // A single shared stub, injected under several method names.
    private any function stub() { return getFunctionCalledName(); }

    // A public method invoked directly through its alias slot.
    public  any function pubAlias() { return "orig-pub"; }

    // Unqualified internal calls to (replaced) private methods, both single and
    // nested — the exact FW/1 advReverse.doWrap( doRear( doFront( x ) ) ) shape.
    public any function callSingle() { return inner1(); }
    public any function callNested() { return outer1( inner1() ); }
    private any function inner1() { return "orig-inner"; }
    private any function outer1( any x ) { return "orig-outer"; }

    // Install the stub over every method, stashing originals — mirrors
    // beanProxy.$replaceMethod (public replaced in `this`, private in `variables`).
    public any function install() {
        this[ "___pubAlias" ] = this.pubAlias;
        this.pubAlias         = variables.stub;
        variables[ "___inner1" ] = variables.inner1;
        variables.inner1         = variables.stub;
        variables[ "___outer1" ] = variables.outer1;
        variables.outer1         = variables.stub;
        return this;
    }
}
