component {
    // This component keeps private state in a struct literally named "instance",
    // exactly like MockBox's MockGenerator does.
    instance = { generationPath = "/gen", mockGenerator = "gen" };
    function init(){ return this; }
    // A method that, when copied onto and invoked on a DIFFERENT receiver, must
    // bind to the receiver's variables scope — not leak this component's own
    // variables.instance onto the target (the MockBox $include clobber, GH #235).
    function injectInto( required any target ){
        target.borrowed = variables.borrowedMethod;
        target.borrowed();
        structDelete( target, "borrowed" );
    }
    function borrowedMethod(){
        // runs bound to the receiver; sets a marker in the receiver's scope
        variables.injected = true;
    }
}
