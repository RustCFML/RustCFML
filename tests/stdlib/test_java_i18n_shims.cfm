<cfscript>
// Java shims that ColdBox's cbi18n module (models/i18n.cfc) leans on:
// java.util.Locale / java.util.TimeZone / java.util.GregorianCalendar and the
// java.text.* date/number-formatting classes. Values are ground-truthed against
// Lucee 7.0.4 / OpenJDK 21, so this suite must stay green on both engines — the
// asserted cases are exactly the ones where RustCFML's shim matches the JVM.
suiteBegin("Java i18n shims (Locale / TimeZone / DateFormat)");

aLocale  = createObject( "java", "java.util.Locale" );
timeZone = createObject( "java", "java.util.TimeZone" );

// ---- Locale ----------------------------------------------------------------
enus = aLocale.init( "en", "US" );
assert( "Locale(en,US).toString()", enus.toString(), "en_US" );
assert( "Locale(en,US).getLanguage()", enus.getLanguage(), "en" );
assert( "Locale(en,US).getCountry()", enus.getCountry(), "US" );
assert( "Locale(en,US).getDisplayName()", enus.getDisplayName(), "English (United States)" );
assert( "Locale(en,US).getDisplayCountry()", enus.getDisplayCountry(), "United States" );
assert( "Locale(en,US).getDisplayLanguage()", enus.getDisplayLanguage(), "English" );
assert( "Locale(en,US).getISO3Country()", enus.getISO3Country(), "USA" );
assert( "Locale(en,US).getISO3Language()", enus.getISO3Language(), "eng" );

en = aLocale.init( "en" );
assert( "Locale(en).toString()", en.toString(), "en" );
assert( "Locale(en).getCountry() is empty", len( en.getCountry() ), 0 );

// getAvailableLocales() returns Locale objects (stringify to their id), so
// isValidLocale's `listFind( arrayToList( ... ), id )` works.
avail = aLocale.getAvailableLocales();
assertTrue( "getAvailableLocales contains en_US", listFindNoCase( arrayToList( avail ), "en_US" ) GT 0 );
assertTrue( "getAvailableLocales contains en_GB", listFindNoCase( arrayToList( avail ), "en_GB" ) GT 0 );

// ---- TimeZone --------------------------------------------------------------
assert( "TimeZone.LONG", timeZone.LONG, 1 );
assert( "TimeZone.SHORT", timeZone.SHORT, 0 );
assert( "TimeZone.getTimeZone(UTC).getID()", timeZone.getTimeZone( "UTC" ).getID(), "UTC" );
assert( "TimeZone.getDefault() is non-null", isNull( timeZone.getDefault() ), false );

// ---- DateFormatSymbols (en_US) ---------------------------------------------
dfs = createObject( "java", "java.text.DateFormatSymbols" ).init( enus );
assert( "DFS.getMonths()[1]", dfs.getMonths()[1], "January" );
assert( "DFS.getShortMonths()[6]", dfs.getShortMonths()[6], "Jun" );
assert( "DFS.getWeekdays()[2]", dfs.getWeekdays()[2], "Sunday" );
assert( "DFS.getAmPmStrings()[2]", dfs.getAmPmStrings()[2], "PM" );

// ---- DecimalFormatSymbols (en_US) ------------------------------------------
dcs = createObject( "java", "java.text.DecimalFormatSymbols" ).init( enus );
assert( "DCS.getPercent()", dcs.getPercent().toString(), "%" );
assert( "DCS.getDecimalSeparator()", dcs.getDecimalSeparator().toString(), "." );
assert( "DCS.getGroupingSeparator()", dcs.getGroupingSeparator().toString(), "," );

// ---- DateFormat: locale-aware date/time formatting -------------------------
df = createObject( "java", "java.text.DateFormat" );
assert( "DateFormat.FULL", df.FULL, 0 );
assert( "DateFormat.LONG", df.LONG, 1 );
assert( "DateFormat.MEDIUM", df.MEDIUM, 2 );
assert( "DateFormat.SHORT", df.SHORT, 3 );

d = createDateTime( 2024, 6, 10, 14, 5, 9 );
enGB = aLocale.init( "en", "GB" );

// Date styles — en/en_US (month-first) vs en_GB (day-first), verified vs JVM.
assert( "date en_US SHORT",  df.getDateInstance( df.SHORT,  enus ).format( d ), "6/10/24" );
assert( "date en_US MEDIUM", df.getDateInstance( df.MEDIUM, enus ).format( d ), "Jun 10, 2024" );
assert( "date en_US LONG",   df.getDateInstance( df.LONG,   enus ).format( d ), "June 10, 2024" );
assert( "date en_US FULL",   df.getDateInstance( df.FULL,   enus ).format( d ), "Monday, June 10, 2024" );
assert( "date en SHORT",     df.getDateInstance( df.SHORT,  en   ).format( d ), "6/10/24" );
assert( "date en_GB SHORT",  df.getDateInstance( df.SHORT,  enGB ).format( d ), "10/06/2024" );
assert( "date en_GB MEDIUM", df.getDateInstance( df.MEDIUM, enGB ).format( d ), "10 Jun 2024" );
assert( "date en_GB FULL",   df.getDateInstance( df.FULL,   enGB ).format( d ), "Monday, 10 June 2024" );

// Time styles without a timezone field (SHORT/MEDIUM). The separator before the
// AM/PM marker is U+202F (narrow no-break space), per the JDK 21 / CLDR pattern.
nnbsp = chr( 8239 );
assert( "time en_US SHORT",  df.getTimeInstance( df.SHORT,  enus ).format( d ), "2:05" & nnbsp & "PM" );
assert( "time en_US MEDIUM", df.getTimeInstance( df.MEDIUM, enus ).format( d ), "2:05:09" & nnbsp & "PM" );
assert( "time en_GB SHORT",  df.getTimeInstance( df.SHORT,  enGB ).format( d ), "14:05" );
assert( "time en_GB MEDIUM", df.getTimeInstance( df.MEDIUM, enGB ).format( d ), "14:05:09" );

// DateTime (date style accessed by name like cbi18n does: aDateFormat["SHORT"]).
assert( "datetime en SHORT/SHORT", df.getDateTimeInstance( df.SHORT, df.SHORT, en ).format( d ), "6/10/24, 2:05" & nnbsp & "PM" );
assert( "style by-name lookup df['SHORT']", df[ "SHORT" ], 3 );

// Time styles WITH a timezone field (LONG=z short abbrev, FULL=zzzz long name),
// now backed by the IANA tz database (chrono-tz) + a Lucee-verified name table.
// June 10 is summer in New York -> EDT. Byte-identical to the JVM/Lucee.
// `d` is an instant in the REQUEST timezone, and a formatter bound to another
// zone converts it (Lucee 7.0/7.1). Pin the request zone to New York for these
// two so the expected text holds on any machine, then put it back.
priorTz = getTimezone();
setTimezone( "America/New_York" );
dNy = createDateTime( 2024, 6, 10, 14, 5, 9 );
nyTZ = timeZone.getTimeZone( "America/New_York" );
fLong = df.getTimeInstance( df.LONG, enus ); fLong.setTimeZone( nyTZ );
fFull = df.getTimeInstance( df.FULL, enus ); fFull.setTimeZone( nyTZ );
assert( "time en_US LONG (z abbrev)", fLong.format( dNy ), "2:05:09" & nnbsp & "PM EDT" );
assert( "time en_US FULL (zzzz long name)", fFull.format( dNy ), "2:05:09" & nnbsp & "PM Eastern Daylight Time" );
// The same instant seen from UTC is converted, not relabelled.
setTimezone( "UTC" );
dUtc = createDateTime( 2024, 6, 10, 14, 5, 9 );
assert( "a UTC instant formatted in New York is converted", fLong.format( dUtc ), "10:05:09" & nnbsp & "AM EDT" );
setTimezone( priorTz );

// RustCFML-specific: rather than emit a guessed string, the shim still throws
// for what it can't reproduce faithfully — an unverified locale (CLDR pattern
// data we only tabulate for en*). On Lucee this returns real JVM output.
if ( isRustCFML() ) {
	assertThrows( "unverified locale throws, not guessed", function(){
		df.getDateInstance( df.SHORT, aLocale.init( "fr", "FR" ) ).format( d );
	} );
}

suiteEnd();
</cfscript>
