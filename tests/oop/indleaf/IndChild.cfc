// Subclass in a DIFFERENT dir (oop/indleaf/). go() is defined HERE and calls
// the INHERITED makeSibling() (defined in oop/indbase/). This is the Wheels
// migrator shape: a migration's own up() calls the inherited createTable().
component extends="oop.indbase.IndBase" {
    public string function go() { return makeSibling(); }
}
