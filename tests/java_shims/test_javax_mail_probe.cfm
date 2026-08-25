<cfscript>
suiteBegin("java shim: javax.mail.Session connection probe");

// Preside's EmailService.validateConnectionSettings() — the "verify these SMTP
// settings" button — builds a JavaMail Session and connects without sending.
// CFML has no BIF for that (<cfmail> must send a message to find out), so this
// shim is backed by the new smtpConnectionTest() builtin.
//
// Deterministic without a mail server: port 1 on loopback refuses. What is being
// asserted is the EXCEPTION TYPING — a caller branches on
// javax.mail.AuthenticationFailedException vs anything else to tell the user
// whether it is their password or their host that is wrong.

function validateConnectionSettings( host, port, username="", password="" ) {
	var errorMessage = "";
	var props        = CreateObject( "java", "java.util.Properties" ).init();

	props.put( "mail.smtp.starttls.enable", "true" );
	props.put( "mail.smtp.auth", "true" );
	props.put( "mail.smtp.connectiontimeout", "3000" );

	var mailSession = CreateObject( "java", "javax.mail.Session" ).getInstance( props, NullValue() );
	var transport   = mailSession.getTransport( "smtp" );

	try {
		transport.connect( arguments.host, arguments.port, arguments.username, arguments.password );
	} catch ( "javax.mail.AuthenticationFailedException" e ) {
		errorMessage = "authentication failure";
	} catch( any e ) {
		errorMessage = e.message;
	} finally {
		// Always runs, including after a failed connect — close() must not throw
		// on a transport that never opened.
		transport.close();
	}

	return errorMessage;
}

msg = validateConnectionSettings( "127.0.0.1", 1, "u", "p" );
assertTrue( "an unreachable host reports a message rather than throwing past the caller", len( msg ) > 0 );
assert( "and it is NOT misreported as an auth failure", msg == "authentication failure", false );
assertTrue( "the message names the connection problem", findNoCase( "MessagingException", msg ) || findNoCase( "refused", msg ) || findNoCase( "connect", msg ) );

// getTransport only speaks SMTP.
protoErr = "";
try {
	CreateObject( "java", "javax.mail.Session" ).getInstance( CreateObject( "java", "java.util.Properties" ).init() ).getTransport( "imap" );
} catch ( any e ) { protoErr = e.type; }
assert( "a non-SMTP protocol raises MessagingException", protoErr, "javax.mail.MessagingException" );

// Sending is deliberately not shimmed — <cfmail> is the supported path.
sendErr = "";
try {
	CreateObject( "java", "javax.mail.Session" ).getInstance().getTransport( "smtp" ).sendMessage( "x" );
} catch ( any e ) { sendErr = e.type; }
assert( "Transport.sendMessage is refused, not faked", sendErr, "java.lang.UnsupportedOperationException" );

// ---- the smtpConnectionTest() builtin directly ----------------------------
res = smtpConnectionTest( "127.0.0.1", 1, "", "", false, false, 2 );
assertFalse( "the probe reports failure for a refused connection", res.success );
assertFalse( "and does not claim an auth failure", res.authFailed );
assertTrue( "and carries a message", len( res.message ) > 0 );
assertThrows( "a host is required", function(){ smtpConnectionTest( "" ); } );

suiteEnd();
</cfscript>
