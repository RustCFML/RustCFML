component {
    instance = {};
    reset();
    function init(){ return this; }
    function reset(){ instance.customDSL = {}; instance.properties = { seeded = true }; }
    function getCustomDSL(){ return instance.customDSL; }
    function getProperties(){ return instance.properties; }
    function instanceKeys(){ return structKeyList( instance ); }
    function wasInjected(){ return structKeyExists( variables, "injected" ) && variables.injected; }
}
