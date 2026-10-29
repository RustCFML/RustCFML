<cfscript>
suiteBegin("cfthread terminate stops every loop shape");

// A terminated thread must stop at its next loop back-edge whatever the loop
// shape. Only the backward `Jump` was polled, so a counted `for` (a fused
// step op) and a `do {} while` (a backward conditional jump) ran to the end
// after `thread action="terminate"`. Each loop below would run for minutes.

function runAndTerminate( required string name, required function body ) {
	var started = getTickCount();
	thread name=arguments.name action="run" body=arguments.body {
		attributes.body();
	}
	sleep( 100 );
	thread action="terminate" name=arguments.name;
	thread action="join" name=arguments.name timeout=5000;
	return { status: cfthread[ arguments.name ].status, ms: getTickCount() - started };
}

r = runAndTerminate( "br_term_for", function() {
	var n = 0;
	for ( var i = 1; i <= 2000000000; i++ ) { n++; }
} );
assert( "counted for loop: thread terminated", r.status, "TERMINATED" );
assertTrue( "counted for loop: stopped promptly", r.ms < 4000 );

r = runAndTerminate( "br_term_dowhile", function() {
	var n = 0;
	do { n++; } while ( n < 2000000000 );
} );
assert( "do-while loop: thread terminated", r.status, "TERMINATED" );
assertTrue( "do-while loop: stopped promptly", r.ms < 4000 );

r = runAndTerminate( "br_term_while", function() {
	var n = 0;
	while ( n < 2000000000 ) { n++; }
} );
assert( "while loop: thread terminated", r.status, "TERMINATED" );
assertTrue( "while loop: stopped promptly", r.ms < 4000 );

suiteEnd();
</cfscript>
