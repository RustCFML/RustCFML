<cfscript>
// structAppend( target, defaults, overwrite=false ) fills a target key whose
// value is NULL, not just a missing one (Lucee 7.1). The arguments scope holds a
// null entry for every declared-but-omitted parameter, so this is how a
// framework's "fill in defaults" helper (Wheels `$args` -> structAppendDefaults)
// reaches an omitted parameter. Verified against Lucee 7.1.
suiteBegin("structAppend overwrite=false fills null-valued keys");

function fillDefaults( p, q ) {
    structAppend( arguments, { p = "D1", q = "D2", z = "D3" }, false );
    return { p = arguments.p, q = arguments.q, z = arguments.z, keys = structKeyList( arguments ) };
}
r = fillDefaults();
assert("an omitted param is filled from defaults",        r.p, "D1");
assert("a second omitted param is filled",                 r.q, "D2");
assert("a genuinely missing key is added",                 r.z, "D3");
assert("declared keys keep their order, new key appended", r.keys, "p,q,z");

function keepSupplied( p, q ) { structAppend( arguments, { p = "D1" }, false ); return arguments.p; }
assert("a SUPPLIED param is not overwritten", keepSupplied( "PV" ), "PV");

function viaDuplicate( p, q ) { var d = duplicate( arguments ); structAppend( d, { p = "D1" }, false ); return d.p; }
assert("duplicate(arguments) carries the null and it is still fillable", viaDuplicate(), "D1");

// The reverse direction copies the null through (Lucee does too).
function reverseAppend( p, q ) { var d = { p = "D1", q = "D2" }; structAppend( d, arguments, true ); return isNull( d.p ) && isNull( d.q ); }
assert("overwrite=true from the arguments scope copies null values", reverseAppend(), true);

// The end-to-end shape that broke Wheels: fill defaults, then forward to a
// function with a REQUIRED parameter of the same name.
function needsP( required any p ) { return "got:" & p; }
function fillThenForward( p ) { structAppend( arguments, { p = "FILLED" }, false ); return needsP( argumentCollection = arguments ); }
assert("a filled default satisfies a required param downstream", fillThenForward(), "got:FILLED");

suiteEnd();
</cfscript>
