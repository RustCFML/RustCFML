component extends="SuperCtorParent" {
    // super.method() in the pseudo-constructor writes this.* members BEFORE the
    // body materializes `this`. Those writes must persist onto the instance
    // (Lucee parity). Regression: lost since v0.468 (fixed by the
    // pseudo_ctor_super_this_writes stash).
    super.setupIt( tag="C" );
    // Child overrides one parent-set member (parent runs first, child wins).
    this.num = 99;

    function readAll() {
        return {
              fromSuper = this.fromSuper ?: "UNDEF"
            , num       = this.num ?: "UNDEF"
        };
    }
}
