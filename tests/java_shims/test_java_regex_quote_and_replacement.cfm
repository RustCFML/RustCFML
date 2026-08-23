<cfscript>
suiteBegin("java.util.regex: \Q…\E quoting, Matcher/Pattern statics, Java replacement syntax (GH ##338)");

// Preside's EmailTemplateService.replaceParameterTokens — the function that
// swaps ${param} tokens in EVERY email subject and body — needs all four of
// the things pinned here at once:
//
//     var Matcher  = CreateObject( "java", "java.util.regex.Matcher" );
//     var token    = "(?i)\Q${#paramName#}\E";
//     replaced     = replaced.replaceAll( token, Matcher.quoteReplacement( value ) );
//
// Every value below was measured on Lucee 7.1.0.204 (box server, 2026-08-23).
//
// The replacement-syntax legs matter as much as the \Q…\E ones: Java and the
// Rust regex crate are different dialects (`$1` vs `${1}`, `\$` vs `$$`), so
// quoteReplacement's output is meaningless unless it is translated. Getting
// this half wrong is invisible in the \Q…\E legs alone.

function tryIt( body ) {
    try { return body(); } catch ( any e ) { return "THREW: " & e.message; }
}

// ── \Q…\E quotes a literal region in the pattern
assert( "\Q…\E matches an interpolated token literally",
        "Hello ${user_name}!".replaceAll( "(?i)\Q${user_name}\E", "Jan" ), "Hello Jan!" );
assert( "\Q…\E neutralises regex metacharacters",
        "a.b|c".replaceAll( "\Q.b|\E", "-" ), "a-c" );
assert( "an unclosed \Q runs to the end of the pattern",
        "a.b*c".replaceAll( "\Q.b*", "-" ), "a-c" );
assert( "an empty \Q\E region is a no-op",
        "abc".replaceAll( "\Q\Eb", "-" ), "a-c" );
assert( "a quantifier after \E applies to the quoted region",
        "aXbXc".replaceAll( "\QX\E+", "-" ), "a-b-c" );
assert( "\Q…\E works inside a character class",
        "a-b".replaceAll( "[\Q-\E]", "+" ), "a+b" );
assert( "a backslash inside \Q…\E is literal",
        "a\b".replaceAll( "\Q\\E", "/" ), "a/b" );
assertTrue( "a lone \E without \Q is rejected, as on the JVM",
        left( tryIt( function(){ return "abc".replaceAll( "a\Eb", "-" ); } ), 6 ) == "THREW:" );

// ── the statics, reached through a bare class handle
matcherClass = createObject( "java", "java.util.regex.Matcher" );
patternClass = createObject( "java", "java.util.regex.Pattern" );
assert( "Matcher.quoteReplacement escapes backslash and dollar",
        matcherClass.quoteReplacement( "a$b\c" ), "a\$b\\c" );
assert( "Matcher.quoteReplacement leaves a plain string alone",
        matcherClass.quoteReplacement( "plain" ), "plain" );
assert( "Pattern.quote wraps the string in \Q…\E",
        patternClass.quote( "a.b*c" ), "\Qa.b*c\E" );

// ── Java replacement syntax, which is NOT the Rust regex crate's
assert( "group references reorder captures",
        "John Smith".replaceAll( "(\w+) (\w+)", "$2 $1" ), "Smith John" );
assert( "a backslash-escaped dollar is a literal dollar",
        "cost".replaceAll( "cost", "\$5" ), "$5" );
assert( "a doubled backslash is a literal backslash",
        "x".replaceAll( "x", "a\\b" ), "a\b" );
assert( "an escaped ${...} is literal, not a named group",
        "ab".replaceAll( "(?<first>a)(b)", "\${first}!" ), "${first}!" );
assert( "a digit run stops at the last real group",
        "ab".replaceAll( "(a)(b)", "$12" ), "a2" );

// ── the JVM rejects these; the Rust crate would silently yield an empty
//    string, so they must throw rather than lose data
assert( "$$ is an illegal group reference on the JVM",
        tryIt( function(){ return "x".replaceAll( "x", "a$$b" ); } ),
        "THREW: Illegal group reference" );
assert( "$ before a letter is an illegal group reference",
        tryIt( function(){ return "x".replaceAll( "x", "cost $f" ); } ),
        "THREW: Illegal group reference" );
assert( "a trailing $ names no group",
        tryIt( function(){ return "x".replaceAll( "x", "100$" ); } ),
        "THREW: Illegal group reference: group index is missing" );
assert( "an out-of-range group number throws, never returns empty",
        tryIt( function(){ return "ab".replaceAll( "(a)(b)", "$3" ); } ),
        "THREW: No group 3" );
assert( "an unknown group NAME throws, never returns empty",
        tryIt( function(){ return "ab".replaceAll( "(a)(b)", "${nope}" ); } ),
        "THREW: No group with name {nope}" );

// ── end to end: the exact Preside idiom, including a value that carries both
//    characters quoteReplacement exists to protect
presideOut = "";
(function(){
    var Matcher  = createObject( "java", "java.util.regex.Matcher" );
    var replaced = javaCast( "String", "Hi ${name}, cost is ${amt}" );
    var params   = { "name" = "a$b", "amt" = "US\$5" };
    for ( var k in params ) {
        replaced = replaced.replaceAll( "(?i)\Q${" & k & "}\E", Matcher.quoteReplacement( params[ k ] ) );
    }
    presideOut = replaced;
})();
assert( "Preside's replaceParameterTokens idiom round-trips $ and \ verbatim",
        presideOut, "Hi a$b, cost is US\$5" );

suiteEnd();
</cfscript>
