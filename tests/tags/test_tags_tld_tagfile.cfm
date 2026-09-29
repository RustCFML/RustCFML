<cfscript>
suiteBegin("TLD tag-file mapping is read relative to the template");
// <cfimport taglib="tldlib_tagfile"> used to look for the .tld relative to the
// process's working directory, so from any other directory the descriptor was
// silently skipped and a tag whose name differs from its file never resolved.
</cfscript>

<cfimport taglib="tldlib_tagfile" prefix="tf">

<cfsavecontent variable="greetResult"><tf:greet name="TLD"></cfsavecontent>
<cfscript>
assert("a TLD tag-file mapping resolves from the template's directory", trim(greetResult), "Hello, TLD!");

suiteEnd();
</cfscript>
