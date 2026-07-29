component {
    /**
     * Mirrors ColdBox EventHandler._privateInvoker: pulls a method out of the
     * variables scope as a VALUE and calls it, so dispatch has no receiver to
     * heal against and must resolve the function id directly.
     */
    public any function privateInvoker( required string method ) {
        var _targetAction  = variables[ arguments.method ];
        var _targetResults = _targetAction();
        if ( !isNull( _targetResults ) ) { return _targetResults; }
    }
}
