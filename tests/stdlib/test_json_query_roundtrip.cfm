<cfscript>
suiteBegin("stdlib: query <-> JSON round trip (GH ##232)");

q = queryNew( "id,name", "integer,varchar", [ [ 1, "a" ], [ 2, "b" ] ] );

// serializeJSON emits the Lucee/ACF {COLUMNS, DATA} envelope, NOT an array of
// row structs. Row-oriented DATA by default; column-oriented with ROWCOUNT when
// serializeQueryByColumns=true. Verified byte-for-byte against Lucee 6.
assert("row-oriented (default)",
    serializeJSON( q ),
    '{"COLUMNS":["id","name"],"DATA":[[1,"a"],[2,"b"]]}');
assert("row-oriented (explicit false)",
    serializeJSON( q, false ),
    '{"COLUMNS":["id","name"],"DATA":[[1,"a"],[2,"b"]]}');
assert("column-oriented (true) -> ROWCOUNT + uppercased DATA keys",
    serializeJSON( q, true ),
    '{"ROWCOUNT":2,"COLUMNS":["id","name"],"DATA":{"ID":[1,2],"NAME":["a","b"]}}');

// deserializeJSON reconstructs a NATIVE query only with strictMapping=false.
q2 = deserializeJSON( serializeJSON( q, false ), false );
assertTrue("row-form round trip is a native query", isQuery( q2 ));
assert("reconstructed recordCount", q2.recordCount, 2);
assert("reconstructed cell id[1]", q2.id[1], 1);
assert("reconstructed cell name[2]", q2.name[2], "b");
// queryGetRow works on the reconstructed query (the reported failure site)
row = queryGetRow( q2, 1 );
assert("queryGetRow.id", row.id, 1);
assert("queryGetRow.name", row.name, "a");

// column-oriented form also reconstructs
qc = deserializeJSON( serializeJSON( q, true ), false );
assertTrue("column-form round trip is a native query", isQuery( qc ));
assert("column-form recordCount", qc.recordCount, 2);
assert("column-form cell name[1]", qc.name[1], "a");

// strictMapping=true (the default) keeps it a struct, matching Lucee
assertFalse("strictMapping default keeps a struct", isQuery( deserializeJSON( serializeJSON( q ) ) ));
assertFalse("strictMapping=true keeps a struct", isQuery( deserializeJSON( serializeJSON( q ), true ) ));

suiteEnd();
</cfscript>
