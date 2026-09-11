<cfscript>
suiteBegin( "Closures are lexically bound across components; plain UDF values re-bind to the receiver (Lucee 7.1 verified)" );
// Every expectation here was read off Lucee 7.1 (2026-09-11) before the fix.
d = new oop.clobind.Def(); t = new oop.clobind.Tgt();
t.inject( "clo", d.mk() ); t.inject( "arr", d.mkArrow() ); t.inject( "wr", d.mkWriter() ); t.inject( "nest", d.mkNested() ); t.inject( "udf", d.mkUdfRef() );
lex = "vars=def this=def-this unscoped=def loc=loc";
assert( "closure called where defined", d.mk()( 1 ), lex & " cnt=1 x=1" );
assert( "injected closure, direct member call: defining scopes", t.clo( 2 ), lex & " cnt=2 x=2" );
assert( "injected closure via this.x() inside a method", t.callThis( 3 ), lex & " cnt=3 x=3" );
assert( "injected closure via bare call inside a method", t.callBare( 4 ), lex & " cnt=4 x=4" );
assert( "injected closure via variables.x()", t.callVars( 5 ), lex & " cnt=5 x=5" );
assert( "injected closure held in a plain struct, s.f()", t.callStructHeld( 6 ), lex & " cnt=6 x=6" );
assert( "closure writes hit the DEFINING component's variables", d.getCounter() & "/" & t.getCounter(), "6/100" );
assert( "arrow function, direct", t.arr(), "vars=def this=def-this unscoped=def" );
assert( "arrow function via bare call", t.callArrowBare(), "vars=def this=def-this unscoped=def" );
t.wr();
assert( "variables.x = and unscoped writes land on the definer", d.getWritten() & "/" & t.getWritten() & "/" & d.getUnscoped() & "/" & t.getUnscoped(), "by-closure/none/unscoped-by-closure/none" );
assert( "nested closure keeps the outer closure's binding", t.callNested(), "vars=def this=def-this" );
assert( "a PLAIN UDF reference re-binds to the receiver", t.udf(), "vars=tgt this=tgt-this" );
assert( "a PLAIN UDF reference via bare call re-binds too", t.callUdfBare(), "vars=tgt this=tgt-this" );
// Page-level closure stored on a component: page variables, and NO `this`.
pageWho = "page";
pc = function() { return "vars=" & variables.pageWho & " hasThis=" & isDefined( "this" ); };
t.inject( "pc", pc );
assert( "page closure invoked as cfc.f(): page scope, no this", t.pc(), "vars=page hasThis=false" );
t.inject( "clo", pc );
assert( "page closure via bare call inside a method: no this", t.callBare( 1 ), "vars=page hasThis=false" );
// Visibility: a closure sees only its own lexical scope, never the caller's.
d2 = new oop.clobind.Def2(); t2 = new oop.clobind.Tgt2(); t2.inject( "probe", d2.mkProbe() );
assert( "caller locals/args invisible: bare", t2.callBare( 9 ), "callerLocal=false callerArg=false defVar=def2" );
assert( "caller locals/args invisible: this.x()", t2.callThis( 9 ), "callerLocal=false callerArg=false defVar=def2" );
assert( "caller locals/args invisible: struct-held", t2.callStruct( 9 ), "callerLocal=false callerArg=false defVar=def2" );
assert( "same-frame closure sees a var declared after it", d2.later(), "sees:5" );
assert( "same-frame closure does NOT see the outer arguments scope", d2.seesArgs( 3 ), "noOuterArg" );
assert( "same-frame closure sees the outer argument by bare name", d2.seesArgsBare( 4 ), "bare:4" );
assert( "same-frame closure sees a later mutation", d2.mutateLater(), 2 );
assert( "closure write to an outer local is visible after", d2.closureWritesLocal(), 7 );
suiteEnd();
</cfscript>
