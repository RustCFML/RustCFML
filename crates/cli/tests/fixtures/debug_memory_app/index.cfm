<!doctype html><html><body>
<cfscript>
widgets = [];
for ( k = 1; k <= 25; k++ ) {
	w = new Widget( k );
	arrayAppend( widgets, w );
	w.build();
}
application.held = [];
for ( k = 1; k <= 2000; k++ ) {
	arrayAppend( application.held, { k = k, s = repeatString( "h", 100 ) } );
}
writeOutput( "made " & arrayLen( widgets ) );
</cfscript>
</body></html>
