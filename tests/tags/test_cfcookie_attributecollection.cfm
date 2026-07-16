<cfscript>
suiteBegin("Tags: cfcookie attributeCollection + same-request readback");
</cfscript>

<!---
============================================================
Background
============================================================
<cfcookie> sets a Set-Cookie response header AND immediately populates the
in-request `cookie` scope, so a cookie set earlier in a request is readable
later in the SAME request. Masa CMS's utility.setCookie() calls
`<cfcookie attributeCollection="#arguments#" />`, and admin/Application.cfc then
reads `cookie.ADMINSIDEBAR` in the template layout of the same request.

RustCFML's __cfcookie handler read `name`/`value` directly off the options
struct, so the attributeCollection form (which nests them inside an
`attributecollection` key) set an empty-named cookie and never populated the
`cookie` scope — the template threw "Variable 'ADMINSIDEBAR' is undefined".
============================================================
--->

<!--- direct form (control) --->
<cfcookie name="DIRECTCK" value="dval">
<cfscript>
assert("direct cfcookie is readable in-request", structKeyExists(cookie, "DIRECTCK"), true);
assert("direct cfcookie value", cookie.DIRECTCK, "dval");
</cfscript>

<!--- attributeCollection form (the Masa path) --->
<cfset ckArgs = { name="ADMINSIDEBAR", value="off", httpOnly=false, maintainCase=true }>
<cfcookie attributeCollection="#ckArgs#" />
<cfscript>
assert("attributeCollection cfcookie populates cookie scope", structKeyExists(cookie, "ADMINSIDEBAR"), true);
assert("attributeCollection cfcookie value", cookie.ADMINSIDEBAR, "off");
assert("attributeCollection cfcookie readable in a comparison", (cookie.ADMINSIDEBAR is 'off'), true);
</cfscript>

<!--- explicit attribute overrides an attributeCollection entry of the same name --->
<cfset ckArgs2 = { name="OVERCK", value="from_collection" }>
<cfcookie attributeCollection="#ckArgs2#" value="explicit_wins" />
<cfscript>
assert("explicit attribute overrides attributeCollection", cookie.OVERCK, "explicit_wins");
suiteEnd();
</cfscript>
