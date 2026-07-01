component {
    // Chained assignment in the PARENT pseudo-constructor. When the object is
    // instantiated through a SUBCLASS, both names must still reference ONE
    // object (GitHub #227). Pre-fix, the subclass got a COPY in variables.obj
    // while this.obj kept the shared reference, so a mutation of this.obj (via
    // the parent's own method) was invisible through variables.obj.
    variables.obj = this.obj = new ChainAliasInner();

    // Two SEPARATE objects assigned to same-named this/variables members must
    // STAY distinct in the subclass too (fix must not over-share).
    this.distinct = new ChainAliasInner();
    variables.distinct = new ChainAliasInner();

    function inject(){
        this.obj[ "added" ] = function(){ return "hi"; };
    }
    function injectDistinct(){
        this.distinct[ "m" ] = function(){ return 1; };
    }
    function probe(){
        return structKeyExists( this.obj, "added" ) & "/" & structKeyExists( variables.obj, "added" );
    }
    function probeDistinct(){
        return structKeyExists( this.distinct, "m" ) & "/" & structKeyExists( variables.distinct, "m" );
    }
    // Resolve the unscoped name (as TestBox specs do with $assert): must reach
    // the shared reference, not a stale variables-scope copy.
    function callUnscoped(){
        return obj.added();
    }
}
