<cfscript>
suiteBegin("QoQ: dotted (nested-struct) source table names");

// A Query-of-Queries source can be a query held at a NESTED STRUCT PATH, so the
// FROM table name may be dotted — e.g. `FROM variables.internal.iplog`. The QoQ
// parser must accept the dotted name and the VM must resolve it by walking the
// struct path. Masa CMS's front-end IP block-list check does exactly:
//   <cfquery dbtype="query"> SELECT blocked FROM variables.internal.iplog WHERE IP = ? </cfquery>
// Previously the parser errored "unexpected token after statement: Dot".

// Build a query nested under variables.internal.iplog
variables.internal = {};
variables.internal.iplog = queryNew("ip,blocked", "varchar,integer",
    [ ["10.0.0.1", 1], ["10.0.0.2", 0], ["10.0.0.9", 1] ]);

// 1. dbtype=query over a dotted (variables-scoped) source with a param
r1 = queryExecute(
      "SELECT blocked FROM variables.internal.iplog WHERE ip = :wanted"
    , { wanted = "10.0.0.1" }
    , { dbtype = "query" }
);
assert("dotted QoQ source resolves + filters (recordCount)", r1.recordCount, 1);
assert("dotted QoQ source returns the right value", r1.blocked, 1);

// 2. dotted source, no scope prefix (page-scoped nested struct)
holder = {};
holder.data = queryNew("id,name", "integer,varchar", [ [1,"a"], [2,"b"] ]);
r2 = queryExecute("SELECT name FROM holder.data WHERE id = 2", {}, { dbtype = "query" });
assert("dotted QoQ source without scope prefix resolves", r2.name, "b");

// 3. plain (non-dotted) source still works (no regression)
plainQ = queryNew("x", "integer", [ [7] ]);
r3 = queryExecute("SELECT x FROM plainQ", {}, { dbtype = "query" });
assert("plain (non-dotted) QoQ source still works", r3.x, 7);

suiteEnd();
</cfscript>
