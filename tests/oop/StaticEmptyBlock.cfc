/** A `static {}` block that declares nothing — same expectations as StaticNone. */
component {
    static { }
    function setIt() { static.X = "v"; return "set-ok"; }
    function getIt() { return static.X ?: "(null)"; }
}
