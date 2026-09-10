<cfscript>
// Target page for test_class_cache_across_requests.cfm — constructs a 3-level
// class ONCE and prints everything the class's cached construction tables
// decide. Request 1 builds the tables; requests 2+ adopt them from the server.
function fnNames( md ) { local.n = []; for ( local.f in md.functions ) arrayAppend( local.n, local.f.name ); arraySort( local.n, "textnocase" ); return arrayToList( local.n ); }
request.ctorsemLog = "";
leaf = new ctorsem.Leaf();
writeOutput( request.ctorsemLog
	& "|" & leaf.viaSuper()
	& "|" & leaf.rootMethod()
	& "|" & leaf.seenFromRoot()
	& "|" & fnNames( getMetadata( leaf ) )
	& "|" & fnNames( getComponentMetaData( "ctorsem.Leaf" ) )
	& "|" & listSort( leaf.ownKeys(), "textnocase" ) );
</cfscript>
