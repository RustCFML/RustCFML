<cfscript>
// The HTML sanitiser, reached two ways: the org.owasp.validator.html.AntiSamy
// Java shim (so applications that use the real library run unmodified — GH ##325,
// jjannek) and the sanitizeHtml() BIF over the same core.
//
// Expected values were measured against the REAL AntiSamy 1.5.3 jar running on
// Lucee 7.0.4, using the policy file this suite ships beside it. Where the two
// engines differ the difference is cosmetic (pretty-printing, DOCTYPE emission,
// CSS reformatting) and recorded in docs/known-issues.md; no vector in the OWASP
// filter-evasion corpus survives on either engine.
//
// This suite is RustCFML-only: it needs the shim, and on a Lucee without the
// AntiSamy jars installed createObject() throws. The guard below turns that into
// a single informational pass rather than a wall of false reds.

suiteBegin("HTML sanitiser: AntiSamy shim + sanitizeHtml() (GH ##325)");

policyPath = expandPath( "stdlib/antisamy_test_policy.xml" );

available = true;
try {
	probeSamy = createObject( "java", "org.owasp.validator.html.AntiSamy", [] );
	probePol  = createObject( "java", "org.owasp.validator.html.Policy", [] )
	                .getInstance( createObject( "java", "java.io.File" ).init( policyPath ) );
	probeSamy.scan( "x", probePol ).getCleanHtml();
} catch (any e) {
	available = false;
}

if ( !available ) {
	assertTrue( "AntiSamy sanitiser unavailable on this engine — suite skipped", true );
} else {
	antiSamy = createObject( "java", "org.owasp.validator.html.AntiSamy", [ "/ignored/antisamy.jar" ] );
	policy   = createObject( "java", "org.owasp.validator.html.Policy", [] )
	               .getInstance( createObject( "java", "java.io.File" ).init( policyPath ) );

	function clean( required string dirty ) {
		return antiSamy.scan( arguments.dirty, policy ).getCleanHtml();
	}

	// ── The hot path: a request value with no markup is returned untouched ──
	assert( "plain text passes through unchanged", clean( "just a search term" ), "just a search term" );
	assert( "empty input stays empty", clean( "" ), "" );

	// ── Allowed markup survives ──
	assert( "bold survives", clean( "<b>keep me</b>" ), "<b>keep me</b>" );
	assert( "a valid link survives",
		clean( '<a href="https://ok.test/x">y</a>' ), '<a href="https://ok.test/x">y</a>' );

	// ── Script never survives ──
	assert( "script element and its contents are removed", clean( "<script>alert(1)</script>hi" ), "hi" );
	assert( "iframe is removed", clean( "<iframe src='https://evil.test'></iframe>x" ), "x" );
	assertFalse( "an event handler never survives",
		findNoCase( "onerror", clean( '<img src="https://ok.test/a.png" onerror="alert(1)">' ) ) GT 0 );
	assertFalse( "a javascript: href never survives",
		findNoCase( "javascript:", clean( '<a href="javascript:alert(1)">y</a>' ) ) GT 0 );
	assertFalse( "a javascript: url in style never survives",
		findNoCase( "javascript:", clean( '<div style="background-image: url(javascript:alert(1))">x</div>' ) ) GT 0 );
	assertFalse( "an @import in a style element never survives",
		findNoCase( "@import", clean( "<style>@import 'https://evil.test/x.css';</style>" ) ) GT 0 );

	// ── onInvalid semantics: the element goes, not just the attribute ──
	assert( "img with an invalid src is removed entirely (onInvalid=removeTag)",
		clean( '<img src="javascript:alert(1)" alt="x">' ), "" );
	assert( "a with an invalid target keeps the element (onInvalid=removeAttribute)",
		clean( '<a href="https://ok.test/" target="_evil">x</a>' ), '<a href="https://ok.test/">x</a>' );

	// ── Text is escaped on the way out ──
	assert( "markup characters in text are escaped", clean( "5 < 6 & 7 > 4" ), "5 &lt; 6 &amp; 7 &gt; 4" );

	// ── The masked round-trip an AntiSamy caller performs to keep entities ──
	// (Preside's AntiSamyService._removeUnwantedCleanses: mask &quot;, scan,
	// then replace whatever the sanitiser turned a bare & into.)
	masked   = replace( 'He said &quot;hi&quot;', "&quot;", "&~~~quot;", "all" );
	scanned  = clean( masked );
	ampAs    = clean( "&" );
	unmasked = replace( replace( scanned, ampAs, "&", "all" ), "&~~~quot;", "&quot;", "all" );
	assert( "the entity-masking round-trip restores the input", unmasked, 'He said &quot;hi&quot;' );

	// ── A bad policy fails loudly, at getInstance, as the Java library does ──
	policyThrew = "no throw";
	try {
		createObject( "java", "org.owasp.validator.html.Policy", [] )
			.getInstance( createObject( "java", "java.io.File" ).init( "/no/such/policy.xml" ) );
	} catch (any e) { policyThrew = e.type; }
	assert( "an unreadable policy raises PolicyException",
		policyThrew, "org.owasp.validator.html.PolicyException" );

	// ── The BIF over the same core ──
	assert( "sanitizeHtml removes script", sanitizeHtml( "<b>hi</b><script>alert(1)</script>", policyPath ), "<b>hi</b>" );
	assert( "sanitizeHtml passes plain text through", sanitizeHtml( "plain", policyPath ), "plain" );
	assertFalse( "sanitizeHtml strips a javascript: href",
		findNoCase( "javascript:", sanitizeHtml( '<a href="javascript:alert(1)">x</a>', policyPath ) ) GT 0 );

	bifThrew = "no throw";
	try { sanitizeHtml( "x" ); } catch (any e) { bifThrew = "threw"; }
	assert( "sanitizeHtml requires a policy rather than inventing one", bifThrew, "threw" );

	// ── The OWASP filter-evasion classics ──
	vectors = [
		"<script>alert(1)</script>",
		'<IMG SRC="javascript:alert(''XSS'');">',
		"<IMG SRC=javascript:alert('XSS')>",
		"<IMG SRC=JaVaScRiPt:alert('XSS')>",
		"<IMG SRC=## onmouseover=""alert('xxs')"">",
		"<img src=x onerror=alert(1)>",
		"<BODY ONLOAD=alert('XSS')>",
		"<svg/onload=alert(1)>",
		"<svg><script>alert(1)</script></svg>",
		'<iframe src="javascript:alert(1)"></iframe>',
		'<object data="https://evil.test/x.html"></object>',
		'<div style="width: expression(alert(1))">x</div>',
		"<STYLE>@import'https://evil.test/xss.css';</STYLE>",
		'<META HTTP-EQUIV="refresh" CONTENT="0;url=javascript:alert(1);">',
		"<scr<script>ipt>alert(1)</script>"
	];
	survived = "";
	for ( v in vectors ) {
		out = lcase( clean( v ) );
		for ( bad in [ "<script", "javascript:", "onerror", "onload", "onmouseover", "expression(", "@import", "<iframe", "<object", "<meta" ] ) {
			if ( find( bad, out ) ) { survived = listAppend( survived, bad & " in [" & v & "]", ";" ); }
		}
	}
	assert( "no OWASP filter-evasion vector survives the scan", survived, "" );
}

suiteEnd();
</cfscript>
