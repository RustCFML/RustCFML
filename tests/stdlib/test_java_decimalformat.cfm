<cfscript>
// java.text.DecimalFormat shim. Cross-engine: on Lucee these run against the
// REAL java.text.DecimalFormat, so a green run on both engines is the proof.
// (`@` stands in for the pattern hash so neither engine's `##` escaping can
// mangle the literal.)
suiteBegin( "java.text.DecimalFormat shim" );

function df( required string pattern ) {
    return createObject( "java", "java.text.DecimalFormat" )
               .init( replace( arguments.pattern, "@", chr(35), "all" ) );
}

// ---- optional vs required fraction digits ----------------------------------
assert( "0.@@ keeps 2 significant decimals", df( "0.@@" ).format( 3.14159 ), "3.14" );
assert( "0.@@ drops trailing zeros",          df( "0.@@" ).format( 3.0 ),     "3"    );
assert( "0.@@ keeps one when present",        df( "0.@@" ).format( 3.1 ),     "3.1"  );
assert( "0.00 pads to 2",                     df( "0.00" ).format( 3.0 ),     "3.00" );
assert( "0.000 pads to 3",                    df( "0.000" ).format( 1.0 ),    "1.000" );
assert( "0 truncates the fraction",           df( "0" ).format( 7.6 ),        "8"    );

// An all-@ integer pattern still prints the units digit — `@.@@` of 0.5 is
// "0.5", not ".5".
assert( "@.@@ keeps the units digit", df( "@.@@" ).format( 0.5 ), "0.5" );
assert( "@.@@ of zero",               df( "@.@@" ).format( 0.0 ), "0"   );

// ---- minimum integer digits -------------------------------------------------
assert( "000 pads the integer part", df( "000" ).format( 7.0 ),  "007"     );
assert( "0000.00 pads both sides",   df( "0000.00" ).format( 12.3 ), "0012.30" );

// ---- grouping: the SIZE comes from the pattern -----------------------------
assert( "@,@@0.00 groups by 3", df( "@,@@0.00" ).format( 1234567.891 ), "1,234,567.89" );
assert( "@,@@0 groups by 3",    df( "@,@@0" ).format( 1234.0 ),         "1,234"        );
// Two digit positions after the last comma = group by TWO (the lakh/crore form).
assert( "@,@0.00 groups by 2",  df( "@,@0.00" ).format( 1234567.891 ),  "1,23,45,67.89" );
assert( "@,@0 groups by 2",     df( "@,@0" ).format( 1234.0 ),          "12,34"         );

// ---- rounding is half-even on the EXACT value ------------------------------
// 0.125 is exactly representable, so it is a true tie and goes to the even 0.12.
assert( "an exact tie goes to even (0.125)", df( "0.00" ).format( 0.125 ), "0.12" );
assert( "an exact tie goes to even (7.5)",   df( "0" ).format( 7.5 ),      "8"    );
assert( "an exact tie goes to even (8.5)",   df( "0" ).format( 8.5 ),      "8"    );
// 0.005 is NOT 0.005 as a double — it is a hair above, so it rounds UP.
assert( "a near-tie rounds by its real value", df( "0.00" ).format( 0.005 ), "0.01" );
assert( "...and does so when negative",        df( "0.00" ).format( -0.005 ), "-0.01" );
assert( "0.135 rounds up",                     df( "0.00" ).format( 0.135 ), "0.14" );
assert( "carry out of the fraction",           df( "0.@@" ).format( 0.999 ), "1"    );
assert( "carry with required digits",          df( "0.00" ).format( 0.999 ), "1.00" );

// ---- sign, percent and literal prefix/suffix -------------------------------
assert( "negative keeps the sign",  df( "@,@@0.00" ).format( -45.5 ), "-45.50" );
assert( "percent scales by 100",    df( "@@0.0%" ).format( 0.256 ),   "25.6%"  );
assert( "a literal prefix is kept", df( "USD @,@@0.00" ).format( 1234.5 ), "USD 1,234.50" );
assert( "a literal suffix is kept", df( "@,@@0.00 EUR" ).format( 1234.5 ), "1,234.50 EUR" );

// ---- pattern round-trip and accessors --------------------------------------
f = df( "@,@@0.00" );
assert( "toPattern round-trips", f.toPattern(), replace( "@,@@0.00", "@", chr(35), "all" ) );
assert( "isGroupingUsed",        f.isGroupingUsed(), true );
assert( "getMinimumFractionDigits", f.getMinimumFractionDigits(), 2 );
assert( "getMaximumFractionDigits", f.getMaximumFractionDigits(), 2 );
g = df( "0.@@@" );
assert( "min/max differ for optional digits", g.getMinimumFractionDigits(), 0 );
assert( "...max counts every position",       g.getMaximumFractionDigits(), 3 );
assert( "no grouping without a comma",        g.isGroupingUsed(), false );

// applyPattern replaces the pattern in place.
h2 = df( "0" );
h2.applyPattern( replace( "0.00", "@", chr(35), "all" ) );
assert( "applyPattern takes effect", h2.format( 1.5 ), "1.50" );

suiteEnd();
</cfscript>
