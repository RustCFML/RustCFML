<cfscript>
suiteBegin("Tag-context expression: nested quotes in string interpolation (GH ##257)");

o = createObject("component", "tags.TagReturnNestedQuoteInterpFixture");

// The tag-context expression tokeniser must treat the inner '"' / '""' / 'all'
// single-quoted strings as arguments INSIDE the #...# interpolation, then
// resume the outer single-quoted string — matching the script path.
assert("cfreturn nested-quote interpolation", o.q("say ""hi"""), '"say ""hi"""');
assert("cfset nested-quote interpolation", o.viaSet("say ""hi"""), '"say ""hi"""');

// Sanity: the identical expression parses/evaluates in script context too.
value  = "he said ""hi""";
result = '"#replaceNoCase( value, '"', '""', 'all' )#"';
assert("cfscript path unchanged", result, '"he said ""hi"""');

suiteEnd();
</cfscript>
