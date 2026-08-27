<cfscript>
suiteBegin("Parser shapes Lucee accepts");

// Three constructs that Lucee compiles and RustCFML rejected at PARSE time —
// which is what made them severe: a parse error takes the whole component down,
// so one unusual line stopped an entire Preside extension from loading.

// --- 1. A reserved word as a struct key ------------------------------------
// LLM tool definitions and error-frame structs are written this way.
function keywordKeys() {
	var frame = { function = "fname" };
	var tool  = { type = "function", function = { name = "getWeather" }, default = 1, case = 2, for = 3, in = 4, new = 5 };
	return frame.function & "|" & tool.type & "|" & tool.function.name & "|" & tool.default & tool.case & tool[ "for" ] & tool.in & tool.new;
}
assert( "reserved words are legal struct keys", keywordKeys(), "fname|function|getWeather|12345" );

// --- 2. The elvis operator written with a space ----------------------------
// `a ? : b` is the same operator as `a ?: b`.
function spacedElvis( any given ) {
	return ( arguments.given ? : "fallback" );
}
assert( "`? :` falls back when the left side is undefined", spacedElvis(), "fallback" );
assert( "...and passes a supplied value through", spacedElvis( "given" ), "given" );

function multilineElvis( any given ) {
	return ( arguments.given ?
		: "across-lines" );
}
assert( "a newline between `?` and `:` is still elvis", multilineElvis(), "across-lines" );

// The ternary is unaffected.
assert( "the ternary still parses", ( true ? "yes" : "no" ), "yes" );

suiteEnd();
</cfscript>
