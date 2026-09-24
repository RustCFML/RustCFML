<cfscript>
// GH ##430 follow-up: `cfadmin` is Lucee's Administrator API tag. RustCFML has no
// Administrator, but most of what real code asks it for is the engine reporting
// its OWN configuration, which we do hold. Shimmed onto `debugging` (cfconfig)
// and the per-application datasource registry.
//
// Cross-engine note: this file is RustCFML-only in the runner, because on Lucee
// every action below talks to a real Administrator and needs its password.
suiteBegin( "cfadmin shim (GH ##430 follow-up)" );

// ---- the script STATEMENT form now runs, and writes returnVariable ----------
// This is the shape Preside and the performance-analyser extension use. It used
// to parse as a bare `admin` identifier plus assignments: silent, and it left
// the attribute names behind as variables.
admin action="getDebug" returnVariable="dbg";
assert( "statement form populated returnVariable", isStruct( dbg ?: "" ), true );
assert( "statement form leaked no attribute variable", structKeyExists( variables, "returnVariable" ), false );
assert( "statement form leaked no action variable", structKeyExists( variables, "action" ), false );

// ---- attributeCollection, the shape LuceeAdminApiWrapper actually uses ------
attribs = { action="getDebug", type="web", returnVariable="dbg2" };
admin attributeCollection=attribs;
assert( "attributeCollection form populated returnVariable", isStruct( dbg2 ?: "" ), true );

// ---- the TAG form and the function form agree ------------------------------
cfadmin( action="getDebug", returnVariable="dbg3" );
assert( "function form populated returnVariable", isStruct( dbg3 ?: "" ), true );

// ---- getDebug reports our REAL state, in Lucee's key shape -----------------
// Reporting a convenient `true` here is what would send a caller on into Lucee
// debugger internals we do not have.
assert( "getDebug reports debug",          structKeyExists( dbg, "debug"          ), true );
assert( "getDebug reports database",       structKeyExists( dbg, "database"       ), true );
assert( "getDebug reports exception",      structKeyExists( dbg, "exception"      ), true );
assert( "getDebug reports tracing",        structKeyExists( dbg, "tracing"        ), true );
assert( "getDebug reports timer",          structKeyExists( dbg, "timer"          ), true );
assert( "getDebug reports implicitAccess", structKeyExists( dbg, "implicitAccess" ), true );
assert( "getDebug reports queryUsage",     structKeyExists( dbg, "queryUsage"     ), true );
assert( "getDebug reports dump",           structKeyExists( dbg, "dump"           ), true );
assert( "getDebug reports debugTemplate",  structKeyExists( dbg, "debugTemplate"  ), true );
assert( "debug is a boolean",              isBoolean( dbg.debug ), true );

// ---- updateDebug round-trips through the live config ----------------------
originalDebug = dbg.debug;
originalTimer = dbg.timer;

admin action="updateDebug" debug=true timer=false returnVariable="ignored";
admin action="getDebug" returnVariable="after";
assert( "updateDebug set debug",  after.debug, true  );
assert( "updateDebug set timer",  after.timer, false );

admin action="updateDebug" debug=false timer=true;
admin action="getDebug" returnVariable="after2";
assert( "updateDebug cleared debug", after2.debug, false );
assert( "updateDebug set timer back", after2.timer, true );

// An attribute that is not supplied is left alone rather than reset.
admin action="updateDebug" dump=true;
admin action="getDebug" returnVariable="after3";
assert( "an omitted flag is untouched", after3.debug, false );

// restore
admin action="updateDebug" debug=originalDebug timer=originalTimer;

// ---- updateDebugSetting maps maxLogs onto debugging.maxRecords -------------
admin action="updateDebugSetting" maxLogs=42;
admin action="getDebug" returnVariable="after4";
assert( "maxLogs round-trips", after4.maxLogs, 42 );

// ---- getDebugEntry: we have no registry of user debug templates ------------
admin action="getDebugEntry" returnVariable="entries";
assert( "getDebugEntry returns a list", isArray( entries ), true );
assert( "getDebugEntry is empty (no user templates)", arrayLen( entries ), 0 );

// ---- getCompilerSettings reports the runtime block -------------------------
admin action="getCompilerSettings" returnVariable="compiler";
assert( "getCompilerSettings is a struct", isStruct( compiler ), true );
assert( "reports dotNotationUpperCase", structKeyExists( compiler, "dotNotationUpperCase" ), true );
assert( "reports nullSupport",          structKeyExists( compiler, "nullSupport"          ), true );
assert( "reports templateCharset",      compiler.templateCharset, "UTF-8" );

// Preside's Bootstrap asks for exactly this, and it is already true here.
admin action="updateCompilerSettings" dotNotationUpperCase=false;
// ...but a setting we genuinely cannot honour is REFUSED, not reported as done.
assertThrows( "cannot turn key-uppercasing on", function(){
    admin action="updateCompilerSettings" dotNotationUpperCase=true;
} );

// ---- updateDatasource registers into the app datasource registry -----------
// This is the one that matters: Preside's env-injected datasource setup runs
// through it on every containerised deploy.
admin action        = "updateDatasource"
      type          = "web"
      name          = "__cfadmin_probe_ds"
      dbdriver      = "MySQL"
      host          = "127.0.0.1"
      port          = 3306
      database      = "probe_db"
      dbusername    = "probe_user"
      dbpassword    = "probe_pass";
// The registry is engine-internal; what we can assert portably is that the call
// was accepted and that a malformed one is refused.
assertThrows( "updateDatasource needs a name", function(){
    admin action="updateDatasource" dbdriver="MySQL" host="127.0.0.1" database="d";
} );

// ---- an action we cannot map THROWS, it does not return empty ---------------
assertThrows( "an unmapped action throws", function(){
    admin action="getMailServers" returnVariable="x";
} );
assertThrows( "a missing action throws", function(){
    admin returnVariable="x";
} );

// The error names the action and lists what IS supported.
unsupportedMessage = "";
try { admin action="getMailServers"; } catch( any e ) { unsupportedMessage = e.message; }
assert( "the refusal names the action", unsupportedMessage contains "getMailServers", true );
assert( "the refusal lists what works", unsupportedMessage contains "getDebug", true );

// ---- feature detection still works ------------------------------------------
// `LuceeAdminApiWrapper.canConnect()` is a try/catch around one call. A shimmed
// read action must let it succeed; an unmapped one must let it fail.
function canConnect( required string probeAction ) {
    try { evaluate( "__probe( probeAction )" ); return true; }
    catch( any e ) { return false; }
}
function __probe( required string probeAction ) {
    cfadmin( action=arguments.probeAction, returnVariable="tmp" );
}
assert( "canConnect() true for a supported action", canConnect( "getDebug" ), true );
assert( "canConnect() false for an unsupported one", canConnect( "getMailServers" ), false );

suiteEnd();
</cfscript>
