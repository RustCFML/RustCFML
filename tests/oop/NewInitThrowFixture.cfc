component {

    // init() guards its argument and throws on an invalid value — the classic
    // constructor-validation pattern. A valid value returns a usable instance.
    function init(numeric windowSeconds = 60) {
        if (arguments.windowSeconds <= 0) {
            throw(type = "Fixture.InvalidConfiguration", message = "windowSeconds must be > 0");
        }
        variables.windowSeconds = arguments.windowSeconds;
        return this;
    }

    public numeric function getWindowSeconds() {
        return variables.windowSeconds;
    }

}
