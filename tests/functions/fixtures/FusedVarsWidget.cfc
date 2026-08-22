component {
    function init( a, b ) {
        variables.a = arguments.a;
        variables.b = arguments.b;
        return this;
    }
    function total()       { return variables.a + variables.b; }
    function bumpAndRead() { variables.a = variables.a + 1; return variables.a + variables.b; }
    function sibling()     { return "sibling:" & ( variables.a + variables.b ); }
    function callSibling() { var m = variables.sibling; return m(); }
    function extractSibling() { return variables.sibling; }
}
