<cfscript>
// A single pipe-delimited line, so the driver can assert on parts of it.
writeOutput(
      "tz="       & getTimeZone()
    & "|locale="  & getLocale()
    & "|lsnum="   & lsNumberFormat( 1234.5 )
    & "|settings_tz="     & getApplicationSettings().timezone
    & "|settings_locale=" & getApplicationSettings().locale
);
</cfscript>
