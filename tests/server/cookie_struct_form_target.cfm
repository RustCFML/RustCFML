<cfscript>
// Sets cookies in both forms, then reads them back in the SAME request. The
// long form is a write instruction, so a read-back must give the value string
// (Lucee), not the struct that set it (GH #480).
cookie[ "gh480a" ] = { value = "alpha", httpOnly = true, path = "/" };
cookie.gh480b = { value = "beta", secure = false };
cookie.gh480c = "gamma";
out = [];
for ( k in [ "gh480a", "gh480b", "gh480c" ] ) {
	v = cookie[ k ];
	arrayAppend( out, k & "=" & ( isStruct( v ) ? "STRUCT" : v ) );
}
// Concatenation is what actually breaks when a struct is left in the scope.
arrayAppend( out, "concat=" & ( isStruct( cookie.gh480a ) ? "STRUCT" : cookie.gh480a & "" ) );
writeOutput( arrayToList( out, "|" ) );
</cfscript>
