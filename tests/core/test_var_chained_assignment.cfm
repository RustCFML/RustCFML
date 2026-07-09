<cfscript>
suiteBegin("var-declared chained assignment (Preside ObjectPicker)");

// `var x = y = expr` — the initialiser is itself an assignment (value position),
// so it must leave its assigned value on the stack for the var's store to
// consume. RustCFML previously stored `y` but left nothing, so `x` was never
// bound → "Variable 'x' is undefined". Surfaced in Preside's
// formcontrols/ObjectPicker.cfc:
//   var labelRenderer = args.labelRenderer = args.labelRenderer ?: <default>;
// which threw on the admin sitetree "add page" screen.

two = function() {
	var x = y = "lit";
	return "#x#|#y#";
};
assert( "var x = y = literal binds both", two(), "lit|lit" );

three = function() {
	var a = b = c = "v";
	return "#a#|#b#|#c#";
};
assert( "var a = b = c = literal binds the whole chain", three(), "v|v|v" );

// The exact Preside shape: var-declared, middle target is a struct member,
// RHS is an elvis over a missing key.
objectPicker = function( args={} ) {
	var labelRenderer = args.labelRenderer = args.labelRenderer ?: "defaultRenderer";
	return "#labelRenderer#|#args.labelRenderer#";
};
assert( "var x = struct.k = (struct.k ?: default) binds the local", objectPicker(), "defaultRenderer|defaultRenderer" );

// When the key IS present, the existing value wins and still threads through.
objectPickerPresent = function( args={} ) {
	var labelRenderer = args.labelRenderer = args.labelRenderer ?: "defaultRenderer";
	return labelRenderer;
};
assert( "present key value threads through the var chain", objectPickerPresent( { labelRenderer="custom" } ), "custom" );

// var-declared chain whose innermost RHS is itself an expression.
expr = function() {
	var t = u = 2 + 3;
	return t + u;
};
assert( "var chain over an arithmetic RHS binds both to the value", expr(), 10 );

suiteEnd();
</cfscript>
