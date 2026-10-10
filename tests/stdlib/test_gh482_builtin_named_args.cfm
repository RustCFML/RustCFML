<cfscript>
suiteBegin("Built-in functions bind named arguments BY NAME (GH 482)");

// A named argument to a builtin used to bind in the order it was WRITTEN, so a
// call whose names were not in the declared order silently got the wrong values
// (`listAppend( value=, list= )` returned "x,a") or threw. Lucee and ACF bind by
// name.
assert("listAppend out of declared order", listAppend( value = "x", list = "a" ), "a,x");
assert("listAppend in declared order", listAppend( list = "a", value = "x" ), "a,x");
assert("left( count=, string= )", left( count = 2, string = "abcdef" ), "ab");
assert("right( count=, string= )", right( count = 2, string = "abcdef" ), "ef");
assert("mid out of order", mid( count = 2, string = "abcdef", start = 2 ), "bc");
assert("replace out of order",
	replace( substring2 = "Y", string = "aXb", substring1 = "X" ), "aYb");
assert("reReplace out of order",
	reReplace( substring = "Z", string = "abc", reg_expression = "b" ), "aZc");
assert("dateAdd out of order",
	dateFormat( dateAdd( number = 1, datepart = "d", date = createDate( 2020, 1, 1 ) ), "yyyy-mm-dd" ),
	"2020-01-02");
assert("dateDiff out of order",
	dateDiff( date2 = createDate( 2020, 1, 11 ), datepart = "d", date1 = createDate( 2020, 1, 1 ) ), 10);
assert("structKeyExists out of order", structKeyExists( key = "a", struct = { a = 1 } ), true);
assert("listGetAt out of order", listGetAt( position = 2, list = "a,b,c" ), "b");
assert("listToArray with a named delimiter",
	arrayLen( listToArray( delimiters = "|", list = "a|b|c" ) ), 3);
assert("find out of order", find( string = "hello", substring = "ll" ), 3);
assert("repeatString out of order", repeatString( count = 3, string = "ab" ), "ababab");
assert("arrayToList out of order", arrayToList( delimiter = "|", array = [ 1, 2 ] ), "1|2");
assert("listFind out of order", listFind( value = "b", list = "a,b,c" ), 2);
assert("numberFormat out of order", numberFormat( mask = "0.00", number = 1.5 ), "1.50");

// A positional call is unchanged, and so is a named call that was already in
// declared order.
assert("positional listAppend", listAppend( "a", "x" ), "a,x");
assert("positional left", left( "abcdef", 2 ), "ab");

// ── Lucee's OWN argument names, which the first pass of this table missed ──
// The table was written from the canonical cfdocs name per parameter, so 138
// spellings Lucee accepts — its documented aliases, and in a few cases the name
// Lucee actually declares (`haystack`/`needle` for arrayContains) — matched
// nothing and fell back to binding in CALL ORDER. Reversed, they silently
// misbound. Every case below was measured on Lucee 7.1.0.204 and agrees.
arrGh482 = [ "a", "b", "c" ];
assert("arrayContains by Lucee's own names, reversed",
	arrayContains( needle = "b", haystack = arrGh482 ), 2);
assert("arrayFind by Lucee's own names, reversed",
	arrayFind( value_or_closure = "c", array = arrGh482 ), 3);
assert("replace( substring= ) — Lucee's name for substring1",
	replace( substring2 = "Y", string = "aXb", substring = "X" ), "aYb");
assert("expandPath( relative_path= )", len( expandPath( relative_path = "/x" ) ) GT 0, true);
assert("duplicate( deepcopy=, object= ) reversed",
	isArray( duplicate( deepcopy = true, object = [ 1, 2 ] ) ), true);
assert("arrayReduce by Lucee's alias set, fully reversed",
	arrayReduce( initalvalue = 10, object = [ 1, 2 ], callback = function( a, b ) { return a + b; } ), 13);
// serializeJSON's second parameter is `queryFormat` on Lucee, not the
// `serializeQueryByColumns` the table had; naming it used to be ignored, so a
// query came back in column format whatever was asked for.
gh482q = queryNew( "id,name", "integer,varchar", [ [ 1, "a" ] ] );
gh482rows = deserializeJSON( serializeJSON( queryFormat = "struct", var = gh482q ) );
assert("serializeJSON( queryFormat= ) is honoured when named", isArray( gh482rows ), true);
assert("and it produced a row struct", gh482rows[ 1 ].id, 1);

suiteEnd();
</cfscript>
