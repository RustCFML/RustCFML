<cfscript>
suiteBegin("cfquery SQL line comments");

src = queryNew("id,name", "integer,varchar", [[1, "alpha"], [2, "beta"]]);
targetId = 2;
</cfscript>

<cfquery name="controlRows" dbtype="query">
    SELECT name
    FROM src
    WHERE id = 2
</cfquery>

<cfquery name="lineCommentRows" dbtype="query">
    SELECT name
    -- this SQL comment must end at the newline, not consume FROM/WHERE
    FROM src
    WHERE id = 2
</cfquery>

<cfquery name="interpolatedLineCommentRows" dbtype="query">
    SELECT name
    -- this SQL comment must also end before interpolated SQL text
    FROM src
    WHERE id = #targetId#
</cfquery>

<cfscript>
assert("control cfquery body without SQL comment", controlRows.recordCount, 1);
assert("cfquery body preserves newline after SQL line comment", lineCommentRows.recordCount, 1);
assert("cfquery body returns row after SQL line comment", lineCommentRows.name, "beta");
assert("interpolated cfquery body preserves newline after SQL line comment", interpolatedLineCommentRows.recordCount, 1);
assert("interpolated cfquery body returns row after SQL line comment", interpolatedLineCommentRows.name, "beta");

suiteEnd();
</cfscript>
