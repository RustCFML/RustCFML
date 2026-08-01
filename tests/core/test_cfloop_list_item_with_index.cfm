<!--- <cfloop list> with BOTH item and index must populate both, Lucee-style:
      item = the list element, index = the 1-based position. RustCFML currently
      treats the combination like the legacy index-only form — the element
      lands in `index` and `item` is never set, so any read of the item
      variable throws "Variable '...' is undefined".

      The two single-attribute forms already match Lucee (item-only → element;
      legacy index-only → element) and are pinned here as controls.

      Real-world repro: moopa's hub schema-sync FK comparison —
        <cfloop list="#structKeyList(code_fk)#" item="code_fk_param" index="i">
      (schemaSync.cfc compareDatabaseSchema). On RustCFML the sysadmin schema
      page 500s with "Variable 'code_fk_param' is undefined" as soon as an
      existing foreign key is compared. (Unreachable before v0.543.0 — the
      cfquery tag columnkey fix (GH #294) made the FK metadata readable, which
      let this loop execute for the first time.)

      Lucee 6.2 direct check (lucee/lucee:6.2 Docker):
        item-only=a,b,c  index-only=a,b,c  both: item=a,b,c index=1,2,3 --->

<cfset itemsOnly = "">
<cfloop list="a,b,c" item="el1">
    <cfset itemsOnly = listAppend(itemsOnly, isNull(el1) ? "UNDEF" : el1)>
</cfloop>

<cfset legacyIndex = "">
<cfloop list="a,b,c" index="ix1">
    <cfset legacyIndex = listAppend(legacyIndex, isNull(ix1) ? "UNDEF" : ix1)>
</cfloop>

<cfset bothItems = "">
<cfset bothIndexes = "">
<cfloop list="a,b,c" item="el2" index="ix2">
    <cfset bothItems = listAppend(bothItems, isNull(el2) ? "UNDEF" : el2)>
    <cfset bothIndexes = listAppend(bothIndexes, isNull(ix2) ? "UNDEF" : ix2)>
</cfloop>

<cfscript>
suiteBegin("cfloop list with both item and index");

assert("item-only: item receives each element", itemsOnly, "a,b,c");
assert("index-only (legacy): index receives each element", legacyIndex, "a,b,c");
assert("item+index: item receives each element", bothItems, "a,b,c");
assert("item+index: index receives the 1-based position", bothIndexes, "1,2,3");

suiteEnd();
</cfscript>
