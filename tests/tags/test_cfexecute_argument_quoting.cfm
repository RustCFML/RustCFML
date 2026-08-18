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

suiteEnd();
</cfscript>
