<cfscript>suiteBegin("Tags: Include");</cfscript>

<!--- cfinclude of a helper file --->
<cfinclude template="_include_target.cfm">
<cfscript>assert("cfinclude sets variable", request._includeTest, "included");</cfscript>

<!--- Verify the type of the included value --->
<cfscript>assertTrue("cfinclude value is string", isSimpleValue(request._includeTest));</cfscript>

<!--- Verify we can use the value after include --->
<cfset includeUpper = uCase(request._includeTest)>
<cfscript>assert("cfinclude value usable", includeUpper, "INCLUDED");</cfscript>

<!--- Bug H: cfinclude path with .. segments must canonicalise.
      `customtags/../_include_target.cfm` should resolve to
      `_include_target.cfm` in the same directory. --->
<cfset request._includeTest = "">
<cfinclude template="customtags/../_include_target.cfm">
<cfscript>assert("cfinclude canonicalises .. segments", request._includeTest, "included");</cfscript>

<!--- Leading-slash includes are webroot-relative (issue 21).
      The entry template's directory is treated as the webroot in CLI mode,
      so /tests/tags/_include_target.cfm resolves to the same file the
      relative include above used. --->
<cfset request._includeTest = "">
<cfinclude template="/tags/_include_target.cfm">
<cfscript>assert("cfinclude resolves leading-slash via webroot", request._includeTest, "included");</cfscript>

<!--- A cfinclude executed INSIDE a function runs in the caller's function-`local`
      scope: `local.x` set before the include is both readable and shared across
      nested includes. (Masa CMS admin cplugins/dsp_table.cfm: list.cfm sets
      `local.category` then includes dsp_table.cfm which reads it — this used to
      throw "Variable 'category' is undefined".) --->
<cffunction name="_includeLocalScopeTest" output="false">
	<cfset local.category = "Application">
	<cfinclude template="_include_local_scope_target.cfm">
</cffunction>
<cfset _includeLocalScopeTest()>
<cfscript>
	assert("cfinclude shares caller function-local scope", request._incLocalCategory, "Application");
	assertTrue("local.x is in the included file's local scope", request._incLocalStructExists);
</cfscript>

<!--- Top-level (non-function) include must NOT expose page `variables` as
      `local.*` — a page template has no function-local scope. --->
<cfset _incLocalPageVar = "pageValue">
<cfset request._incLocalPageLeak = true>
<cfinclude template="_include_local_scope_probe.cfm">
<cfscript>
	assertFalse("top-level include does not leak page vars into local scope", request._incLocalPageLeak);
</cfscript>

<cfscript>suiteEnd();</cfscript>
