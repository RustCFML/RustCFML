<cfscript>
// What getApplicationSettings() reports in the request, then inside a cfthread.
// A single pipe-delimited line, so the driver can assert on parts of it.
function describe( required struct settings ) {
    return "label=" & ( arguments.settings.projectLabel ?: "(missing)" )
         & ",host=" & ( arguments.settings.storage.host ?: "(missing)" )
         & ",region=" & ( arguments.settings.storage.region ?: "(missing)" )
         & ",appname=" & ( arguments.settings[ "name" ] ?: "(missing)" );
}

inRequest = describe( getApplicationSettings() );

thread name="settingsProbe" action="run" {
    thread.seen = describe( getApplicationSettings() );
}
thread action="join" name="settingsProbe" timeout="5000";

inThread = cfthread.settingsProbe.seen ?: ( "error: " & ( cfthread.settingsProbe.error.message ?: "no result" ) );

writeOutput( "request[" & inRequest & "]|thread[" & inThread & "]" );
</cfscript>
