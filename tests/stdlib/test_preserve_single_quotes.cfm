<cfscript>
suiteBegin("PreserveSingleQuotes");

fragment = "person.id::text AS id,'label',person.label";
assert("preserveSingleQuotes returns original SQL fragment", preserveSingleQuotes(fragment), fragment);

suiteEnd();
</cfscript>
