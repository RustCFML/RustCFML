<!--- GH #287: a doubled quote in a LITERAL tag-attribute value is CFML source
      escaping for the delimiting quote. It used to be stored still-escaped and
      then escaped a second time downstream, so the runtime value was wrong
      (`p ""q"" r` instead of `p "q" r`), and the `''` form was never decoded at
      all. Verified against Lucee 7. --->
<cfscript>
suiteBegin("Tags: escaped quotes in literal attributes");

dqExpected = 'p ' & chr(34) & 'q' & chr(34) & ' r';
sqExpected = "x " & chr(39) & "y" & chr(39) & " z";
</cfscript>

<cfparam name="dq" default="p ""q"" r">
<cfparam name="sq" default='x ''y'' z'>
<cfparam name="mixed" default='say "hi"'>
<cfparam name="mixed2" default="it's fine">
<cfparam name="edges" default="""lead"" and trail""">
<cfset innerWord = "IN">
<cfparam name="interp" default="a ""#uCase('b')#"" c">
<cfparam name="litHash" default="cost ##5 ""x""">
<cfparam name="nestedStr" default="#uCase('a' & innerWord & 'b')#">

<cfscript>
assert("double-quoted attribute: '' escape decodes once", dq, dqExpected);
assert("single-quoted attribute: '' escape decodes once", sq, sqExpected);
assert("lone double quote inside a single-quoted attribute is literal",
	mixed, "say " & chr(34) & "hi" & chr(34));
assert("lone single quote inside a double-quoted attribute is literal",
	mixed2, "it" & chr(39) & "s fine");
assert("escaped quotes at both edges of the value",
	edges, chr(34) & "lead" & chr(34) & " and trail" & chr(34));
assert("escaped quotes around an interpolated segment",
	interp, "a " & chr(34) & "B" & chr(34) & " c");
assert("escaped hash and escaped quotes in the same value",
	litHash, "cost " & chr(35) & "5 " & chr(34) & "x" & chr(34));
assert("quotes inside an interpolated expression stay the expression's own",
	nestedStr, "AINB");

// The interpolated path was always correct — pin it so a fix here can't
// regress it.
v = 'p ' & chr(34) & 'q' & chr(34) & ' r';
</cfscript>

<cfparam name="fromVar" default="#v#">
<cfparam name="fromVarWrapped" default="[#v#]">

<cfscript>
assert("interpolated value carrying quotes is untouched", fromVar, dqExpected);
assert("interpolated value with literal wrapper", fromVarWrapped, "[" & dqExpected & "]");

suiteEnd();
</cfscript>
