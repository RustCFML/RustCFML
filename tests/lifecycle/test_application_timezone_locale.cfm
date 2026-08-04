<!---
  Application.cfc `this.timezone` / `this.locale` — docs/known-issues.md §1.

  Both were parsed into the application's settings struct and then read by
  nothing, which made them the worst kind of no-op: `getApplicationSettings()`
  reported the declared value while every date and every ls* number was still
  formatted in the SERVER's zone and locale. They now seed the same request state
  that the cfconfig `runtime.*` keys and setTimeZone()/setLocale() already used.

  Needs its own Application.cfc, so it runs over HTTP against a fixture app the
  way the other lifecycle suites do, and skips on the CLI where there is no
  server. Verified against Lucee 7.
--->
<cfscript>
suiteBegin( "Lifecycle: this.timezone / this.locale (§1)" );

serverPort = structKeyExists( cgi, "server_port" ) ? trim( cgi.server_port ) : "";

if ( serverPort == "" || serverPort == "0" ) {
    assertTrue( "this.timezone/this.locale skipped (no cgi.server_port)", true );
} else {
    base = "http://127.0.0.1:" & serverPort & "/tests/lifecycle/";

    // --- declared values take effect -------------------------------------
    http url="#base#application_timezone_locale/index.cfm" method="GET" result="r";
    assert( "fixture app responds", r.statuscode, "200 OK" );
    body = trim( r.filecontent );

    assertTrue( "this.timezone is applied to getTimeZone()",
                findNoCase( "tz=Asia/Tokyo|", body ) > 0 );
    assertTrue( "this.locale is applied to getLocale()",
                findNoCase( "|locale=german (standard)|", body ) > 0 );
    // The functional half: the locale must actually reach the ls* family, not
    // just getLocale(). German groups with "." where en_US groups with ",".
    assertTrue( "this.locale reaches lsNumberFormat",
                findNoCase( "|lsnum=1.235|", body ) > 0 );
    // These two always worked — the settings struct was never the problem.
    assertTrue( "settings struct still reports the timezone",
                findNoCase( "|settings_tz=Asia/Tokyo|", body ) > 0 );
    assertTrue( "settings struct still reports the locale",
                findNoCase( "|settings_locale=de_DE", body ) > 0 );

    // --- setTimeZone()/setLocale() still override -------------------------
    http url="#base#application_timezone_locale/override.cfm" method="GET" result="r2";
    assert( "override page responds", r2.statuscode, "200 OK" );
    body2 = trim( r2.filecontent );
    assertTrue( "request starts on the declared zone/locale",
                findNoCase( "before=Asia/Tokyo/german (standard)|", body2 ) > 0 );
    assertTrue( "setTimeZone/setLocale override Application.cfc",
                findNoCase( "|after=America/New_York/english (us)", body2 ) > 0 );

    // --- an unusable value is ignored, not fatal --------------------------
    http url="#base#application_timezone_locale_bad/index.cfm" method="GET" result="r3";
    assert( "bad-value app still boots", r3.statuscode, "200 OK" );
    body3 = trim( r3.filecontent );
    assertTrue( "an unknown locale falls back to the default",
                findNoCase( "locale=english (us)|", body3 ) > 0 );
    assertTrue( "the fallback locale drives ls* too",
                findNoCase( "|lsnum=1,235", body3 ) > 0 );
}

suiteEnd();
</cfscript>
