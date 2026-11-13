<!---
  A bodyless <cftransaction action="commit|rollback" /> inside a <cftransaction>
  block, as Adobe ColdFusion 2025 accepts it. RustCFML counts the nested
  tag as opening a level, so the outer block has no matching end tag and the
  file fails to compile. A <cftransaction> written inside a CFML comment is
  counted the same way, and a self-closing <cftransaction action="begin" />
  is reported as missing its end tag. Every form here runs on Adobe ColdFusion
  2025.
--->
<cfscript>
suiteBegin("cftransaction: bodyless nested commit / rollback");

tmpfile = getTempDirectory() & "/rustcfml_txn_nested_" & createUUID() & ".sqlite";
ds = "sqlite://" & tmpfile;
skip = false;

try {
    queryExecute("CREATE TABLE t (id INTEGER PRIMARY KEY, name TEXT)", [], { datasource: ds });
} catch (any e) {
    skip = true;
    assertTrue("cftransaction nested skipped (no sqlite): " & e.message, true);
}
</cfscript>

<cfif NOT skip>

<!--- Commit part-way through, then keep going in the same block --->
<cftransaction>
    <cfquery datasource="#ds#">INSERT INTO t (name) VALUES ('first')</cfquery>
    <cftransaction action="commit" />
    <!--- <cftransaction> in a comment opens nothing --->
    <cfquery datasource="#ds#">INSERT INTO t (name) VALUES ('second')</cfquery>
</cftransaction>

<!--- Roll back on a condition --->
<cftransaction>
    <cfquery datasource="#ds#">INSERT INTO t (name) VALUES ('discarded')</cfquery>
    <cfif true>
        <cftransaction action="rollback"/>
    </cfif>
</cftransaction>

<!--- The slash-less form of a nested rollback --->
<cftransaction>
    <cfquery datasource="#ds#">INSERT INTO t (name) VALUES ('also discarded')</cfquery>
    <cftransaction action="rollback">
</cftransaction>

<!---
  Lucee's rule (transformer.cfml.attributes.impl.Transaction): a cftransaction
  inside another one is bodyless whenever it has an action attribute, whatever
  its value. Each form below runs on Lucee 7.1.
--->
<!--- A nested action="begin" is an empty block: the next end tag closes the outer one --->
<cftransaction>
    <cfquery datasource="#ds#">INSERT INTO t (name) VALUES ('third')</cfquery>
    <cftransaction action="begin">
</cftransaction>

<!--- An action known only at run time --->
<cfset nestedAction = "rollback" />
<cftransaction>
    <cfquery datasource="#ds#">INSERT INTO t (name) VALUES ('dynamic discarded')</cfquery>
    <cftransaction action="#nestedAction#">
</cftransaction>

<!--- An unquoted action --->
<cftransaction>
    <cfquery datasource="#ds#">INSERT INTO t (name) VALUES ('fourth')</cfquery>
    <cftransaction action=commit>
</cftransaction>

<!--- A self-closing begin is an empty block, with no end tag --->
<cftransaction action="begin" />

<cfset afterBlocks = true />

<cfquery name="q" datasource="#ds#">SELECT name FROM t ORDER BY id</cfquery>

<cfscript>
assert("code after the blocks runs", afterBlocks, true);
assert("committed rows kept", valueList(q.name), "first,second,third,fourth");
try { fileDelete(tmpfile); } catch (any e) {}
</cfscript>

</cfif>

<cfscript>suiteEnd();</cfscript>
