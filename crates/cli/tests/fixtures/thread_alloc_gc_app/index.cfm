<cfscript>
// Regression fixture for the cycle collector's blind spot on cfthreads.
//
// ?step=holder  — create a PLAIN struct in application scope. Displacement from
//                 a plain struct is invisible to the relog mutation hook (only the
//                 scope struct itself is flagged persistent), so only the carried
//                 survivor set can ever reclaim what is dropped from it.
// ?step=build   — a cfthread allocates a cycle plus `n` nodes hanging off it and
//                 stores the lot under application.holder.gen. Every one of those
//                 allocations happens ON THE THREAD.
// ?step=drop    — structDelete(application.holder, "gen"): the whole graph is now
//                 garbage. A collector that never logged the thread's allocations
//                 cannot see it, and can never free the cycle at its root.
// ?step=dropscope — structDelete(application, "holder"): same graph, displaced
//                 from the persistent scope itself, so the relog hook fires and
//                 the DISPLACEMENT SWEEP (not the doubling budget) must free it.
// Used by ?step=runaway. A page-level UDF so every iteration of the runaway loop
// passes through a function frame, which is where the `--max-memory` hard tier
// polls for its abort.
function makeNode( n, pad ) {
    return { i = arguments.n, tags = [ 1, 2 ], pad = repeatString( "x", arguments.pad ) };
}

param name="url.step" default="noop";
param name="url.n"    default="600";

switch ( url.step ) {
    case "holder":
        application.holder = {};
        writeOutput( "holder" );
        break;
    case "build":
        thread name="builder" n=url.n {
            var a = {}; var b = {};
            a.b = b; b.a = a;                       // the cycle at the root
            var nodes = [];
            for ( var i = 1; i <= attributes.n; i++ ) {
                var node = { i = i, root = a };     // hangs off the cycle
                arrayAppend( nodes, node );
            }
            a.nodes = nodes;
            application.holder.gen = { root = a, nodes = nodes };
        }
        thread action="join" name="builder" timeout="30000";
        writeOutput( "built status=#cfthread.builder.status# keys=#structKeyList(application.holder.gen)#" );
        break;
    case "drop":
        structDelete( application.holder, "gen" );
        writeOutput( "dropped left=#structCount(application.holder)#" );
        break;
    case "dropscope":
        // Displace from the persistent scope itself: this is the path the relog
        // hook watches, and a generation-sized displacement there must trigger
        // the collector's displacement sweep at this request's end without any
        // RUSTCFML_GC_PERSISTENT_ALWAYS help.
        structDelete( application, "holder" );
        writeOutput( "dropped-scope has=#structKeyExists(application, 'holder')#" );
        break;
    case "runaway":
        // --max-memory HARD tier: allocate tracked containers without bound and
        // KEEP them, so nothing in the engine can end this request — only the
        // watchdog's abort. Each iteration goes through a user-function call,
        // which is where the abort is polled, and each node is a struct plus an
        // array (two tracked containers) so the request clears the victim floor
        // long before it reaches the limit. The loop bound is finite only so a
        // build where the hard tier does NOT fire fails the test instead of
        // hanging.
        param name="url.pad" default="2048";
        keep = [];
        for ( i = 1; i <= 10000000; i++ ) {
            arrayAppend( keep, makeNode( i, url.pad ) );
        }
        writeOutput( "runaway completed unaborted len=#arrayLen(keep)#" );
        break;
    case "hog":
        // --max-memory test: hold ~mb megabytes of LOCAL data for holdms, then
        // let it go at request end. Strings, not structs, so the collector has
        // nothing to do with it — this is pure footprint.
        param name="url.mb"     default="200";
        param name="url.holdms" default="0";
        chunk = repeatString( "x", 1024 * 1024 );
        hoard = [];
        for ( i = 1; i <= url.mb; i++ ) { arrayAppend( hoard, chunk & i ); }
        if ( url.holdms > 0 ) { sleep( url.holdms ); }
        writeOutput( "hogged #arrayLen(hoard)#MB" );
        break;
    default:
        writeOutput( "noop" );
}
</cfscript>
