<cfscript>
// GH #331 — an UNQUOTED tag attribute value is a LITERAL STRING, never an
// expression. Lucee's transformer (AbstrCFMLExprTransformer.transformAsString)
// tries a quoted string, then ONE whole `#...#`, then `simple()` — a literal run
// terminated by whitespace, `>` or `/>`. It never reaches the expression parser.
// We used to guess with a `quote_if_needed` heuristic that treated `.`, `/`, `(`
// and `[` as "looks like an expression", so `default=a.b` silently resolved a
// variable and `default=http://x/?a=` failed to parse at all.
// Every assertion here is verified green on Lucee 7.0.4.34 too.
suiteBegin("Tags: unquoted attribute values are literal strings");

// A variable whose dotted path WOULD resolve — the silent-wrong-value case.
a = { b = "SURPRISE" };
</cfscript>

<cfparam name="u1" default=abc>
<cfparam name="u2" default=a.b>
<cfparam name="u3" default=a/b>
<cfparam name="u4" default=a.b.c.d>
<cfparam name="u5" default=1+2>
<cfparam name="u6" default=/foo/bar.cfm>
<cfparam name="u7" default=http://x/?a=>
<cfparam name="u8" default=abc=>
<cfparam name="u9" default=arr[1]>
<cfparam name="u10" default=5>
<cfparam name="u11" default=-5>
<cfparam name="u12" default=true>
<!--- A whole `#expr#` IS still an expression, and keeps its native type. --->
<cfparam name="u13" default=#a.b#>
<cfparam name="u14" default=#[1,2,3]#>

<cfscript>
assert("plain word stays literal", u1, "abc");
assert("dotted path is NOT a variable read", u2, "a.b");
assert("slash is not division", u3, "a/b");
assert("deep dotted path stays literal", u4, "a.b.c.d");
assert("arithmetic stays literal", u5, "1+2");
assert("leading-slash path stays literal", u6, "/foo/bar.cfm");
assert("unquoted URL parses and stays literal", u7, "http://x/?a=");
assert("trailing = is part of the value", u8, "abc=");
assert("bracket index stays literal", u9, "arr[1]");
assert("bare number stays a numeric-castable string", u10 & "", "5");
assert("negative number stays literal", u11 & "", "-5");
assert("bare true stays literal", u12 & "", "true");
assert("a whole hash expression is still evaluated", u13, "SURPRISE");
assertTrue("a whole hash expression keeps its native type", isArray(u14));
assert("a whole hash expression yields the real array", arrayLen(u14), 3);
</cfscript>

<!--- Same rule on a tag that funnels its attributes through the generic
      struct-literal path rather than cfparam's dedicated one. --->
<cfset thrown = "">
<cftry>
	<cfthrow message=a.b>
	<cfcatch><cfset thrown = cfcatch.message></cfcatch>
</cftry>

<cfscript>
assert("unquoted cfthrow message stays literal", thrown, "a.b");
suiteEnd();
</cfscript>
