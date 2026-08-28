<cfscript>
// `returntype="void"` is not "nothing came back".
//
// Lucee's Decision.isVoid ALSO accepts an empty string, boolean false, and any
// number that truncates to zero — and, because UDF return values are VALIDATED
// rather than cast (UDFImpl checks isCastableTo and returns the original), the
// value comes back unchanged rather than as null.
//
// This matters in the wild: `return false;` inside a void function reads as
// "stop here", and a shipped Preside extension's interceptor does exactly that.
// Rejecting it stopped the whole application booting.
//
// Note the asymmetry the rule creates: 0 passes, "0" does not — a String is
// only tested for emptiness.

suiteBegin( "void return type (Lucee's Decision.isVoid)" );

void function voidFalse()     { return false; }
void function voidTrue()      { return true; }
void function voidEmpty()     { return ""; }
void function voidZero()      { return 0; }
void function voidHalf()      { return 0.5; }
void function voidNegHalf()   { return -0.5; }
void function voidOne()       { return 1; }
void function voidZeroString(){ return "0"; }
void function voidSpace()     { return " "; }
void function voidArray()     { return []; }
void function voidStruct()    { return {}; }
void function voidNothing()   { return; }

// ── accepted, and handed back UNCHANGED ─────────────────────────────────
assertFalse( "void accepts false, and returns it unchanged", voidFalse() );
assert( "void accepts an empty string" , voidEmpty()  , ""   );
assert( "void accepts 0"               , voidZero()   , 0    );
// intValue() truncates toward zero, so anything under 1 in magnitude is "zero".
assert( "void accepts 0.5"             , voidHalf()   , 0.5  );
assert( "void accepts -0.5"            , voidNegHalf(), -0.5 );
assertTrue( "a bare return is still null", IsNull( voidNothing() ) );

// ── rejected ────────────────────────────────────────────────────────────
assertThrows( "void rejects true"      , function(){ voidTrue();       } );
assertThrows( "void rejects 1"         , function(){ voidOne();        } );
assertThrows( "void rejects the STRING '0'", function(){ voidZeroString(); } );
assertThrows( "void rejects a space"   , function(){ voidSpace();      } );
assertThrows( "void rejects an array"  , function(){ voidArray();      } );
assertThrows( "void rejects a struct"  , function(){ voidStruct();     } );

// The two message forms Lucee uses, discriminated by the VALUE: a String gets
// the bare cast message, everything else gets it wrapped.
voidMessage = "";
try { voidTrue(); } catch ( any e ) { voidMessage = e.message; }
assertTrue( "a non-string violation names the function", FindNoCase( "has an invalid return value", voidMessage ) > 0 );
voidMessage = "";
try { voidZeroString(); } catch ( any e ) { voidMessage = e.message; }
assert( "a string violation gets the bare cast message", voidMessage, "Cannot cast String [0] to a value of type [void]" );

suiteEnd();
</cfscript>
