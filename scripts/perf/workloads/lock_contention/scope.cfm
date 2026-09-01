<cfsilent>
<!---
  The `scope=` counterpart to index.cfm.

  `scope="application"` resolves to ONE synthetic key per application
  (`scope_lock_key`), so unlike `name="user_#id#"` it cannot shard — every
  request in the application contends on the same lock. That makes it the worse
  case for GH #401, not a milder one.
--->
<cfscript>
	iterations = Val( url.iterations ?: 1 );
	if ( iterations < 1 ) {
		iterations = 1;
	}

	lock scope="application" type="exclusive" timeout="30" {
		application.hits = ( application.hits ?: 0 ) + 1;
		total = 0;
		for ( i = 1; i <= iterations; i++ ) {
			total += i;
		}
	}
</cfscript>
</cfsilent>
<cfoutput>ok #application.hits# #total#</cfoutput>
