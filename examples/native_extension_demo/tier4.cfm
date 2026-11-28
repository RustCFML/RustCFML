<cfscript>
// Tier 4 — CFML errors as data, named arguments, and a closure kept for a
// later request.
//
//   rustcfml ext build examples/native_extension_demo
//   rustcfml ext install demo-0.1.0.rcx --dir examples/native_extension_demo/extensions
//   rustcfml --serve . --port 8500   → /tier4.cfm, then reload it

writeOutput( "— a callback that throws, caught by the extension —" & chr(10) );
lookup = function( city ) {
    if ( city == "Atlantis" ) {
        throw( message = "no data for " & city, type = "weather.unknownCity", detail = "try a real place" );
    }
    return city & ": 18C";
};
writeDump( demoTryApply( lookup, "Paris" ) );
// The request carries on: the extension turned the error into data.
writeDump( demoTryApply( lookup, "Atlantis" ) );

writeOutput( chr(10) & "— named arguments (argumentCollection from Rust) —" & chr(10) );
forecast = function( city, days = 1 ) { return city & " for " & days & " day(s)"; };
writeOutput( demoApplyNamed( forecast, { days = 3, city = "Oslo" } ) & chr(10) );
writeOutput( demoApplyNamed( forecast, { city = "Rome" } ) & chr(10) );

writeOutput( chr(10) & "— a closure kept by the extension, called in a LATER request —" & chr(10) );
// Kept once per application. Its captured `greeting` lives as long as the
// extension keeps it; `request` is whichever request calls it.
function makeGreeter( greeting ) {
    var calls = 0;
    return function( who ) {
        calls++;
        return greeting & ", " & who & " (call " & calls & ", request " & request.demoId & ")";
    };
}
if ( !structKeyExists( application, "demoKeptAt" ) ) {
    demoKeep( "greeter", makeGreeter( "Hello" ) );
    application.demoKeptAt = now();
}
request.demoId = createUUID();
writeOutput( demoCallKept( "greeter", "visitor" ) & chr(10) );
writeOutput( "(reload: the call count climbs, and the request id is always this one's)" & chr(10) );
</cfscript>
