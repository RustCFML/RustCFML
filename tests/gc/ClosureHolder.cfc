component {
    public any function init( required string id ) {
        variables.id = arguments.id;
        // Defining a closure here is what keeps the LIVE variables scope.
        variables.reader = function() { return variables.id; };
        variables.writer = function( required string v ) { variables.id = arguments.v; };
        return this;
    }
    public string function readId()          { return variables.id; }
    public string function readViaClosure()  { return variables.reader(); }
    public void   function writeViaClosure( required string v ) { variables.writer( arguments.v ); }
    public void   function setPeer( required any p ) { variables.peer = arguments.p; }
    public string function readPeer()        { return variables.peer.readId(); }
}
