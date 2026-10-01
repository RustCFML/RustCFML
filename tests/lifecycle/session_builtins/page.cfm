<cfscript>
op = url.op ?: "";
if ( op == "write" ) { session.mine = "kept"; }
if ( op == "clear" ) { structClear( session ); }
r = { keys = listSort( structKeyList( session ), "textnocase" ), mine = session.mine ?: "" };
for ( k in [ "cfid", "cftoken", "sessionid", "urltoken", "timecreated", "lastvisit" ] ) {
	r[ k ] = session[ k ] ?: "";
}
r.timecreatedIsDate = isDate( r.timecreated );
r.lastvisitIsDate   = isDate( r.lastvisit );
r.app = getApplicationMetadata().name;
writeOutput( serializeJSON( r ) );
</cfscript>
