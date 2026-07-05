<cfscript>
suiteBegin("File BIFs: relative paths resolve against the BASE TEMPLATE (Lucee parity)");

// Verified on Lucee 7: a relative path passed to FileRead/FileExists (and
// ExpandPath) from inside a CFC resolves against the request's BASE TEMPLATE
// directory — the page that started the request — NOT the CFC's own directory.
// A component in tests/stdlib/relpath/ reading "./x" therefore reads the file
// next to the base template (the test runner), not a sibling of the CFC.
// (Supersedes GitHub #171, whose fix resolved against the CFC dir and diverged
// from Lucee — the reference engine.)

// Write the fixture next to the base template via ExpandPath (base-relative),
// then read it back through bare/dot-relative paths from a CFC in a subdir.
marker = '{"who":"base-template"}';
target = expandPath( "rel_bif_probe.json" );
fileWrite( target, marker );

try {
	reader = new relpath.Reader();

	assert(
		"FileRead('x') (bare relative) resolves against the base template",
		reader.readBareRelative(),
		marker
	);

	assert(
		"FileRead('./x') resolves against the base template",
		reader.readDotRelative(),
		marker
	);

	assertTrue(
		"FileExists('./x') uses the same base as FileRead",
		reader.existsRelative()
	);

	// The relative read must agree with the ExpandPath-wrapped read.
	assert(
		"FileRead('./x') agrees with FileRead(ExpandPath('./x'))",
		reader.readDotRelative(),
		reader.readViaExpandPath()
	);
} finally {
	if ( fileExists( target ) ) {
		fileDelete( target );
	}
}

suiteEnd();
</cfscript>
