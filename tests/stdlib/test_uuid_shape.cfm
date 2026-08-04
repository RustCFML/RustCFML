<!---
  createUUID() / createUniqueID() shape — docs/known-issues.md §34.

  Two defects: the FIRST createUUID() in a process always came back with its
  first block zeroed (`00000000-CFC5-…`), and no UUID carried the RFC 4122
  version-4 nibble that Lucee's do. The zeroed block came from the PRNG's lazy
  seed being the bare nanosecond clock — `cfml_random() * u32::MAX` then equalled
  exactly the `nanos >> 32` that createUUID XORed it against, so the high word
  cancelled itself out.

  The "first call in a process" property cannot be observed from here (the suite
  has already drawn from the PRNG by the time this file runs); it is pinned in
  Rust instead, in cfml-stdlib's `uuid_tests`, which spawns a fresh thread to get
  a fresh thread-local PRNG. What this file pins is the shape and uniqueness, on
  both engines.
--->
<cfscript>
suiteBegin( "createUUID / createUniqueID shape (§34)" );

u = createUUID();

// CFML groups a UUID 8-4-4-16 (the standard 8-4-4-4-12 with the last two
// groups joined).
assert( "8-4-4-16 hex shape",
        reFind( "^[0-9A-Fa-f]{8}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{16}$", u ) > 0,
        true );
assert( "length is 35", len( u ), 35 );
assert( "isValid uuid", isValid( "uuid", u ), true );

// RFC 4122: version nibble 4 opens the third block, and the variant bits (10xx)
// make the fourth block open with 8, 9, A or B.
assert( "version nibble is 4", mid( u, 15, 1 ), "4" );
assert( "variant bits are 10xx",
        listFindNoCase( "8,9,A,B", mid( u, 20, 1 ) ) > 0,
        true );

// Every UUID in a batch is v4-shaped, unique, and none has a zeroed first block.
seen = {};
badVersion = 0;
badVariant = 0;
zeroPrefix = 0;
for ( i = 1; i <= 500; i++ ) {
    id = createUUID();
    if ( mid( id, 15, 1 ) != "4" ) { badVersion++; }
    if ( !listFindNoCase( "8,9,A,B", mid( id, 20, 1 ) ) ) { badVariant++; }
    if ( left( id, 8 ) == "00000000" ) { zeroPrefix++; }
    seen[ id ] = 1;
}
assert( "every uuid is version 4",   badVersion,           0 );
assert( "every uuid has 10xx variant", badVariant,         0 );
assert( "no uuid has a zeroed first block", zeroPrefix,    0 );
assert( "500 uuids are all distinct", structCount( seen ), 500 );

// createUniqueID shared the same construction, so its first bytes collapsed to
// zero the same way — base64'd, that showed up as a leading "AAAAA".
uid = createUniqueID();
assert( "createUniqueID is 22 chars", len( uid ), 22 );
assert( "createUniqueID is not zero-prefixed", left( uid, 5 ) == "AAAAA", false );

uids = {};
for ( i = 1; i <= 200; i++ ) { uids[ createUniqueID() ] = 1; }
assert( "200 unique ids are all distinct", structCount( uids ), 200 );

// createUniqueID("counter") is the separate monotonic form — unchanged here.
// Asserted only as "advances and stays short", because the two engines ENCODE
// the counter differently: Lucee base-36s it ("2q" then "2r") where RustCFML
// emits decimal ("1" then "2"). That encoding gap is its own divergence, not
// part of §34.
c1 = createUniqueID( "counter" );
c2 = createUniqueID( "counter" );
assert( "counter form advances", c1 != c2, true );
assert( "counter form is short", len( c2 ) <= 22, true );

suiteEnd();
</cfscript>
