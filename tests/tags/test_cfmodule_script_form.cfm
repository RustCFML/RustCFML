<cfscript>
suiteBegin("cfmodule script form");

// Regression: the CFScript statement form `module template="x" attr=v;` (and the
// `module attributeCollection=args;` form Preside's Renderer uses to render EVERY
// view) previously parsed as a bare `module` identifier + `template=` assignment
// and silently produced NO output — so all Preside views rendered empty. The tag
// form <cfmodule> always worked. Both must emit the module template's output into
// the current (capturable) output buffer.

// Fixture tests/tags/cfmodule_target.cfm outputs: [out foo=#attributes.foo#]

// --- script form: bare, output must reach the buffer ---
savecontent variable="scriptBare" {
	module template="cfmodule_target.cfm" foo="SCRIPT";
}
assert( "script module emits output", trim( scriptBare ), "[out foo=SCRIPT]" );

// --- script form inside savecontent must be CAPTURED (the Preside pattern) ---
assertTrue( "savecontent captured the module output", len( scriptBare ) GT 0 );

// --- attributeCollection form (Preside's exact renderViewComposite pattern) ---
moduleArgs = { template="cfmodule_target.cfm", foo="AC" };
savecontent variable="acOut" {
	module attributeCollection=moduleArgs;
}
assert( "module attributeCollection resolves template + attrs", trim( acOut ), "[out foo=AC]" );

// --- template supplied via attributeCollection, extra attr merged on top ---
moduleArgs2 = { template="cfmodule_target.cfm" };
savecontent variable="acOut2" {
	module attributeCollection=moduleArgs2 foo="MERGED";
}
assert( "attributeCollection merges with inline attrs", trim( acOut2 ), "[out foo=MERGED]" );

// --- function-call form `cfmodule( template=…, attr=… )` (ColdBox's
// Renderer.renderViewComposite calls cfmodule() this way). Must resolve the
// same as the tag/statement forms; previously threw "Variable 'cfmodule' is
// undefined" because the cf-prefixed tag function wasn't routed to __cfmodule. ---
savecontent variable="fnOut" {
	cfmodule( template="cfmodule_target.cfm", foo="FUNC" );
}
assert( "cfmodule() function-call form emits output", trim( fnOut ), "[out foo=FUNC]" );

// --- a cfmodule template inherits the CALLING function's `arguments` scope
// (Lucee semantics). ColdBox's RendererEncapsulator reads
// `arguments.viewHelperPath` expecting the enclosing Renderer method's args. ---
string function renderViaModule( required array viewHelperPath ){
	var captured = "";
	savecontent variable="captured" {
		cfmodule( template="cfmodule_args_target.cfm" );
	}
	return captured;
}
assert(
	"cfmodule template sees caller's arguments scope",
	trim( renderViaModule( viewHelperPath = [ "CALLERARG" ] ) ),
	"[argVHP=CALLERARG]"
);

// --- String.isEmpty() member function returns a real boolean (was Null, which
// is falsy — broke ColdBox's `if ( !thisAllowedMethods.isEmpty() )` guard and
// 405'd every default GET request). ---
assertTrue( "empty string .isEmpty() is true", "".isEmpty() );
assertFalse( "non-empty string .isEmpty() is false", "GET".isEmpty() );
assertTrue( "String.isEmpty() returns a boolean", isBoolean( "".isEmpty() ) );

suiteEnd();
</cfscript>
