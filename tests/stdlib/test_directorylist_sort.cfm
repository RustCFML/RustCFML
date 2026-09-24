<cfscript>
suiteBegin( "directoryList / cfdirectory sort (Lucee 7.1 verified)" );
// Every expectation here was read off Lucee 7.1 (2026-09-24). Lucee sorts TEXT
// case-sensitively, `size` numerically; a bad spec sorts nothing; and the
// directoryList `name`/`path` arrays are never sorted.
_dlRoot = getTempDirectory() & "dlsort_" & createUUID();
directoryCreate( _dlRoot );
directoryCreate( _dlRoot & "/beta" ); directoryCreate( _dlRoot & "/Alpha" ); directoryCreate( _dlRoot & "/beta/inner" );
fileWrite( _dlRoot & "/c.txt", "12345" ); fileWrite( _dlRoot & "/B.txt", "1" ); fileWrite( _dlRoot & "/a.txt", "123" );
fileWrite( _dlRoot & "/beta/z.txt", "12" ); fileWrite( _dlRoot & "/beta/inner/y.txt", "1234" );
function _dlNames( q ) { var r = []; for ( var row in q ) r.append( row.name ); return r.toList(); }
function _dlQ( sort, filter = "" ) { return _dlNames( directoryList( _dlRoot, false, "query", filter, sort ) ); }

assert( "name (case-sensitive)", _dlQ( "name" ), "Alpha,B.txt,a.txt,beta,c.txt" );
assert( "name asc", _dlQ( "name asc" ), "Alpha,B.txt,a.txt,beta,c.txt" );
assert( "name desc", _dlQ( "name desc" ), "c.txt,beta,a.txt,B.txt,Alpha" );
assert( "column and direction are case-insensitive", _dlQ( "NAME DESC" ), "c.txt,beta,a.txt,B.txt,Alpha" );
assert( "size is numeric", _dlQ( "size", "*.txt" ), "B.txt,a.txt,c.txt" );
assert( "size desc", _dlQ( "size desc", "*.txt" ), "c.txt,a.txt,B.txt" );
assert( "type, name: Dir before File", _dlQ( "type, name" ), "Alpha,beta,B.txt,a.txt,c.txt" );
assert( "type desc, name desc", _dlQ( "type desc, name desc" ), "c.txt,a.txt,B.txt,beta,Alpha" );
q = directoryList( _dlRoot, true, "query", "", "directory, name" ); a = [];
for ( row in q ) a.append( replace( replace( row.directory, _dlRoot, "" ), "\", "/", "all" ) & "/" & row.name );
assert( "recursive directory, name", a.toList(), "/Alpha,/B.txt,/a.txt,/beta,/c.txt,/beta/inner,/beta/z.txt,/beta/inner/y.txt" );
assert( "named sort argument", _dlNames( directoryList( path = _dlRoot, listInfo = "query", sort = "name desc" ) ), "c.txt,beta,a.txt,B.txt,Alpha" );

// A bad spec sorts nothing: the listing equals the unsorted one.
unsorted = _dlQ( "" );
assert( "unknown column leaves filesystem order", _dlQ( "bogus" ), unsorted );
assert( "unknown direction leaves filesystem order", _dlQ( "name sideways" ), unsorted );
// The name/path arrays ignore sort.
assert( "listInfo=name ignores sort", directoryList( _dlRoot, false, "name", "", "name desc" ).toList(), directoryList( _dlRoot, false, "name" ).toList() );

q = directoryList( _dlRoot, false, "query", "a.txt" );
assertTrue( "dateLastModified is a date", isDate( q.dateLastModified ) );
if ( !findNoCase( "windows", server.os.name ) ) {
	assertTrue( "mode is the octal permission bits", reFind( "^[0-7]{3}$", q.mode ) > 0 );
}
q = directoryList( _dlRoot, false, "query", "beta" );
assert( "a directory's size is 0", q.size, 0 );

cfdirectory( action = "list", directory = _dlRoot, name = "d", sort = "name" );
assert( "cfdirectory name", _dlNames( d ), "Alpha,B.txt,a.txt,beta,c.txt" );
cfdirectory( action = "list", directory = _dlRoot, name = "d", sort = "name desc" );
assert( "cfdirectory name desc", _dlNames( d ), "c.txt,beta,a.txt,B.txt,Alpha" );
cfdirectory( action = "list", directory = _dlRoot, name = "d", sort = "size desc" );
assert( "cfdirectory size desc", _dlNames( d ), "c.txt,a.txt,B.txt,beta,Alpha" );
cfdirectory( action = "list", directory = _dlRoot, name = "d", sort = "type, name" );
assert( "cfdirectory type, name", _dlNames( d ), "Alpha,beta,B.txt,a.txt,c.txt" );
cfdirectory( action = "list", directory = _dlRoot, name = "d", listInfo = "name", sort = "name desc" );
assert( "cfdirectory listInfo=name is sorted", _dlNames( d ), "c.txt,beta,a.txt,B.txt,Alpha" );
cfdirectory( action = "list", directory = _dlRoot, name = "d" );
unsortedTag = _dlNames( d );
cfdirectory( action = "list", directory = _dlRoot, name = "d", sort = "bogus" );
assert( "cfdirectory unknown column leaves filesystem order", _dlNames( d ), unsortedTag );

directoryDelete( _dlRoot, true );
suiteEnd();
</cfscript>
