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
    default:
        writeOutput( "noop" );
}
</cfscript>
