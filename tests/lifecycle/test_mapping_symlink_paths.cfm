<cfscript>
suiteBegin("Mappings: ExpandPath and file BIFs agree through a symlinked mapping target");

// Regression (found booting Preside with /preside mapped to a symlinked checkout):
// resolving a leading-slash mapping for a file BIF ran the result through
// `canonicalize()`, which resolves symlinks. `ExpandPath` deliberately does NOT
// (it normalizes lexically — resolving there once broke every symlinked-extension
// admin asset). So for a mapping whose target is a symlink the two disagreed:
//
//   ExpandPath( "/m/" )        -> <site>/link/
//   DirectoryList( "/m" )[ 1 ] -> <site>/real/helper.cfm
//
// Preside's Config.cfc._getMappedPathFromFull does
// `Replace( fullPath, ExpandPath( mapping ), "" )` to turn each listed file back
// into a mapped path. With the prefixes disagreeing the Replace stripped nothing,
// and every UDF helper came out as "/preside/system/helpers/<absolute path>" —
// "Error loading UDF library". Both sides now normalize lexically.
//
// Cross-engine: Lucee builds directoryList results from the directory argument it
// was given and never canonicalizes, so it agrees with ExpandPath there too.

// Merge into the existing mapping set — `application action="update" mappings=`
// REPLACES it (Lucee parity), and dropping the runner's mappings would abort
// every later test file.
mappings = getApplicationMetadata().mappings ?: {};
// Anchor on this file's own directory, not a web-root-absolute path: the runner
// includes this file, and the two engines differ on what "/tests/..." resolves to.
mappings[ "/rustcfmlSymlinkTest" ] = getDirectoryFromPath( getCurrentTemplatePath() ) & "mapping_symlink/link";
application action="update" mappings=mappings;

expanded = expandPath( "/rustcfmlSymlinkTest/" );
listed   = directoryList( "/rustcfmlSymlinkTest", false, "path", "*.cfm" );

assert( "the mapping lists exactly the fixture file", arrayLen( listed ), 1 );

// The real assertion: a listed path starts with what ExpandPath reports for the
// same mapping, so a prefix-strip round-trips.
assertTrue(
	  "directoryList entries are prefixed by expandPath( mapping )"
	, arrayLen( listed ) && findNoCase( expanded, listed[ 1 ] ) == 1
);

// And the round-trip Preside actually performs.
assert(
	  "stripping expandPath( mapping ) off a listed entry yields the relative path"
	, arrayLen( listed ) ? replace( listed[ 1 ], expanded, "" ) : ""
	, "helper.cfm"
);

// fileExists must reach the file through the same mapping.
assertTrue( "fileExists through the symlinked mapping", fileExists( "/rustcfmlSymlinkTest/helper.cfm" ) );

suiteEnd();
</cfscript>
