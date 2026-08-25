<cfscript>
suiteBegin("java shim: Lucee OSGi bundle plumbing is inert");

// A CFML library that ships its own jars installs an OSGi bundle and then names
// it when constructing a class. There is no OSGi container here, so the shim
// exists purely so that ceremony completes and the REAL request — the
// createObject for the class — is reached and answered on its own merits.

osgiUtil = CreateObject( "java", "lucee.runtime.osgi.OSGiUtil" );
engine   = CreateObject( "java", "lucee.loader.engine.CFMLEngineFactory" ).getInstance();

// Every bundle reports as already loaded, so callers skip installBundle and
// never build a Resource from a jar that will not be read.
bundle = osgiUtil.getBundleLoaded( "spreadsheet-cfml", osgiUtil.toVersion( "5.0.0.3" ), javaCast( "null", "" ) );
assertFalse( "getBundleLoaded reports a bundle rather than null", isNull( bundle ) );
assert( "toVersion round-trips the version string", osgiUtil.toVersion( "5.0.0.3" ), "5.0.0.3" );

// The install path still works for a caller that checks nothing.
resource = engine.getResourceUtil().toResourceExisting( getPageContext(), "/no/such/lib.jar" );
assertFalse( "a resource handle is produced", isNull( resource ) );
osgiUtil.installBundle( engine.getBundleContext(), resource, javaCast( "boolean", true ) );
assertTrue( "installBundle completes without error", true );

bundle.uninstall();
assertTrue( "uninstall completes without error", true );

// And the point of all of it: the class request afterwards is answered on its
// own merits. A class the engine models natively works...
poi = CreateObject( "java", "org.apache.poi.xssf.usermodel.XSSFWorkbook", "spreadsheet-cfml", "5.0.0.3" ).init();
poi.createSheet( "Sheet1" );
assert( "a bundled class the engine models is constructed", poi.getNumberOfSheets(), 1 );

// ...and one it does not still fails loudly, naming itself. The inert loader
// must not turn "no such class" into a silent success.
missingErr = "";
try {
	CreateObject( "java", "com.example.NotAThing", "spreadsheet-cfml", "5.0.0.3" );
} catch ( any e ) {
	missingErr = e.message;
}
assertTrue( "an unmodelled bundled class still fails, by name"
          , findNoCase( "com.example.NotAThing", missingErr ) > 0 );

// A method the inert shim does not model is refused rather than no-op'd.
assertThrows( "an unmodelled OSGi method throws", function(){ osgiUtil.getBundleFile(); } );

suiteEnd();
</cfscript>
