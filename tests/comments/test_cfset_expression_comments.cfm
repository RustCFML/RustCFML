<cfscript>
suiteBegin("Comments: cfset expression comments");
</cfscript>

<cfset records = [
	{ name: "before" },
	<!--- Lucee ignores CFML comments inside tag-mode expression bodies. --->
	{ name: "after" }
] />

<cfscript>
assert("cfset expression comments are ignored inside array literals",
	records[2].name,
	"after");

suiteEnd();
</cfscript>
