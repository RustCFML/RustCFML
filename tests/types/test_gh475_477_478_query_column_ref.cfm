<cfscript>
suiteBegin("A q.col reference is the current row's value (GH 475, 477, 478)");

// GH 475 — member functions apply to the current row's value, as on Lucee.
q = queryNew( "name", "varchar", [ [ "_a.cfm" ], [ "b" ] ] );
assert("the bare reference is the first row", q.name, "_a.cfm");
assert("len() of the reference", q.name.len(), 6);
assert("reFind() of the reference", q.name.reFind( "^_" ), 1);
assert("ucase() of the reference", q.name.ucase(), "_A.CFM");
seen = "";
cfloop( query = q ) { seen = listAppend( seen, q.name.len() ); }
assert("inside a query loop the member call follows the cursor", seen, "6,1");
assert("the BIF form is unchanged", reFind( "^_", q.name ), 1);
assert("valueList still sees the whole column", valueList( q.name ), "_a.cfm,b");

// GH 477 — isEmpty() tests the current row's value, not the column.
e = queryNew( "id,cond", "varchar,varchar" );
queryAddRow( e );
querySetCell( e, "id", "a" );
assert("isEmpty of an empty cell", isEmpty( e.cond ), true);
assert("isEmpty through ?:", isEmpty( e.cond ?: "" ), true);
assert("isEmpty of a filled cell", isEmpty( e.id ), false);
assert("isEmpty of an indexed cell", isEmpty( e.cond[ 1 ] ), true);

// GH 478 — `q.col = v` sets the current row's cell; the variable stays a query.
w = queryNew( "id,title", "varchar,varchar", [ [ "1", "a" ] ] );
w.title = "x";
assert("the variable is still a query", isQuery( w ), true);
assert("the cell was set", w.title, "x");
assert("the other columns survive", w.columnList, "ID,TITLE");

// The same through a nested path, and on a query with more than one row.
multi = queryNew( "id,flag", "varchar,bit", [ [ "1", 0 ], [ "2", 0 ] ] );
holder = { detail = multi };
holder.detail.flag = true;
assert("a nested write keeps the query", isQuery( holder.detail ), true);
assert("it sets the CURRENT row", holder.detail.flag, true);
assert("it leaves the other rows alone", holder.detail.flag[ 2 ], 0);
assert("it leaves the other columns alone", holder.detail.id, "1");

// A write to a column that doesn't exist is an error, as on Lucee, rather than
// silently replacing the query with a one-key struct.
assertThrows("writing an unknown column throws", function() {
	var bad = queryNew( "id", "varchar", [ [ "1" ] ] );
	bad.nope = 1;
});

// The indexed write-back form still replaces the cell it names.
idx = queryNew( "a", "varchar", [ [ "1" ], [ "2" ] ] );
idx.a[ 2 ] = "z";
assert("q.col[row] = v still works", valueList( idx.a ), "1,z");

suiteEnd();
</cfscript>
