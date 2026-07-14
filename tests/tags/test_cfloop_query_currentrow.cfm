<cfscript>
suiteBegin("cfloop query pseudo-columns (currentRow / recordCount / columnList)");

// Shared fixture query.
q = queryNew("a,b", "varchar,varchar", [["x","1"],["y","2"],["z","3"]]);
</cfscript>

<!--- TAG FORM: <cfloop query="q"> --- Masa dbUtility.tables() reads
      `rsCheck.currentRow` inside exactly this loop. --->
<cfset rows = "">
<cfset rcs = "">
<cfset cl = "">
<cfloop query="q">
  <cfset rows = rows & q.currentRow & ":" & q.a & " ">
  <cfset rcs = rcs & q.recordCount & " ">
  <cfset cl = q.columnList>
</cfloop>

<cfscript>
assert("tag: currentRow increments 1-based with row cols", trim(rows), "1:x 2:y 3:z");
assert("tag: recordCount is total rows on every iteration", trim(rcs), "3 3 3");
assert("tag: columnList reflects the query columns", cl, "a,b");
// The query variable is restored to the query (not the last row) after the loop.
assert("tag: query var restored after loop", q.recordCount, 3);

// SCRIPT FORM: loop query=q2 { ... }
q2 = queryNew("a,b", "varchar,varchar", [["x","1"],["y","2"],["z","3"]]);
srows = "";
loop query=q2 {
    srows &= q2.currentRow & ":" & q2.a & " ";
}
assert("script: currentRow increments 1-based with row cols", trim(srows), "1:x 2:y 3:z");

// recordCount/columnList in script form.
q3 = queryNew("a,b", "varchar,varchar", [["x","1"],["y","2"]]);
srcs = "";
scl = "";
loop query=q3 {
    srcs &= q3.recordCount & " ";
    scl = q3.columnList;
}
assert("script: recordCount total on every iteration", trim(srcs), "2 2");
assert("script: columnList value", scl, "a,b");

// Nested query loops must keep independent currentRow counters (temp-name
// collision regression — the fix keys temps on tag position, not tag length).
outer = queryNew("v", "varchar", [["A"],["B"]]);
inner = queryNew("w", "varchar", [["p"],["q"],["r"]]);
pairs = "";
</cfscript>
<cfloop query="outer">
  <cfset ov = outer.currentRow>
  <cfloop query="inner">
    <cfset pairs = pairs & ov & "-" & inner.currentRow & " ">
  </cfloop>
</cfloop>
<cfscript>
assert("nested: independent currentRow counters",
       trim(pairs), "1-1 1-2 1-3 2-1 2-2 2-3");

// currentRow must be a number (usable in arithmetic / queryRowToStruct-style
// row indexing), not a string.
q4 = queryNew("a", "varchar", [["only"]]);
</cfscript>
<cfloop query="q4">
  <cfset numeric_ok = isNumeric(q4.currentRow) and (q4.currentRow + 1 eq 2)>
</cfloop>
<cfscript>
assertTrue("currentRow is numeric and usable in arithmetic", numeric_ok);

suiteEnd();
</cfscript>
