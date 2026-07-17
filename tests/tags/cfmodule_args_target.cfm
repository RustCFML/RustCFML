<cfscript>
	// Mirrors ColdBox's RendererEncapsulator: a cfmodule template that reads the
	// CALLING function's `arguments` scope (unqualified `arguments.X`). On Lucee a
	// custom-tag/cfmodule template inherits the caller function's arguments scope.
	writeOutput( "[argVHP=" & ( arguments.viewHelperPath[ 1 ] ?: "" ) & "]" );
</cfscript>
