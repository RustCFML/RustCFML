<cfscript>
// cfexecute `arguments` string parsing: Lucee tokenizes SHELL-STYLE — a
// double-quoted span is ONE argument with the quotes stripped. RustCFML
// splits on whitespace straight through the quotes and passes the quote
// characters to the program literally:
//
//   arguments='"quoted arg" plain'
//     Lucee    -> argv: [quoted arg] [plain]        (echo: quoted arg plain)
//     RustCFML -> argv: ["quoted] [arg"] [plain]    (echo: "quoted arg" plain)
//
// The whitespace-preservation leg is the discriminating one: '"two  spaces"'
// must keep its DOUBLE space (one argv entry); splitting loses it when echo
// rejoins with single spaces.
//
// Repro class: titan (Moopa) renders every PDF by shelling out to the typst
// CLI with conventionally-quoted paths — and it turned out typst had NEVER
// run via cfexecute on this engine: it received '"/path/out.pdf"' quotes
// included and died with `could not infer output format for path` (extension
// = `pdf"`). All call sites had to be rewritten unquoted with space-free
// paths. Any Lucee code that quotes an argument — which is what you do the
// moment a path may contain spaces — mis-executes here.
//
// (The existing CfexecuteQuotedArgFixture pins only that the SOURCE parses;
// it sits behind <cfif false> and never executes. This suite pins runtime
// semantics. POSIX-only by the same convention as the other cfexecute tests.)
//
// The first three legs are from Matthew (@Blute), GH PR #333. The argv-boundary
// legs below were added with the fix: they use `printf` (which reuses its
// format, so every argv entry prints as `[entry]`) to see the exact argument
// boundaries that `echo` hides by rejoining with single spaces. All of them
// were measured byte-identical on Lucee 7.1.0.204, and they pin the parts of
// Lucee's tokenizer (lucee.commons.cli.Command.toList) that the echo legs
// cannot reach — most importantly that an UNMATCHED quote stays literal, which
// is what keeps `it's` working as an ordinary argument.

suiteBegin("cfexecute arguments: double-quoted spans group as one argument, quotes stripped");
</cfscript>

<cfset plainOut = "" />
<cfexecute name="/bin/echo" arguments='plain one' variable="plainOut" timeout="10" />
<cfscript>assert( "control: unquoted arguments split on whitespace", trim( plainOut ), "plain one" );</cfscript>

<cfset quotedOut = "" />
<cfexecute name="/bin/echo" arguments='"quoted arg" plain' variable="quotedOut" timeout="10" />
<cfscript>assert( "double quotes are stripped, not passed to the program", trim( quotedOut ), "quoted arg plain" );</cfscript>

<cfset groupOut = "" />
<cfexecute name="/bin/echo" arguments='"two  spaces"' variable="groupOut" timeout="10" />
<cfscript>
assert( "a quoted span is ONE argument (internal double space preserved)", trim( groupOut ), "two  spaces" );
</cfscript>

<!--- argv-boundary legs: `[%s] ` makes printf bracket every argument it receives. --->
<cfset argvCases = [
      { args = 'plain one',                 want = "[plain][one]",              label = "unquoted splits on whitespace" }
    , { args = "it's fine",                 want = "[it's][fine]",              label = "an unmatched quote is literal, not an opening quote" }
    , { args = 'say "hi',                   want = '[say]["hi]',                label = "an unmatched double quote is passed through literally" }
    , { args = '"it''s here"',              want = "[it's here]",               label = "the other quote character inside a span is literal" }
    , { args = 'a"b c"d',                   want = "[ab cd]",                   label = "quotes suppress whitespace, they do not delimit arguments" }
    , { args = 'a "" b',                    want = "[a][b]",                    label = "an empty quoted argument is dropped entirely" }
    , { args = 'a "   " b',                 want = "[a][b]",                    label = "a whitespace-only quoted argument is dropped" }
    , { args = '--path="/a b/c"',           want = "[--path=/a b/c]",           label = "a quoted span mid-token joins the surrounding token" }
    , { args = '''single quoted arg''',     want = "[single quoted arg]",       label = "single quotes group exactly like double quotes" }
    , { args = '"C:\Program Files\app.exe"', want = "[C:\Program Files\app.exe]", label = "backslashes are never escapes (Windows paths survive)" }
] />
<cfloop from="1" to="#arrayLen( argvCases )#" index="argvI">
    <cfset argvOut = "" />
    <cfexecute name="/usr/bin/printf" arguments='[%s] #argvCases[ argvI ].args#' variable="argvOut" timeout="10" />
    <cfscript>assert( "argv: " & argvCases[ argvI ].label, trim( argvOut ), argvCases[ argvI ].want );</cfscript>
</cfloop>

<cfscript>
suiteEnd();
</cfscript>
