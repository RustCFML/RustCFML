<cfscript>
suiteBegin( "Query mutators change the query in place and return Lucee's values (Lucee 7.1 verified)" );
// Every expectation here was read off Lucee 7.1 (2026-09-24). A query is a
// reference: a mutator changes it for every variable and struct holding it.
function _qmMk() { return queryNew( "a,b", "varchar,varchar", [ [ "1", "x" ], [ "2", "y" ], [ "3", "z" ] ] ); }
function _qmOther() { return queryNew( "a,b", "varchar,varchar", [ [ "9", "w" ] ] ); }
function _qmSt( q ) { return q.recordCount & ":" & valueList( q.a ) & "|" & lcase( q.columnList ); }
function _qmRet( v ) {
	if ( isQuery( v ) ) return "query";
	if ( isArray( v ) ) return "array:" & v.toList();
	if ( isBoolean( v ) && !isNumeric( v ) ) return "bool:" & v;
	return "simple:" & v;
}
function _qmCase( f ) {
	var q = _qmMk(); var alias = q; var held = { q = q };
	var r = f( q );
	return _qmRet( r ) & " " & _qmSt( q ) & " alias=" & _qmSt( alias ) & " held=" & _qmSt( held.q );
}

assert( "queryDeleteRow", _qmCase( function( q ){ return queryDeleteRow( q, 2 ); } ), "bool:true 2:1,3|a,b alias=2:1,3|a,b held=2:1,3|a,b" );
assert( "q.deleteRow()", _qmCase( function( q ){ return q.deleteRow( 2 ); } ), "query 2:1,3|a,b alias=2:1,3|a,b held=2:1,3|a,b" );
assert( "queryDeleteColumn returns the removed values", _qmCase( function( q ){ return queryDeleteColumn( q, "b" ); } ), "array:x,y,z 3:1,2,3|a alias=3:1,2,3|a held=3:1,2,3|a" );
assert( "q.deleteColumn()", _qmCase( function( q ){ return q.deleteColumn( "b" ); } ), "query 3:1,2,3|a alias=3:1,2,3|a held=3:1,2,3|a" );
assert( "queryAppend", _qmCase( function( q ){ return queryAppend( q, _qmOther() ); } ), "query 4:1,2,3,9|a,b alias=4:1,2,3,9|a,b held=4:1,2,3,9|a,b" );
assert( "q.append()", _qmCase( function( q ){ return q.append( _qmOther() ); } ), "query 4:1,2,3,9|a,b alias=4:1,2,3,9|a,b held=4:1,2,3,9|a,b" );
assert( "queryPrepend", _qmCase( function( q ){ return queryPrepend( q, _qmOther() ); } ), "query 4:9,1,2,3|a,b alias=4:9,1,2,3|a,b held=4:9,1,2,3|a,b" );
assert( "q.prepend()", _qmCase( function( q ){ return q.prepend( _qmOther() ); } ), "query 4:9,1,2,3|a,b alias=4:9,1,2,3|a,b held=4:9,1,2,3|a,b" );
assert( "queryInsertAt a query", _qmCase( function( q ){ return queryInsertAt( q, _qmOther(), 2 ); } ), "query 4:1,9,2,3|a,b alias=4:1,9,2,3|a,b held=4:1,9,2,3|a,b" );
assert( "q.insertAt()", _qmCase( function( q ){ return q.insertAt( _qmOther(), 2 ); } ), "query 4:1,9,2,3|a,b alias=4:1,9,2,3|a,b held=4:1,9,2,3|a,b" );
assert( "queryRowSwap", _qmCase( function( q ){ return queryRowSwap( q, 1, 3 ); } ), "query 3:3,2,1|a,b alias=3:3,2,1|a,b held=3:3,2,1|a,b" );
assert( "q.rowSwap()", _qmCase( function( q ){ return q.rowSwap( 1, 3 ); } ), "query 3:3,2,1|a,b alias=3:3,2,1|a,b held=3:3,2,1|a,b" );
assert( "querySetRow", _qmCase( function( q ){ return querySetRow( q, 2, { a = "S", b = "s" } ); } ), "bool:true 3:1,S,3|a,b alias=3:1,S,3|a,b held=3:1,S,3|a,b" );
assert( "q.setRow() returns true too", _qmCase( function( q ){ return q.setRow( 2, { a = "S", b = "s" } ); } ), "bool:true 3:1,S,3|a,b alias=3:1,S,3|a,b held=3:1,S,3|a,b" );
assert( "queryAddColumn returns the column number", _qmCase( function( q ){ return queryAddColumn( q, "c", "varchar", [ "p", "q", "r" ] ); } ), "simple:3 3:1,2,3|a,b,c alias=3:1,2,3|a,b,c held=3:1,2,3|a,b,c" );
assert( "q.addColumn()", _qmCase( function( q ){ return q.addColumn( "c", "varchar", [ "p", "q", "r" ] ); } ), "query 3:1,2,3|a,b,c alias=3:1,2,3|a,b,c held=3:1,2,3|a,b,c" );
assert( "q.setCell()", _qmCase( function( q ){ return q.setCell( "a", "C", 1 ); } ), "query 3:C,2,3|a,b alias=3:C,2,3|a,b held=3:C,2,3|a,b" );
assert( "queryClear", _qmCase( function( q ){ return queryClear( q ); } ), "query 0:|a,b alias=0:|a,b held=0:|a,b" );
assert( "q.clear()", _qmCase( function( q ){ return q.clear(); } ), "query 0:|a,b alias=0:|a,b held=0:|a,b" );
assert( "queryReverse returns a new query", _qmCase( function( q ){ return _qmSt( queryReverse( q ) ); } ), "simple:3:3,2,1|a,b 3:1,2,3|a,b alias=3:1,2,3|a,b held=3:1,2,3|a,b" );
assert( "q.reverse() leaves the query alone", _qmCase( function( q ){ return _qmSt( q.reverse() ); } ), "simple:3:3,2,1|a,b 3:1,2,3|a,b alias=3:1,2,3|a,b held=3:1,2,3|a,b" );
assert( "q.slice() returns a new query", _qmCase( function( q ){ return _qmSt( q.slice( 2, 2 ) ); } ), "simple:2:2,3|a,b 3:1,2,3|a,b alias=3:1,2,3|a,b held=3:1,2,3|a,b" );

// Direct calls on a page variable (the codegen write-back path).
q = _qmMk(); alias = q; queryDeleteRow( q, 2 );
assert( "direct queryDeleteRow reaches an alias", _qmSt( alias ), "2:1,3|a,b" );
q = _qmMk(); r = queryDeleteRow( q, 2 );
assert( "direct r = queryDeleteRow(): r is true, q still a query", _qmRet( r ) & " " & _qmSt( q ), "bool:true 2:1,3|a,b" );
q = _qmMk(); alias = q; queryDeleteColumn( q, "b" );
assert( "direct queryDeleteColumn reaches an alias", _qmSt( alias ), "3:1,2,3|a" );
q = _qmMk(); r = queryAddColumn( q, "c", "varchar", [ "p", "q", "r" ] );
assert( "direct r = queryAddColumn()", _qmRet( r ) & " " & _qmSt( q ), "simple:3 3:1,2,3|a,b,c" );
q = _qmMk(); q.setRow( 1, { a = "Z" } );
assert( "q.setRow() does not replace q with true", _qmSt( q ), "3:Z,2,3|a,b" );
s = { q = _qmMk() }; s.q.setRow( 1, { a = "Z" } );
assert( "s.q.setRow() does not replace s.q with true", _qmSt( s.q ), "3:Z,2,3|a,b" );

// Edge cases.
function _qm2() { return queryNew( "a,b", "varchar,varchar", [ [ "1", "x" ], [ "2", "y" ] ] ); }
function _qmSt2( q ) { return q.recordCount & ":" & valueList( q.a ) & "/" & valueList( q.b ); }
q = _qm2(); queryInsertAt( q, { a = "S", b = "s" }, 2 );
assert( "queryInsertAt a struct", _qmSt2( q ), "3:1,S,2/x,s,y" );
q = _qm2(); queryAppend( q, q );
assert( "queryAppend onto itself", _qmSt2( q ), "4:1,2,1,2/x,y,x,y" );
q = _qm2(); queryPrepend( q, q );
assert( "queryPrepend onto itself", _qmSt2( q ), "4:1,2,1,2/x,y,x,y" );
q = _qm2(); querySetRow( q, 1, { a = "Z" } );
assert( "querySetRow keeps the columns it is not given", _qmSt2( q ), "2:Z,2/x,y" );
assertThrows( "queryAppend with different columns", function(){ queryAppend( _qm2(), queryNew( "c", "varchar", [ [ "C" ] ] ) ); } );
assertThrows( "queryInsertAt with different columns", function(){ queryInsertAt( _qm2(), queryNew( "a", "varchar", [ [ "P" ] ] ), 2 ); } );
assertThrows( "queryInsertAt past recordCount + 1", function(){ queryInsertAt( _qm2(), { a = "E" }, 4 ); } );
assertThrows( "queryDeleteColumn of a missing column", function(){ queryDeleteColumn( _qm2(), "zz" ); } );
assertThrows( "queryDeleteRow out of range", function(){ queryDeleteRow( _qm2(), 5 ); } );
assertThrows( "queryRowSwap out of range", function(){ queryRowSwap( _qm2(), 1, 5 ); } );

// The for-in loop sees a delete made by its body (it iterates the query live).
q = _qmMk(); a = [];
for ( row in q ) { if ( q.currentRow == 1 ) queryDeleteRow( q, 3 ); a.append( row.a ); }
assert( "queryDeleteRow inside for ( row in q )", a.toList(), "1,2" );
suiteEnd();
</cfscript>
