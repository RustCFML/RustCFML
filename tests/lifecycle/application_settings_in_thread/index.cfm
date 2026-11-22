<cfscript>
// What getApplicationSettings() reports in the request, then inside a cfthread.
// A single pipe-delimited line, so the driver can assert on parts of it.
function describe( required struct settings ) {
    return "label=" & ( arguments.settings.projectLabel ?: "(missing)" )
         & ",host=" & ( arguments.settings.storage.host ?: "(missing)" )
         & ",region=" & ( arguments.settings.storage.region ?: "(missing)" )
         & ",appname=" & ( arguments.settings[ "name" ] ?: "(missing)" )
         & ",s3=" & s3UsesAppHost();
}

// Do the s3*() builtins see this.s3 from here? Presign with no inline settings
// and look for the application's host. This engine signs offline and returns
// the URL; Lucee contacts the host first and fails naming it. Either names the
// host, which is all this asks.
function s3UsesAppHost() {
    try {
        var seen = s3GeneratePresignedURL( bucketNameOrPath = "my-bucket", objectName = "a.txt" );
    } catch ( any e ) {
        seen = e.message & " " & ( e.detail ?: "" );
    }
    return find( "settings-in-thread-probe.example.com", seen ) ? "yes" : "no";
}

inRequest = describe( getApplicationSettings() );

thread name="settingsProbe" action="run" {
    thread.seen = describe( getApplicationSettings() );
}
thread action="join" name="settingsProbe" timeout="5000";

inThread = cfthread.settingsProbe.seen ?: ( "error: " & ( cfthread.settingsProbe.error.message ?: "no result" ) );

writeOutput( "request[" & inRequest & "]|thread[" & inThread & "]" );
</cfscript>
