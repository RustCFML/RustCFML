component extends="BaseSpec230" {
    function beforeTests(){
        addAssertions( { isAwesome : function( required expected ){ return "yes:" & expected; } } );
    }
    function testIt(){
        // unqualified read -> variables.myassert
        return myassert.isAwesome( "test" );
    }
}
