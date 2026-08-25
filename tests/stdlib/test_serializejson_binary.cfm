<!--- GH #359 — serializeJSON( binary ) emits the base64 text, like Lucee. It
      used to fall into the serializer's `null` arm, so a struct carrying a
      binary member round-tripped through serializeJSON/deserializeJSON with the
      payload silently replaced by null: a cache write, a queue message or an API
      response lost the bytes with nothing thrown anywhere.

      base64 is also the shape that recovers — see the round-trip below.
      Measured on Lucee 7.1.0.204. --->
<cfscript>
suiteBegin("stdlib: serializeJSON of binary (GH ##359)");

_bin = binaryDecode( "QUJD", "base64" );   // "ABC"

assert("a bare binary serializes to its base64 text", serializeJSON(_bin), '"QUJD"');
assert("as a struct member", serializeJSON({ d = _bin }), '{"d":"QUJD"}');
assert("as an array element", serializeJSON([ _bin ]), '["QUJD"]');

// The round trip recovers the original bytes.
_back = binaryDecode( deserializeJSON( serializeJSON( _bin ) ), "base64" );
assert("binaryDecode(deserializeJSON(...)) recovers the bytes", toString(_back), "ABC");

// toString() always agreed; only the JSON serializer was affected.
assert("toString is unchanged", toString(_bin), "ABC");

suiteEnd();
</cfscript>
