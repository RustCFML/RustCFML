<!---
  An argument declared type="integer" accepts whole numbers, as in Adobe
  ColdFusion (#524).

  RustCFML mirrors Lucee 7, which has no cast target for "integer" and so
  rejects every value (test_fn_type_enforcement.cfm asserts that). Adobe
  ColdFusion 2025 accepts a whole number and rejects anything else, so code
  written for Adobe cannot call these functions at all. Every assertion below
  was checked on Adobe ColdFusion 2025. It covers arguments only: Adobe rejects
  every value for returntype="integer" too.
--->
<cffunction name="tagInt" returntype="string" output="false">
    <cfargument name="n" type="integer" required="true">
    <cfreturn "ok">
</cffunction>
<cfscript>
suiteBegin( "type=""integer"" arguments accept whole numbers (Adobe ColdFusion)" );

function scriptInt( integer n ) { return "ok"; }

function rejects( cb ) {
    try {
        cb();
        return false;
    } catch ( any e ) {
        return true;
    }
}

assert( "tag argument: integer", tagInt( 5 ), "ok" );
assert( "tag argument: negative integer", tagInt( -3 ), "ok" );
assert( "tag argument: integral string", tagInt( "5" ), "ok" );
assert( "tag argument: whole-number float", tagInt( 5.0 ), "ok" );
assert( "script argument: integer", scriptInt( 5 ), "ok" );

// Anything that is not a whole number is still rejected
assertTrue( "rejects a fraction", rejects( function(){ tagInt( 1.5 ); } ) );
assertTrue( "rejects a decimal string", rejects( function(){ tagInt( "5.0" ); } ) );
assertTrue( "rejects letters", rejects( function(){ tagInt( "abc" ); } ) );
assertTrue( "rejects an empty string", rejects( function(){ tagInt( "" ); } ) );
assertTrue( "rejects a boolean", rejects( function(){ tagInt( true ); } ) );
assertTrue( "script argument rejects a fraction", rejects( function(){ scriptInt( 1.5 ); } ) );

suiteEnd();
</cfscript>
