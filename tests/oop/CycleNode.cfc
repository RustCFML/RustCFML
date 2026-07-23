component {
    // Plain component; public members are assigned externally by the test to
    // form self-referential / mutually-referential instance graphs. Used to
    // regression-guard serializeJSON / Serialize() / writeDump against the
    // flyweight-Instance re-opening of the GH #178 circular-reference abort.
}
