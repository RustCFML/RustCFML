/**
 * Fixture for the Phase C.2.2 component-instance flyweight prototype.
 * Exercises: constructor member writes (this + variables), public method,
 * private method called from a public one, unscoped var access, mutation of
 * both the `variables` and `this` scopes via methods, and fluent `return this`.
 * Behaviour must be identical whether backed by a marker struct (default) or
 * the flyweight Instance (feature `component-instance` + allowlist).
 */
component accessors="false" {

    variables.counter = 0;
    this.publicField = "hello";

    function init( required string name ) {
        variables.name = arguments.name;
        this.greeting  = "hi " & arguments.name;
        return this;
    }

    public string function getName() {
        return variables.name;
    }

    public string function greet() {
        return _prefix() & variables.name;
    }

    private string function _prefix() {
        return "Hello, ";
    }

    public numeric function bump() {
        variables.counter++;
        return variables.counter;
    }

    public string function readPublic() {
        return this.publicField;
    }

    public any function setField( required string v ) {
        this.publicField = arguments.v;
        return this;
    }
}
