component {
    function init() {
        return this;
    }

    // Bare (unqualified) name via createObject("component", "Sibling").
    // Lucee/ACF/BoxLang resolve a bare component name relative to the
    // CALLING CFC's package (oop.relcomp) first, so this finds the
    // sibling. Wrapped so a resolution failure surfaces as a wrong
    // VALUE ("ERR:..."), keeping the test runtime-level / runner-safe.
    function viaCreate() {
        try {
            return createObject("component", "Sibling").hi();
        } catch (any e) {
            return "ERR:" & e.message;
        }
    }
}
