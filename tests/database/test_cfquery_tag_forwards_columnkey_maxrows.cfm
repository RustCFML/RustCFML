<cfscript>
// Source: RED test contributed by Matthew (@Blute) in GH PR #294; the
// keyColumn-alias and control-flow-body cases were added with the fix.
//
// The <cfquery> TAG must behave identically to queryExecute() for the same
// attributes. queryExecute honours returntype="struct" + columnKey (row-keyed
// struct, Lucee behavior — supported since v0.13.0) and maxrows — but the tag
// lowering forwards only a whitelist of attributes into the options struct
// (tag_parser.rs "cfquery" arm: datasource/name/result/returntype/dbtype/
// attributeCollection), so columnkey and maxrows are silently dropped.
//
// With columnkey dropped, returntype="struct" falls into the "single-row map"
// branch and returns a struct keyed by COLUMN NAMES instead of row key values.
//
// Real-world repro: moopa's hub schema-sync reads index/FK metadata with
// <cfquery returntype="struct" columnkey="name"> (schemaSync.cfc
// getTableIndexes/getTableForeignKeys). On RustCFML the "existing indexes"
// struct comes back keyed by the query's column names, so the schema diff
// offers to DROP CONSTRAINT name/table_name/onupdate/... on every table and
// re-CREATE every index and FK that already exists.
//
// Verified identical on v0.125.0, v0.126.0, v0.531.0 and v0.541.0 release
// binaries — the tag path has never forwarded these attributes.

suiteBegin("cfquery tag forwards columnkey and maxrows like queryExecute");

memDs = { class: "org.sqlite.JDBC", connectionString: "jdbc:sqlite::memory:" };
twoRowSql = "SELECT 'alpha' AS name, '1' AS v UNION ALL SELECT 'beta', '2'";

// ---- Control: queryExecute already honours both options ----
stQe = queryExecute(twoRowSql, [], { datasource: memDs, returntype: "struct", columnKey: "name" });
assert("queryExecute: struct is keyed by the columnKey column's row values", listSort(structKeyList(stQe), "text"), "alpha,beta");
assert("queryExecute: row struct carries the other columns", stQe["alpha"].v, "1");

qQe = queryExecute(twoRowSql, [], { datasource: memDs, maxrows: 1 });
assert("queryExecute: maxrows caps the resultset", qQe.recordCount, 1);
</cfscript>

<!--- ---- The same attributes through the <cfquery> TAG ---- --->
<cfquery name="stTag" datasource="#memDs#" returntype="struct" columnkey="name">
    SELECT 'alpha' AS name, '1' AS v UNION ALL SELECT 'beta', '2'
</cfquery>
<cfscript>
assert("cfquery tag: struct is keyed by the columnkey column's row values", listSort(structKeyList(stTag), "text"), "alpha,beta");
hasAlpha = structKeyExists(stTag, "alpha");
assertTrue("cfquery tag: row key resolves to its row struct", hasAlpha AND (stTag["alpha"].v ?: "") EQ "1");
</cfscript>

<cfquery name="qTag" datasource="#memDs#" maxrows="1">
    SELECT 'alpha' AS name UNION ALL SELECT 'beta'
</cfquery>
<cfscript>
assert("cfquery tag: maxrows caps the resultset", qTag.recordCount, 1);
</cfscript>

<!--- The tag forwards EVERY attribute, not a whitelist: the `keyColumn`
      spelling Lucee also accepts, and a dynamic value. --->
<cfset keyCol = "name">
<cfquery name="stAlias" datasource="#memDs#" returntype="struct" keycolumn="#keyCol#">
    SELECT 'alpha' AS name, '1' AS v UNION ALL SELECT 'beta', '2'
</cfquery>
<cfscript>
assert("cfquery tag: keyColumn alias, dynamic value", listSort(structKeyList(stAlias), "text"), "alpha,beta");
</cfscript>

<!--- Control flow in the body switches the lowering to the runtime
      savecontent branch — same options struct, so the attributes must still
      arrive. --->
<cfset wantAll = true>
<cfquery name="stFlow" datasource="#memDs#" returntype="struct" columnkey="name">
    SELECT 'alpha' AS name, '1' AS v
    <cfif wantAll>UNION ALL SELECT 'beta', '2'</cfif>
</cfquery>
<cfscript>
assert("cfquery tag: columnkey survives the control-flow body path", listSort(structKeyList(stFlow), "text"), "alpha,beta");

suiteEnd();
</cfscript>
