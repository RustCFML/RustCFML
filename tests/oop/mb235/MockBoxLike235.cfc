component {
    // Mimics TestBox 5.4.0 MockBox: keeps its OWN private state in a struct
    // literally named "instance" (exactly the key collision behind GH #235).
    instance = {};
    function init(){
        instance.generationPath = "/gen/";
        instance.mockGenerator  = "gen-object";
        return this;
    }

    // MockBox.decorateMock: copies its `$` method onto the target and stashes a
    // back-reference to itself as `obj.mbox`.
    function decorate( required any obj ){
        obj.$    = variables.$;
        obj.mbox = this;
    }

    function getMockGenerator(){ return instance.mockGenerator; }

    // MockBox.$ : this method is INJECTED onto the target and invoked THERE
    // (`this` becomes the target). It then makes a NESTED call back into the
    // MockBox object (`this.mbox.getMockGenerator()`). That nested scope-prefixed
    // call is what used to splice MockBox's own `variables.instance` onto the
    // target's `variables.instance`, wiping the target's private state.
    function $( required string method ){
        var gen = this.mbox.getMockGenerator();
        return "mocked " & arguments.method & " via " & gen;
    }
}
