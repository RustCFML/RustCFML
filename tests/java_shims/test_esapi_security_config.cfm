<cfscript>
suiteBegin("java shim: OWASP ESAPI DefaultSecurityConfiguration");

// Preside's saml2-sso extension constructs ESAPI's default SecurityConfiguration
// at config time for one reason only: to read its class name into the
// org.owasp.esapi.SecurityConfiguration system property. Nothing on the object
// is ever called, so the shim only has to answer the reflection calls.
sysProp = "org.owasp.esapi.SecurityConfiguration";
config  = CreateObject( "java", "org.owasp.esapi.reference.DefaultSecurityConfiguration" );
sys     = CreateObject( "java", "java.lang.System" );

assert( "getClass().getName() is the fully-qualified class name"
      , config.getClass().getName()
      , "org.owasp.esapi.reference.DefaultSecurityConfiguration" );
assert( "getClass().getSimpleName() drops the package"
      , config.getClass().getSimpleName()
      , "DefaultSecurityConfiguration" );

// ...and the round trip the extension actually performs.
if ( IsNull( sys.getProperty( sysProp ) ) ) {
	sys.setProperty( sysProp, config.getClass().getName() );
}
assert( "the class name round-trips through a system property"
      , sys.getProperty( sysProp )
      , "org.owasp.esapi.reference.DefaultSecurityConfiguration" );

// A genuine ESAPI call has no implementation behind it and must say so loudly
// rather than hand back an empty value. (Lucee has the real class, so this is
// a RustCFML-only expectation.)
if ( isRustCFML() ) {
	assertThrows( "a real ESAPI method throws rather than silently no-opping"
	            , function() { config.getEncoder(); } );
}

suiteEnd();
</cfscript>
