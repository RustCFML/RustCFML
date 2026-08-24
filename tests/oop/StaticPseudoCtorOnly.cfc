/**
 * The GH #353 repro in its minimal form: a body write with NO `static {}` block
 * anywhere, so nothing else can be creating the scope.
 */
component {
    static.FromCtor = "ctor";
    function get() { return static.FromCtor ?: "(null)"; }
}
