<!---
  A query column satisfies a declared SIMPLE type — docs/known-issues.md §35.

  `q.col` is a proxy that stands in for its current row's value: `isArray(q.col)`
  is false on Lucee 7 and here, and every scalar context (comparison, coercion,
  Len) already treats it as that one cell. The §29 type checker was the one place
  that treated it as a collection, so a `string`-declared function returning
  `q.col` was rejected as "Object type [Array]" — which is how Preside's
  SqlSchemaVersioning.getDbVersion (`return versionRecord.version_hash`) failed.

  Green on RustCFML and Lucee 7.
--->
<cfscript>
suiteBegin( "Query columns vs declared types (§35)" );

q = queryNew( "version_hash,n", "varchar,integer", [ [ "abc", 7 ] ] );

// The reported failure: a string-declared RETURN of a query column.
string function getHash() {
    return q.version_hash;
}
assert( "string return accepts a query column", getHash(), "abc" );

// A numeric column against a numeric declaration.
numeric function getN() {
    return q.n;
}
assert( "numeric return accepts a numeric column", getN(), 7 );

// Argument position takes the same path as return position.
string function takesString( required string s ) {
    return s;
}
assert( "string argument accepts a query column", takesString( q.version_hash ), "abc" );

numeric function takesNumeric( required numeric v ) {
    return v;
}
assert( "numeric argument accepts a numeric column", takesNumeric( q.n ), 7 );

// A column is NOT an array to either engine — the property that makes the above
// correct rather than merely convenient.
assertFalse( "isArray on a query column is false", isArray( q.version_hash ) );
assertTrue( "a query column is a simple value", isSimpleValue( q.version_hash ) );

// A genuinely wrong type must still be refused, so this stayed a real check.
assertThrows( "a struct still fails a string declaration", function() {
    return takesString( { a = 1 } );
} );

suiteEnd();
</cfscript>
