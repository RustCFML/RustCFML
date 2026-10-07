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

suiteEnd();
</cfscript>
