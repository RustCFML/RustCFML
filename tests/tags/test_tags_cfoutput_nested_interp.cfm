<cfscript>
suiteBegin("cfoutput nested interpolation");
</cfscript>

<cfset tc = { template = "foo.cfm", line = 42 }>
<cfset isFirst = true>

<!--- Bare interpolated string literal nested inside an output hash expression.
      Regression: find_closing_hash used to stop at the nested string's hash,
      desyncing the parser (Preside errorTemplate.cfm parse failure). --->
<cfsavecontent variable="out1"><cfoutput>#"line #tc.line#"#</cfoutput></cfsavecontent>
<cfscript>
assert("bare nested interp in cfoutput hash", trim(out1), "line 42");
</cfscript>

<!--- Ternary whose branches are interpolated string literals, with a literal
      colon inside (matches Preside errorTemplate.cfm line 102/110). --->
<cfsavecontent variable="out2"><cfoutput>#isFirst ? "<b>#tc.template#: line #tc.line#</b>" : "called from #tc.template#: line #tc.line#"#</cfoutput></cfsavecontent>
<cfsavecontent variable="out3"><cfoutput>#false ? "<b>#tc.template#: line #tc.line#</b>" : "called from #tc.template#: line #tc.line#"#</cfoutput></cfsavecontent>
<cfscript>
assert("ternary true branch, nested interp + colon", trim(out2), "<b>foo.cfm: line 42</b>");
assert("ternary false branch, nested interp + colon", trim(out3), "called from foo.cfm: line 42");
</cfscript>

<cfscript>
suiteEnd();
</cfscript>
