/**
 * A perfectly ordinary CFC, instantiated and driven from Rust.
 *
 * @hint A component the demo extension constructs, injects into, and calls.
 */
component {

    // Set by the extension via set_property — dependency injection, from Rust.
    variables.injected = "(not injected)";

    public string function hello() {
        return "Greeter.hello() says: " & variables.injected;
    }

    public numeric function double( required numeric n ) {
        return arguments.n * 2;
    }
}
