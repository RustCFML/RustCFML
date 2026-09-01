<cfsilent>
<!---
  Named-lock contention workload for scripts/perf/concurrency_ab.py.

  The locked body is deliberately TINY. That is the whole point: the time a
  waiter spends acquiring the lock should be bounded by how long the holder
  holds it, so with a body this small, throughput at concurrency N should stay
  close to throughput at concurrency 1. If acquisition polls on a timer instead
  of blocking, latency quantises to the poll interval and throughput collapses
  as N rises — which is what GH #401 reports.

  `iterations` scales the held duration so the same page can measure both the
  near-zero-hold case and a realistic one.
--->
<cfscript>
	iterations = Val( url.iterations ?: 1 );
	if ( iterations < 1 ) {
		iterations = 1;
	}

	lock name="rustcfml_bench_lock" type="exclusive" timeout="30" {
		application.hits = ( application.hits ?: 0 ) + 1;
		total = 0;
		for ( i = 1; i <= iterations; i++ ) {
			total += i;
		}
	}
</cfscript>
</cfsilent>
<cfoutput>ok #application.hits# #total#</cfoutput>
