<cfscript>
// Lucee strips redundant parentheses around a `( name = value )` call argument
// and treats it as a NAMED argument. Preside ships this idiom, e.g.
// system/views/renderers/asset/image/richEditor.cfm:
//   event.buildLink( ( assetId=args.id ?: "" ), derivative=( args.derivative ?: "" ) )
// RustCFML previously parsed the leading `(` as a positional parenthesized
// assignment-expression, so the call looked like mixed positional+named args
// and threw "all parameters must be named" — which 500'd the rich-editor
// image/widget preview render (assetManager.renderEmbeddedImageForEditor).
suiteBegin("Core: parenthesized named arguments");

function f( assetId="NONE", derivative="NONE" ) {
	return "assetId=" & assetId & ";derivative=" & derivative;
}

// A: the Preside idiom — first arg is a paren-wrapped named binding.
assert("( name=value ) is a named arg", f( ( assetId="X" ), derivative="Y" ), "assetId=X;derivative=Y");

// B: parens around the named-arg VALUE (always worked).
assert("name=( value ) still works", f( assetId=( "X" ), derivative=( "Y" ) ), "assetId=X;derivative=Y");

// C: paren-wrapped binding with an elvis inside (exact Preside value shape).
d = "SOMEID";
assert("( name=value ?: fallback ) unwraps to named", f( ( assetId=d ?: "" ), derivative=( "thumb" ) ), "assetId=SOMEID;derivative=thumb");

// D: a single paren-wrapped named arg.
assert("single ( name=value ) named arg", f( ( assetId="Z" ) ), "assetId=Z;derivative=NONE");

// E: genuine mixed positional+named must STILL error (Lucee parity).
assertThrows("genuine positional+named mix throws", function(){
	f( "X", derivative="Y" );
});

suiteEnd();
</cfscript>
