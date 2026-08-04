/**
 * A method declared to return its OWN type, called from the PSEUDO-CONSTRUCTOR —
 * the shape of ColdBox's LogBoxConfig (docs/known-issues.md §35). While the
 * pseudo-constructor runs, `this` still carries the parser's "Anonymous"
 * placeholder name, so the §29 return check could not match it against the
 * declared type and Preside stopped booting.
 */
component accessors="true" {
    instance = structNew();
    reset();                                    // called DURING construction

    function init() {
        return this;
    }

    pc_self_type_target function reset() {
        instance.appenders = structNew();
        return this;
    }

    // Same thing, reached one level deeper: a PC-called method that calls
    // another self-returning one.
    pc_self_type_target function resetTwice() {
        return reset();
    }

    function marker() {
        return "constructed";
    }
}
