<!---
  LSParseNumber with the locale's grouping (thousands) separator.

  Adobe ColdFusion and Lucee parse "5,000" as 5000 in an English locale: the
  comma is that locale's grouping separator. RustCFML throws "Cannot convert
  '5,000' to a number". lsParseCurrency already accepts grouping
  ("$1,234.56" -> 1234.56, test_locale_functions.cfm), so LSParseNumber is the
  odd one out.
--->
<cfscript>
suiteBegin( "LSParseNumber: grouping separator" );

lsPrevLocale = setLocale( "English (US)" );

assert( "plain integer", lsParseNumber( "5000" ), 5000 );
assert( "one grouping separator", lsParseNumber( "5,000" ), 5000 );
assert( "several grouping separators", lsParseNumber( "1,234,567" ), 1234567 );
assert( "grouping with decimals", lsParseNumber( "1,234,567.89" ), 1234567.89 );
assert( "negative with grouping", lsParseNumber( "-12,500" ), -12500 );
assert( "surrounding spaces", lsParseNumber( " 5,000 " ), 5000 );

// The locale argument sets both separators: German groups with "." and uses "," for decimals
assert( "German grouping and decimal", lsParseNumber( "1.234.567,89", "German (Standard)" ), 1234567.89 );
assert( "German plain decimal", lsParseNumber( "2,5", "German (Standard)" ), 2.5 );

// Still not a number: these must keep throwing
assertThrows( "letters", function() { lsParseNumber( "abc" ); } );
assertThrows( "empty string", function() { lsParseNumber( "" ); } );

setLocale( lsPrevLocale );
suiteEnd();
</cfscript>
