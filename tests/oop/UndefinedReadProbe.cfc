component {

    // No init(): a bean can be constructed without its `variables` backing set,
    // exactly the FW/1 InjectPropertiesTest::testInjectWithType scenario.

    // Explicitly-scoped undefined read inside a method body.
    function readVarScoped() {
        return variables.definitelyNotSet;
    }

    // Bare (unscoped) undefined read inside a method body.
    function readUnscoped() {
        return definitelyNotSetEither;
    }

    // Catch the undefined read *within* the same method frame and report the
    // type the handler observed (the in-handler, same-frame path).
    function readAndReportType() {
        try {
            return variables.stillNotSet;
        } catch (any e) {
            return e.type;
        }
    }
}
