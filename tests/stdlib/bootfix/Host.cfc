component {
    this.marker = "HOST";
    function setup(){
        var origin = new Origin();
        // Member-extract Origin.provide and inject it under a new name — exactly
        // how WireBox's virtual inheritance / buildProviderMixer injects methods.
        variables.newInstance = origin.provide;
        return this;
    }
    function run(){
        // Bare call as a member of THIS component.
        return newInstance();
    }
}
