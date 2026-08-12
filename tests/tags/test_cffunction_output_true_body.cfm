<cfscript>
// Tag-based cffunction bodies have THREE output modes (all Lucee-measured,
// docker lucee/lucee:7.0):
//
//   output="true"   -> body is processed AS IF INSIDE CFOUTPUT: hash
//                      expressions interpolate and ## collapses to a literal
//                      #, with no explicit cfoutput anywhere. THE GAP:
//                      RustCFML emits these bodies raw.
//   output="false"  -> body text suppressed entirely; only the return value
//                      escapes. (Already matches.)
//   attr omitted    -> body text is emitted RAW — no interpolation, ## stays
//                      doubled. (Already matches; pinned so the output="true"
//                      fix doesn't overshoot into this mode.)
//
// Repro class: titan (Moopa) route CFCs render whole screens from
// output="true" function bodies with no explicit cfoutput — Lucee has always
// interpolated them. On RustCFML the sale-edit screen emitted a raw
// `#application.lib.auth.signedEndpoint(route="...")#` into an Alpine x-data
// attribute: the quote inside the un-evaluated expression terminated the
// attribute, spilled the rest of the component as page text, and every
// subsequent hash flipped between raw/literal — six SyntaxErrors and a dead
// page from one missing interpolation mode. 31 functions needed explicit
// cfoutput wraps to run.

suiteBegin("cffunction output=""true"": body is processed as if inside cfoutput (Lucee parity)");

// Resolution differs by harness: RustCFML resolves "tags.X" relative to the
// running template's directory (tests/); a plain-docker Lucee webroot resolves
// "tests.tags.X" from the repo root. Try the repo-convention spelling first,
// fall back to webroot-relative.
try {
    fx = createObject("component", "tags.OutputTrueBodyFixture");
} catch (any e) {
    fx = createObject("component", "tests.tags.OutputTrueBodyFixture");
}
</cfscript>

<cfsavecontent variable="implicitOut"><cfset fx.implicitBody(val=7) /></cfsavecontent>
<cfsavecontent variable="explicitOut"><cfset fx.explicitBody(val=7) /></cfsavecontent>
<cfsavecontent variable="suppressedOut"><cfset suppressedRet = fx.suppressedBody(val=7) /></cfsavecontent>
<cfsavecontent variable="defaultOut"><cfset fx.defaultBody(val=7) /></cfsavecontent>

<cfscript>
// ── The gap: implicit interpolation in an output="true" body ──
assertTrue( "implicit: hash expression interpolates (saw: " & trim( implicitOut ) & ")",
    findNoCase( "VAL:7", implicitOut ) GT 0 );
assertFalse( "implicit: no raw ##arguments.val## reaches the output",
    findNoCase( "arguments.val", implicitOut ) GT 0 );
assertTrue( "implicit: ## collapses to a single literal hash",
    find( "ESC:[#chr(35)#]", implicitOut ) GT 0 AND find( "ESC:[#chr(35)##chr(35)#]", implicitOut ) EQ 0 );
assertTrue( "implicit: ## collapses inside a JS template literal too",
    find( "TICK:[`Job #chr(35)#${js}`]", implicitOut ) GT 0 );

// ── Control: the same body with an explicit cfoutput (works today) ──
assertTrue( "explicit-cfoutput control interpolates", findNoCase( "EXPLICIT VAL:7", explicitOut ) GT 0 );
assertTrue( "explicit-cfoutput control collapses ##",
    find( "ESC:[#chr(35)#]", explicitOut ) GT 0 );

// ── Control: output="false" suppresses the body, return value intact ──
assert( "output=false: body text fully suppressed", trim( suppressedOut ), "" );
assert( "output=false: return value unaffected", suppressedRet, "ret:7" );

// ── Pinned third state: omitted attribute emits the body RAW ──
assertTrue( "omitted attr: body emits with hashes UNinterpolated",
    findNoCase( "VAL:#chr(35)#arguments.val#chr(35)#", defaultOut ) GT 0 );
assertTrue( "omitted attr: ## stays doubled",
    find( "ESC:[#chr(35)##chr(35)#]", defaultOut ) GT 0 );

suiteEnd();
</cfscript>
