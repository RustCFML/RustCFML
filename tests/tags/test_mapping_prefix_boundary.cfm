<cfscript>
suiteBegin("Tags: a mapping only owns whole path segments");

// ============================================================
// Background
// ============================================================
// A mapping name must match on a path-SEGMENT boundary. A "/app" mapping owns
// "/app/x"; it does NOT own "/application/x", which merely starts with the same
// letters. RustCFML's expandPath compared the prefix with a bare starts_with,
// so the mapping target was spliced onto the wrong remainder:
//
//   this.mappings[ "/app" ] = "<webroot>/application"
//   expandPath( "/application/extensions/x" )
//     -> "<webroot>/application" + "lication/extensions/x"   WRONG
//     -> "<webroot>/application/extensions/x"                correct
//
// It surfaced in Preside (which declares exactly that "/app" mapping) as
// "The file [<webroot>/application/lication/extensions/.../timezones.jsonl]
// does not exist" — a path with a chewed-off segment, at boot.
//
// Here "/wheelsmap" (-> tests/oop/) is a strict prefix of BOTH the
// "/wheelsmapprobe" mapping (-> tests/tags/) and of that path's first segment,
// so a boundary-blind match resolves "/wheelsmapprobe/..." against the wrong
// target with a mangled remainder.
// ============================================================

// --- CONTROL (green on both engines): each mapping resolves its OWN path ---
assertTrue( "CONTROL: /wheelsmap resolves to its own target",
    directoryExists( expandPath( "/wheelsmap" ) ) );
assertTrue( "CONTROL: /wheelsmapprobe resolves to its own target",
    directoryExists( expandPath( "/wheelsmapprobe" ) ) );

// --- the gap: the shorter mapping must not claim the longer path ---
// tests/tags/test_mapping_include.cfm exists; tests/oop/ has no such file, and
// a spliced remainder ("probe/...") names nothing at all.
probeFile = expandPath( "/wheelsmapprobe/test_mapping_include.cfm" );
assertTrue( "a mapping whose name is a prefix of another does not claim it",
    fileExists( probeFile ) );
// A spliced remainder would land under the OTHER mapping's target (tests/oop/)
// with a chewed-off first segment.
assertFalse( "...and the resolved path did not land under the shorter mapping's target",
    probeFile contains "oop" );

// The same rule in the other direction: a path that merely STARTS with a
// mapping name, with no boundary, resolves webroot-relative instead.
assertFalse( "a longer segment is not claimed by a shorter mapping name",
    expandPath( "/wheelsmappery/x.cfm" ) contains "oop" );

suiteEnd();
</cfscript>
