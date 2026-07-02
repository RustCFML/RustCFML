component {
    variables.myassert = this.myassert = new Assertion230();
    function addAssertions( required any assertions ){
        structAppend( this.myassert, arguments.assertions, true );
        return this;
    }
}
