<cfscript>
// The bare-attribute script form `http url=… { … }` (issue #55).
//
// Its body was scanned for `httpparam` statements and every other token was
// skipped, so a `var`, loop or `if` around them never ran. A param built from
// a loop variable threw "variable [KEY] doesn't exist" — the shape API clients
// use to send a struct of fields:
//
//     http url=… { for ( var key in params ) { httpparam name=key value=params[ key ]; } }
//
// The body now runs as ordinary statements and each httpparam is collected at
// runtime, as the `cfhttp( … ) { }` form already did.
//
// Separately, `type="url"` and `type="formfield"` values were sent raw, so a
// value containing & = + or % split into extra fields or decoded differently.
// Lucee form-encodes them unless `encoded="false"`.

suiteBegin( "Tags: script http body runs as statements (issue ##55)" );

function sendFields( required struct fields ) {
	var seen = [];
	http url="http://127.0.0.1:1/" method="post" timeout="1" result="local.r" throwonerror=false {
		var prefix = "f_";
		httpparam type="header" name="X-Test" value="1";
		for ( var key in arguments.fields ) {
			ArrayAppend( seen, prefix & key );
			httpparam type="formfield" name=key value=arguments.fields[ key ];
		}
		if ( StructCount( arguments.fields ) > 1 ) {
			ArrayAppend( seen, "if-ran" );
		}
	}
	ArraySort( seen, "text" );
	return ArrayToList( seen );
}

loopError = "";
loopSeen  = "";
try {
	loopSeen = sendFields( { a = 1, b = 2 } );
} catch ( any e ) {
	loopError = e.message;
}
assert( "httpparam inside a for-in loop does not throw", loopError, "" );
assert( "body var, loop and if all ran", loopSeen, "f_a,f_b,if-ran" );

suiteEnd();

echoBase = "https://rustcfml-worker.rustcfml.workers.dev/echo";
http url="#echoBase#/request.cfm" method="GET" result="probeResult" timeout="20";
echoReachable = isStruct( probeResult ) && ( probeResult.status_code ?: 0 ) == 200;

suiteBegin( "Tags: cfhttpparam url and formfield values are encoded" );
</cfscript>

<cfif NOT echoReachable>
<cfscript>
	assertTrue( "cfhttp echo server unreachable — network tests skipped", true );
	suiteEnd();
</cfscript>
<cfelse>
<cfscript>
	tricky = "a&b=c+d e/?%";

	http url="#echoBase#/request.cfm" method="POST" result="formResult" {
		httpparam type="formfield" name="tricky" value=tricky;
		httpparam type="formfield" name="other" value="x";
	}
	formData = deserializeJSON( formResult.fileContent );
	assert( "formfield value with & = + % arrives intact", formData.form.tricky, tricky );
	assert( "the next formfield is not swallowed", formData.form.other, "x" );

	http url="#echoBase#/request.cfm" method="GET" result="urlResult" {
		httpparam type="url" name="tricky" value=tricky;
	}
	urlData = deserializeJSON( urlResult.fileContent );
	assert( "url param value with & = + % arrives intact", urlData.args.tricky, tricky );

	suiteEnd();
</cfscript>
</cfif>
