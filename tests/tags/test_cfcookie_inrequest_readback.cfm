<cfscript>
suiteBegin("Tags: cfcookie in-request scope readback (Lucee parity)");

// Regression (Preside cbi18n getFwLocale): <cfcookie name="x" value="y"> must update the
// readable `cookie` scope in the SAME request, not only emit the Set-Cookie response
// header. cbi18n's getfwLocale() does `<cfcookie name="DefaultLocale" value=...>` then
// reads `cookie.DefaultLocale` back — which returned undefined until this fix.

cfcookie( name="RCFML_READBACK", value="en_GB" );

assertTrue( "cookie key exists in-request after cfcookie", structKeyExists( cookie, "RCFML_READBACK" ) );
assert( "cookie value readable in-request", cookie.RCFML_READBACK, "en_GB" );

// A scope captured into a var BEFORE the write must still see it (structs are by-reference).
storageAlias = cookie;
cfcookie( name="RCFML_READBACK2", value="fr_FR" );
assertTrue( "alias of cookie scope sees later cfcookie write", structKeyExists( storageAlias, "RCFML_READBACK2" ) );
assert( "alias value readable", storageAlias.RCFML_READBACK2, "fr_FR" );

// Overwriting an existing cookie updates the in-request value too.
cfcookie( name="RCFML_READBACK", value="de_DE" );
assert( "cfcookie overwrite updates in-request value", cookie.RCFML_READBACK, "de_DE" );

suiteEnd();
</cfscript>
