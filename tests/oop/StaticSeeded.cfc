/**
 * A `static {}` block that writes through the SCOPE (`static.Seed = ...`) rather
 * than bare (`Seed = ...`). The scoped form left the scope reference itself
 * behind in the init frame's locals, which was then captured as a member — so
 * every seeded static scope carried a self-referential `static` key and
 * `for ( k in static )` iterated a phantom entry (GH #347).
 */
component {
    static { static.Seed = "s"; }
    function setIt()   { static.X = "v"; return "set-ok"; }
    function getIt()   { return static.X ?: "(null)"; }
    function keyList() { return structKeyList( static ); }
}
