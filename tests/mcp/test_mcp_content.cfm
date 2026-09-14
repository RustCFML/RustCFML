<cfscript>
suiteBegin("MCP content builders and session BIFs");

// The MCP content builders shape an explicit content block when a tool wants
// exact control over what the client renders (the automatic coercion of a
// plain return value is covered by the Rust suites, which can see the wire).
// These BIFs are RustCFML-only, so the suite is gated on isRustCFML() to keep
// the cross-engine Lucee run clean — same convention as the WebSocket harness.
// Live protocol behaviour lives in crates/cli/tests/mcp_{stdio,http}.rs.
if ( isRustCFML() ) {

block = mcpText( "hello" );
assert( "mcpText type", block.type, "text" );
assert( "mcpText text", block.text, "hello" );

block = mcpImage( "AAAA", "image/png" );
assert( "mcpImage type", block.type, "image" );
assert( "mcpImage data", block.data, "AAAA" );
assert( "mcpImage mimeType", block.mimeType, "image/png" );

block = mcpAudio( "BBBB", "audio/wav" );
assert( "mcpAudio type", block.type, "audio" );
assert( "mcpAudio mimeType", block.mimeType, "audio/wav" );

// An omitted optional field is left OUT rather than written as null, so the
// block stays valid against the protocol's schema.
block = mcpImage( "CCCC" );
assert( "mcpImage type without a mime type", block.type, "image" );
assertFalse( "an omitted mimeType is absent, not null", structKeyExists( block, "mimeType" ) );

block = mcpResourceLink( "doc://pages/intro", "intro", "The introduction", "text/markdown" );
assert( "mcpResourceLink type", block.type, "resource_link" );
assert( "mcpResourceLink uri", block.uri, "doc://pages/intro" );
assert( "mcpResourceLink name", block.name, "intro" );
assert( "mcpResourceLink description", block.description, "The introduction" );
assert( "mcpResourceLink mimeType", block.mimeType, "text/markdown" );

block = mcpResource( { uri = "doc://x", mimeType = "text/plain", text = "body" } );
assert( "mcpResource type", block.type, "resource" );
assert( "mcpResource carries the resource struct", block.resource.uri, "doc://x" );

// mcpSessions() outside a server has nothing to report — an empty array, not
// an error, so a page can ask unconditionally.
sessions = mcpSessions( "/mcp/nothing" );
assertTrue( "mcpSessions returns an array", isArray( sessions ) );
assert( "no sessions outside serve mode", arrayLen( sessions ), 0 );

// mcp() is only meaningful inside an MCP handler, and says so rather than
// handing back a handle that silently does nothing.
assertThrows( "mcp() outside a handler throws", function() { mcp(); } );

// mcpNotify with no method is a programming error, not a silent no-op.
assertThrows( "mcpNotify() needs a method", function() {
    mcpNotify( server = "/mcp/demo" );
} );

// With a method it is safe to call from an ordinary page: nothing is
// listening, so it reports zero deliveries.
delivered = mcpNotify( server = "/mcp/demo", method = "notifications/custom", params = { a = 1 } );
assert( "no sessions means no deliveries", delivered, 0 );

// The client BIFs validate before doing anything expensive. (Live connections
// are covered by crates/cli/tests/mcp_client.rs, which drives a real
// two-process round trip.)
assertThrows( "an unknown transport is refused", function() {
    mcpConnect( transport = "carrier-pigeon", url = "x" );
} );
assertThrows( "a stdio connection needs a command", function() {
    mcpConnect( transport = "stdio" );
} );
assertThrows( "an http connection needs a url", function() {
    mcpConnect( transport = "http" );
} );
assertThrows( "mcpClient needs a name", function() { mcpClient(); } );

}

suiteEnd();
</cfscript>
