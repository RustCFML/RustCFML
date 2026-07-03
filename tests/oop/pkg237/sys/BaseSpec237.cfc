component {
    // Unqualified `new Expectation237()` is LEXICALLY written here (package
    // oop.pkg237.sys), so it must resolve to oop.pkg237.sys.Expectation237 even
    // when this method is inherited and invoked via a subclass in another
    // package (GH #237 — the fully-qualified metadata.name must be the DEFINING
    // file's package, not the concrete subclass's).
    function makeExpectation(){ return new Expectation237(); }
}
