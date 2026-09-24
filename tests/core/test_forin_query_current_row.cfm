<cfscript>
suiteBegin( "for ( row in query ) moves the query's current row (Lucee 7.1 verified)" );
// Every expectation here was read off Lucee 7.1 (2026-09-24). Preside's widget
// auto-discovery reads `views.name` inside `for ( var view in views )`; with the
// current row stuck at 1, only the first widget per directory registered.

function _fqList( q ) { var r = []; for ( var row in q ) { r.append( q.name ); } return r.toList(); }
function _fqFind( q, v ) { for ( var row in q ) { if ( q.name == v ) return q.currentRow; } return 0; }

q = queryNew( "name,type", "varchar,varchar", [ [ "one", "Dir" ], [ "two", "File" ], [ "three", "Dir" ] ] );
q2 = queryNew( "n", "varchar", [ [ "x" ], [ "y" ] ] );

a = []; for ( row in q ) { a.append( q.name & "@" & q.currentRow ); }
assert( "q.col and q.currentRow follow the loop", a.toList(), "one@1,two@2,three@3" );
assert( "current row restored after the loop", q.currentRow & "/" & q.name, "1/one" );

a = []; for ( row in q ) { a.append( q[ "name" ] & "=" & row.name & "=" & queryCurrentRow( q ) ); }
assert( "bracket read, row var and queryCurrentRow() agree", a.toList(), "one=one=1,two=two=2,three=three=3" );

assert( "var-scoped loop inside a function", _fqList( q ), "one,two,three" );
fn = function() { var r = []; for ( var row in q ) { r.append( q.type ); } return r.toList(); };
assert( "inside a closure", fn(), "Dir,File,Dir" );
s = { held = q }; a = []; for ( row in s.held ) { a.append( s.held.name ); }
assert( "query held in a struct", a.toList(), "one,two,three" );

a = []; for ( row in q ) { a.append( q.currentRow ); if ( q.currentRow == 2 ) break; }
assert( "break", a.toList() & " after=" & q.currentRow, "1,2 after=1" );
a = []; for ( row in q ) { if ( q.currentRow == 2 ) continue; a.append( q.name ); }
assert( "continue", a.toList(), "one,three" );

assert( "return from inside the loop sees the loop's row", _fqFind( q, "two" ), 2 );
assert( "current row restored after a return", q.currentRow, 1 );
inCatch = "";
try { for ( row in q ) { if ( q.currentRow == 2 ) throw( "boom" ); } } catch ( any e ) { inCatch = q.currentRow; }
assert( "current row already restored inside the catch", inCatch & "/" & q.currentRow, "1/1" );

a = []; for ( r1 in q ) { for ( r2 in q2 ) { a.append( q.name & "/" & q2.n ); } }
assert( "nested loops over two queries", a.toList(), "one/x,one/y,two/x,two/y,three/x,three/y" );
a = []; for ( r1 in q ) { outer = q.currentRow; for ( r2 in q ) { a.append( outer & ">" & q.currentRow ); } a.append( "after=" & q.currentRow ); }
assert( "nested loops over the same query restore the outer row", a.toList(), "1>1,1>2,1>3,after=1,2>1,2>2,2>3,after=2,3>1,3>2,3>3,after=3" );

a = []; cfloop( query = q ) { for ( row in q ) { a.append( q.currentRow ); } a.append( "|" & q.currentRow ); }
assert( "for-in inside cfloop query= hands the cfloop row back", a.toList(), "1,2,3,|1,1,2,3,|2,1,2,3,|3" );
a = []; cfloop( query = q ) { if ( q.currentRow == 2 ) { try { for ( row in q ) { if ( q.currentRow == 3 ) throw( "x" ); } } catch ( any e ) { a.append( "caught@" & q.currentRow ); } } a.append( q.currentRow ); }
assert( "a throw out of an inner loop restores the cfloop row", a.toList(), "1,caught@2,2,3" );

qq = queryExecute( "select name from q where type = 'Dir'", {}, { dbtype = "query" } );
a = []; for ( row in qq ) { a.append( qq.name ); }
assert( "query of queries result", a.toList(), "one,three" );

e = queryNew( "name" ); z = 0; for ( row in e ) { z++; }
assert( "empty query", z & "/" & e.currentRow, "0/1" );

m = queryNew( "name", "varchar", [ [ "one" ], [ "two" ], [ "three" ] ] );
a = []; for ( row in m ) { if ( m.currentRow == 1 ) querySetCell( m, "name", "CHANGED", 3 ); a.append( row.name ); }
assert( "rows are read live: a later cell changed by the body", a.toList(), "one,two,CHANGED" );
m = queryNew( "name", "varchar", [ [ "one" ], [ "two" ] ] );
a = []; for ( row in m ) { if ( m.currentRow == 1 ) { queryAddRow( m ); querySetCell( m, "name", "added" ); } a.append( row.name ); }
assert( "rows are read live: a row added by the body is iterated", a.toList() & " rc=" & m.recordCount, "one,two,added rc=3" );
m = queryNew( "name", "varchar", [ [ "one" ], [ "two" ], [ "three" ] ] );
a = []; for ( row in m ) { row.name = "mut"; a.append( m.name ); }
assert( "the row variable is a copy", a.toList(), "one,two,three" );

dir = getTempDirectory() & "forin_query_" & createUUID();
directoryCreate( dir ); directoryCreate( dir & "/a" ); directoryCreate( dir & "/b" );
dl = directoryList( dir, false, "query" );
a = []; for ( row in dl ) { a.append( dl.name ); }
directoryDelete( dir, true );
a.sort( "text" );
assert( "directoryList query (the Preside widget-discovery shape)", a.toList(), "a,b" );
suiteEnd();
</cfscript>
