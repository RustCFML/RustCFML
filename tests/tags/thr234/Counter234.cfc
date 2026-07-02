component {
    variables.events = [];
    function init(){ return this; }
    function add( required any e ){ arrayAppend( variables.events, arguments.e ); return this; }
    function count(){ return arrayLen( variables.events ); }
}
