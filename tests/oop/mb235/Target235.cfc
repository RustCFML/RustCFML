component {
    instance = {};
    reset();
    function init(){ return this; }
    function reset(){ instance.customDSL = {}; instance.properties = { seeded = true }; }
    function getCustomDSL(){ return instance.customDSL; }
    function getProperties(){ return instance.properties; }
    // Sorted and case-folded: Lucee upper-cases dot-notation keys and does not
    // keep struct insertion order; the key SET is what the tests check.
    function instanceKeys(){ return listSort( lcase( structKeyList( instance ) ), "text" ); }
    function wasInjected(){ return structKeyExists( variables, "injected" ) && variables.injected; }
}
