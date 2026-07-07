<!---
  GitHub #247: with returnType="struct" + keyColumn, the `result` option variable
  received the SAME struct as the returned data (keyed by keyColumn) instead of
  the standard query-metadata struct. A subsequent result.sql read then threw
  "Variable 'sql' is undefined". returnType only reshapes the RETURNED value; the
  `result` variable must ALWAYS get {recordCount, cached, columnList, sql,
  executionTime} and must never be the same object as the return.

  Root cause: the result-writeback treated ANY returned Struct as a mutation
  metadata struct, so the returnType=struct data payload was aliased into
  `result`. Fixed by discriminating on returnType — a shaped (array/struct)
  return now has its metadata synthesized (recordCount + columnList from the row
  keys). Also restores the previously-missing columnList on the array path.
--->
<cfscript>
suiteBegin("queryExecute returnType result metadata (GitHub 247)");

function probe() {
    var q = QueryNew("id,name", "integer,varchar");
    QueryAddRow(q, {id: 1, name: "a"});
    QueryAddRow(q, {id: 2, name: "b"});

    // 1. Control: default (query) returntype — metadata correct.
    var r0 = QueryExecute("SELECT * FROM q", [], {dbtype: "query", result: "local.meta0"});
    assert("default returntype result keys",
           listSort(structKeyList(local.meta0), "textnocase"),
           "cached,columnList,executionTime,recordCount,sql");

    // 2. returnType="array": metadata struct (not the array), now WITH columnList.
    var r1 = QueryExecute("SELECT * FROM q", [], {dbtype: "query", returnType: "array", result: "local.meta1"});
    assert("array return is an array", isArray(r1), true);
    assert("array result is a struct (metadata)", isStruct(local.meta1), true);
    assert("array result has recordCount", local.meta1.recordCount, 2);
    assert("array result columnList present", listSort(local.meta1.columnList, "textnocase"), "id,name");
    assert("array result sql present", len(trim(local.meta1.sql)) > 0, true);

    // 3. THE BUG: returnType="struct" + keyColumn — result must be METADATA,
    //    not the keyed data struct.
    var r2 = QueryExecute("SELECT * FROM q", [], {dbtype: "query", returnType: "struct", keyColumn: "id", result: "local.meta2"});
    assert("struct return is a struct", isStruct(r2), true);
    assert("struct result has recordCount (not data keys)", local.meta2.recordCount, 2);
    assert("struct result columnList present", listSort(local.meta2.columnList, "textnocase"), "id,name");
    assert("struct result sql present and non-empty", len(trim(local.meta2.sql)) > 0, true);
    assert("struct result does NOT carry data key '1'", structKeyExists(local.meta2, "1"), false);

    // 3b. result must NOT be the same object as the returned data.
    r2["1"]["name"] = "MUTATED";
    assert("struct result is a distinct object from the return",
           structKeyExists(local.meta2, "1"), false);

    // 3c. The exact read Wheels does after every query ($identitySelect).
    var caughtSql = "";
    try {
        caughtSql = Trim(local.meta2.sql);
    } catch (any e) {
        caughtSql = "THREW:" & e.message;
    }
    assert("reading result.sql after struct-mode does not throw", left(caughtSql, 6) != "THREW:", true);
}
probe();

suiteEnd();
</cfscript>
