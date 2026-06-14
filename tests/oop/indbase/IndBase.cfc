// Base in oop/indbase/. Its method does a BARE CreateObject of its
// package-sibling IndSibling (also in oop/indbase/). The bare name must
// resolve relative to THIS file's dir (where the literal is defined),
// regardless of which subclass — or which OUTER frame — the call runs under.
component {
    public string function makeSibling() {
        try { return CreateObject("component", "IndSibling").hi(); }
        catch (any e) { return "ERR:" & e.message; }
    }
}
