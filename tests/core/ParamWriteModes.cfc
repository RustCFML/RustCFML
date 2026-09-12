/**
 * Fixture for test_param_write_modes.cfm (§109/§110): how bare writes to a
 * parameter land under localmode="modern" vs classic, on Lucee 7.1.
 */
component {
    function concatModern( a ) localmode="modern" {
        a &= "X";
        return ( structKeyExists( local, "a" ) ? "local.a=" & local.a : "no-local" ) & " args.a=" & arguments.a & " bare=" & a;
    }
    function plusModern( a ) localmode="modern" {
        a += 1;
        return ( structKeyExists( local, "a" ) ? "local.a=" & local.a : "no-local" ) & " args.a=" & arguments.a & " bare=" & a;
    }
    function assignModern( a ) localmode="modern" {
        a = a & "X";
        return ( structKeyExists( local, "a" ) ? "local.a=" & local.a : "no-local" ) & " args.a=" & arguments.a & " bare=" & a;
    }
    function concatTwiceModern( a ) localmode="modern" {
        a &= "X"; a &= "Y";
        return "args.a=" & arguments.a & " bare=" & a;
    }
    function concatThenAssignModern( a ) localmode="modern" {
        a &= "X"; a = "Z";
        return "args.a=" & arguments.a & " bare=" & a;
    }
    function assignThenConcatModern( a ) localmode="modern" {
        a = "Z"; a &= "X";
        return "args.a=" & arguments.a & " bare=" & a & ( structKeyExists( local, "a" ) ? " local.a=" & local.a : " no-local" );
    }
    function varThenConcatModern( a ) localmode="modern" {
        var a = "V"; a &= "X";
        return "args.a=" & arguments.a & " bare=" & a;
    }
    function nonParamConcatModern() localmode="modern" {
        b = "B"; b &= "X";
        return ( structKeyExists( local, "b" ) ? "local.b=" & local.b : "no-local" ) & " vars.b=" & ( structKeyExists( variables, "b" ) ? variables.b : "none" );
    }
    function concatClassic( a ) {
        a &= "X";
        return "args.a=" & arguments.a & " bare=" & a;
    }
    function deleteThenWriteClassic( a ) {
        structDelete( arguments, "a" );
        a = "W";
        return "vars.a=" & ( structKeyExists( variables, "a" ) ? variables.a : "none" ) & " local.a=" & ( structKeyExists( local, "a" ) ? local.a : "none" ) & " args.a=" & ( structKeyExists( arguments, "a" ) ? arguments.a : "none" );
    }
    function deleteThenWriteModern( a ) localmode="modern" {
        structDelete( arguments, "a" );
        a = "W";
        return "vars.a=" & ( structKeyExists( variables, "a" ) ? variables.a : "none" ) & " local.a=" & ( structKeyExists( local, "a" ) ? local.a : "none" ) & " args.a=" & ( structKeyExists( arguments, "a" ) ? arguments.a : "none" );
    }
    function deleteThenConcatClassic( a ) {
        structDelete( arguments, "a" );
        a = "W"; a &= "V";
        return "vars.a=" & ( structKeyExists( variables, "a" ) ? variables.a : "none" ) & " args=" & structKeyExists( arguments, "a" );
    }
    function clearVars() { structDelete( variables, "a" ); structDelete( variables, "b" ); }
}
