/**
 * A component with NO `static {}` block at all.
 *
 * Lucee gives every component a static scope whether or not it declares one, so
 * `static.X = v` from a method persists and is shared across instances. RustCFML
 * used to create the scope only for components that DECLARED a block; without one
 * the write landed in the method's own locals and vanished at frame exit, while
 * still reporting success (GH #347 — silent data loss).
 */
component {
    function setIt()  { static.X = "v"; return "set-ok"; }
    function getIt()  { return static.X ?: "(null)"; }
    function keyList(){ return structKeyList( static ); }
    function bump()   { static.N = ( static.N ?: 0 ) + 1; return static.N; }
}
