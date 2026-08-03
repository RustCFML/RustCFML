<cfscript>
suiteBegin("Tags: cfinvoke webservice= fails loudly");

// `webservice` was not a reserved <cfinvoke> attribute, so it was passed along
// as a METHOD ARGUMENT while `component` resolved to "" — the call went nowhere
// with a confusing error, or worse (docs known-issues §27). RustCFML has no SOAP
// client; the tag now says so. NOTE: this is a RUNTIME throw, not a compile
// error, so an app that merely contains an unreached SOAP call still starts —
// Lucee compiles the tag happily and only fails on the call.
wsError = "";
</cfscript>

<cftry>
	<cfinvoke webservice="http://example.com/service?wsdl" method="doThing" returnvariable="wsResult">
	<cfcatch>
		<cfscript> wsError = cfcatch.message; </cfscript>
	</cfcatch>
</cftry>

<cfscript>
assertTrue( "cfinvoke webservice= raises an error", len( wsError ) > 0 );
// Both engines refuse; only the wording differs (Lucee reports a WSDL/SOAP
// failure for the unreachable endpoint), so assert only that the error names
// the mechanism rather than a missing component.
assertFalse( "cfinvoke webservice= does not report a missing component",
	wsError contains "component [] " );

suiteEnd();
</cfscript>
