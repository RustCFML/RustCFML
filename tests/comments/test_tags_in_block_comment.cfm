<cfscript>
suiteBegin("Comments: tags inside a block comment (component body)");

// A /* */ block comment in a script-component body must be opaque: literal CFML
// tags inside it (<cfset>, <cfoutput>) are documentation, not markup — they must
// not be lexed or executed, and the component must still compile to a normal CFC.
// Lucee, Adobe CF, and BoxLang all treat the comment interior as inert. (Closed
// tags only — an UNCLOSED tag inside a comment is a separate cross-engine hazard.)
//
// Wheels' Test.cfc documents <cfset>/<cfoutput> usage inside such a comment, so
// this affects real framework code.
o = createObject("component", "comments.BlockCommentTags");
assert("component with closed tags in a /* */ comment compiles and runs", o.ping(), "pong");

suiteEnd();
</cfscript>
