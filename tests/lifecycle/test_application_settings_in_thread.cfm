<!---
  getApplicationSettings() inside a cfthread.

  An Application.cfc can declare keys of its own on `this` beside the standard
  ones (this.storage = {...}, this.projectLabel = "..."), and
  getApplicationSettings() reports them. In a request RustCFML does. Inside a
  cfthread RustCFML v0.751.0 returns only the standard keys (name, mappings,
  sessionManagement, ...): the application's own keys are missing, so code that
  reads its configuration from getApplicationSettings() works in a request and
  finds nothing in a background thread. Lucee reports the same settings in both.

  Real-world shape: a library that builds a storage client from
  getApplicationSettings().s3 (Lucee's own name for an application's S3
  settings). A page that writes a file works; the same write from a cfthread
  fails with "not configured".

  Needs its own Application.cfc, so it runs over HTTP against a fixture app the
  way the other lifecycle suites do, and skips on the CLI where there is no
  server. Verified against Lucee 6.2.
--->
<cfscript>
suiteBegin( "Lifecycle: getApplicationSettings() inside a cfthread" );

serverPort = structKeyExists( cgi, "server_port" ) ? trim( cgi.server_port ) : "";

if ( serverPort == "" || serverPort == "0" ) {
    assertTrue( "getApplicationSettings() in a cfthread skipped (no cgi.server_port)", true );
} else {
    http url="http://127.0.0.1:#serverPort#/tests/lifecycle/application_settings_in_thread/index.cfm" method="GET" result="r";
    // Lucee 6 reports "200", Lucee 7 and RustCFML "200 OK".
    assert( "fixture app responds", left( r.statuscode, 3 ), "200" );
    body = trim( r.filecontent );

    expected = "label=kept,host=https://storage.example.com,region=auto,appname=rcfml_settings_in_thread_fixture";

    // Control: the request sees the application's own keys.
    inRequest = reReplace( body, "^request\[(.*?)\]\|.*$", "\1" );
    assert( "in the request: the application's own keys are reported", inRequest, expected );

    // The same call inside a cfthread reports the same settings.
    inThread = reReplace( body, "^.*\|thread\[(.*)\]$", "\1" );
    assert( "in a cfthread: the application's own keys are reported", inThread, expected );
    assertTrue( "in a cfthread: a simple key (this.projectLabel) is there",
                find( "label=kept,", inThread ) > 0 );
    assertTrue( "in a cfthread: a struct key (this.storage) is there, with its members",
                find( ",host=https://storage.example.com,region=auto,", inThread ) > 0 );
    assertTrue( "in a cfthread: the standard keys are still there (this.name)",
                find( ",appname=rcfml_settings_in_thread_fixture", inThread ) > 0 );
}

suiteEnd();
</cfscript>
