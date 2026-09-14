# MCP (Model Context Protocol)

RustCFML speaks [MCP](https://modelcontextprotocol.io) natively: a CFC becomes
an MCP server that Claude, Claude Code, or any other agent runtime can discover
and call — served from the **same process and port** as your HTTP traffic, or
launched as a subprocess over stdio. No sidecar, no SDK, no separate runtime.

You write CFML functions. The engine derives the JSON Schema, shapes the
results, and speaks the protocol.

> **Status: phase 6.** RustCFML works in both directions — a CFC is an MCP
> server (tools, resources, prompts, SSE streaming, sampling), and CFML can
> call out to remote MCP servers — with bearer-token authentication, address
> rules and tool filtering. See [What's not here yet](#whats-not-here-yet)
> for the remainder.

## Quick start

Create `mcp/docs.cfc` under your web root:

```cfml
component mcp="docs" version="1.0.0" description="Documentation tools" {

    /**
     * @query.description Free-text search terms
     * @limit.description Maximum rows to return
     */
    function searchDocs( required string query, numeric limit = 10 )
        tool        = "search_docs"
        description = "Search the documentation"
        readOnly    = true
    {
        return queryExecute(
              "select title, url from docs where title like :q limit :n"
            , { q = "%#arguments.query#%", n = arguments.limit }
        );
    }
}
```

That is the whole server. Point a client at it either way:

```bash
# stdio — the client launches the engine as a subprocess
rustcfml mcp docs /path/to/webroot

# HTTP — served alongside your app
rustcfml --serve /path/to/webroot     # → http://localhost:8500/mcp/docs
```

This works the same in a `--build` self-contained binary — `myapp mcp docs`
serves the MCP server embedded in it, which is how a client launches one.

In a client's config file:

```jsonc
{
  "mcpServers": {
    "docs": { "command": "rustcfml", "args": ["mcp", "docs", "/path/to/webroot"] },
    // A bundled app carries its own server:
    "bundled": { "command": "/path/to/myapp", "args": ["mcp", "docs"] },
    // …or, against a running server:
    "docs-http": { "url": "http://localhost:8500/mcp/docs" }
  }
}
```

## Discovery

- One CFC under `<webroot>/mcp/` is one MCP server. `mcp/docs.cfc` is reachable
  at `/mcp/docs` over HTTP and as `rustcfml mcp docs` over stdio.
- `component mcp="docs"` names the server; without it the filename is used.
  `version` and `description` are reported to the client at handshake
  (`description` becomes the server's `instructions`).
- The `/mcp/{name}` routes are registered **only** when an `mcp/` directory
  exists, and a URL that names no server CFC falls through to your application
  — so a page at `/mcp/anything.cfm` keeps working.

## Declaring tools

A method becomes a tool by annotation. Only `public` and `remote` methods are
eligible — a `private` helper annotated `tool=` stays unreachable.

| Annotation | Effect |
|---|---|
| `tool="name"` | Exposes the method under that MCP tool name. |
| `description="…"` | Shown to the model. Write it for a reader who cannot see your code. |
| `title="…"` | Optional human-facing label. |
| `readOnly` / `destructive` / `idempotent` / `openWorld` | Behaviour hints; clients use them to decide how much confirmation to ask for. |
| `secured` / `secured="admin,ops"` | Authorization gate — same contract as a [WebSocket handler](websockets.md#authorization). |
| `streaming` | Run this tool on an SSE stream so it can log and report progress even when the client sent no progress token. |
| `inputSchema="{…}"` / `outputSchema="{…}"` | Raw JSON Schema, overriding what is derived. |

### The schema is derived, not written

Hand-writing JSON Schema is the part of authoring an MCP server that everyone
gets wrong, so the engine reads it off the CFML signature:

```cfml
function searchDocs( required string query, numeric limit = 10 ) tool="search_docs"
```

becomes

```jsonc
{
  "type": "object",
  "properties": {
    "query": { "type": "string", "description": "Free-text search terms" },
    "limit": { "type": "number", "default": 10, "description": "Maximum rows to return" }
  },
  "required": [ "query" ]
}
```

- CFML types map to JSON types; `date` becomes a `date-time`-formatted string,
  and `any` (or an undeclared type) emits no `type` at all rather than guessing.
- A parameter is `required` only when CFML says so **and** it has no default —
  a defaulted parameter is always optional to the caller.
- Descriptions come from the `@param.description` doc-comment annotation, or
  from a `hint` attribute.

Reach for an explicit `inputSchema=` only when you need something CFML's type
system cannot express, such as an enum or a nested object.

## Returning results

Return whatever is natural; the engine shapes it.

| You return | The client receives |
|---|---|
| a string, number, boolean | one `text` content block |
| a struct, array or query | `structuredContent`, plus the serialized JSON as a `text` block |
| binary | an embedded resource with base64 `blob` |
| `null` | an empty content list |

A query becomes an array of row objects — not Lucee's `{COLUMNS, DATA}`
envelope, which no MCP client understands.

Need exact control? Build the blocks yourself and they pass through untouched:

```cfml
return [ { type = "text", text = "Here is the chart:" }
       , { type = "image", data = toBase64( chart ), mimeType = "image/png" } ];
```

### Errors

**Throwing is the right way to report a tool failure.** The exception message
comes back as `isError: true` with the text, so the model can read it and adapt:

```cfml
function deploy( required string env ) tool="deploy" {
    if ( !listFindNoCase( "dev,staging", arguments.env ) ) {
        throw( type = "BadEnvironment", message = "Unknown environment [#arguments.env#]" );
    }
    ...
}
```

Stack traces are never sent to a client — they name CFC paths and line numbers,
and everything in an MCP response is on its way to a remote caller. A `secured`
refusal is reported differently, as a JSON-RPC error, so the model is told "you
may not" rather than "that failed, try again".

## Resources

A resource is addressable content the model can read: a document, a config
file, a record. Annotate a method with the URI it serves.

```cfml
function readme() resource="doc://readme" mimeType="text/markdown" {
    return fileRead( expandPath( "/README.md" ) );
}
```

The engine attaches the envelope, so a handler returns just the content: a
string becomes `text`, binary becomes a base64 `blob`, and a struct or array is
serialized as `application/json`. The `mimeType` annotation overrides the
default in each case.

### URI templates

A URI containing `{placeholders}` is a **template**, and the placeholders are
bound as arguments:

```cfml
/**
 * @slug.description Which page to read
 */
function page( string slug ) resource="doc://pages/{slug}" mimeType="text/plain" {
    return fileRead( expandPath( "/pages/#arguments.slug#.txt" ) );
}
```

Reading `doc://pages/intro` calls `page( slug = "intro" )`. Declare a `uri`
parameter as well if the handler wants the full URI it was asked for.

A placeholder never matches across `/`, so `doc://pages/{slug}` will not
swallow `doc://pages/deep/path` — it reports an unknown resource instead of
quietly serving the wrong thing. Templates are listed by
`resources/templates/list`, not `resources/list`, because a template is not
itself a readable URI.

Literal URIs are matched exactly and take precedence over templates, so a
concrete resource is never shadowed by a template that happens to cover it.

## Prompts

A prompt is a reusable conversation starter the user picks from a menu.

```cfml
/**
 * @language.description The language being reviewed
 */
function review( string language = "CFML" ) prompt="code_review" description="Ask for a review" {
    return [ { role = "assistant", content = "I review #arguments.language# code." }
           , { role = "user",      content = "Please review my #arguments.language#." } ];
}
```

Return an array of `{ role, content }` structs, or a bare string for a
single-message prompt. `content` may be plain text — it is wrapped into a
content block for you — and `role` defaults to `user`.

Prompt arguments are described to the client as a flat list (name, description,
required), not as JSON Schema; that is what the protocol asks for, and the
engine derives it from the signature exactly as it derives a tool's schema.

## Talking back mid-call

A long-running tool does not have to be silent. Inside a handler, `mcp()` is a
live handle on the call:

```cfml
function reindex( numeric batches = 10 ) tool="reindex" streaming=true {
    for ( var i = 1; i <= arguments.batches; i++ ) {
        indexBatch( i );
        mcp().progress( i, arguments.batches, "batch #i# of #arguments.batches#" );
        mcp().log( "info", "indexed batch #i#" );
    }
    return "reindexed #arguments.batches# batches";
}
```

| Method | Effect |
|---|---|
| `mcp().progress( done [, total] [, message] )` | `notifications/progress` on this call's stream. |
| `mcp().log( level, data [, logger] )` | `notifications/message` — structured logging into the client's console. Filtered by whatever the client asked for with `logging/setLevel`. |
| `mcp().session()` / `.server()` | Identifiers for the conversation and the server. |
| `mcp().client()` | The client's `initialize` details and its capabilities. |
| `mcp().streaming()` | Whether this call actually has a stream. |
| `mcp().toolsChanged()` | `notifications/tools/list_changed`, telling the client to re-read the tool list. |
| `mcp().notify( method [, params] )` | Any other notification. |

Each of these returns a boolean saying whether it was delivered, so a tool can
tell when nobody is listening rather than guessing.

### When does a call get a stream?

The response mode has to be chosen **before** the handler runs, so it is decided
from two things that are known in advance:

1. the client attached a `_meta.progressToken` to the request, or
2. the tool declares `streaming`.

…and in both cases only if the client's `Accept` header includes
`text/event-stream`. Otherwise the call is answered with a single JSON response.
`mcp().progress()` on a call with no stream returns `false` rather than
vanishing — and note that progress is **only** legal when the client sent a
token, because the spec forbids unsolicited progress notifications.

## Asking the client

A tool can turn the conversation around and ask the client something mid-call.

```cfml
function summarise( required string text ) tool="summarise" streaming=true {
    var answer = mcp().sample(
          messages = [ { role = "user", content = "Summarise: #arguments.text#" } ]
        , options  = { maxTokens = 200, systemPrompt = "Be concise." }
    );
    return answer.content.text;
}
```

| Method | Asks the client to… |
|---|---|
| `mcp().sample( messages [, options] )` | generate text with **its own** model — no API key needed on your side. |
| `mcp().elicit( message [, schema] [, timeout] )` | collect input from the user. |
| `mcp().roots( [timeout] )` | list the directories it has exposed to this server. |

`options` accepts `maxTokens` (default 1000), `systemPrompt`, `temperature`,
`stopSequences`, `modelPreferences`, `includeContext` and `timeout` (seconds).

These block the CFML thread until the client answers, which is why they come
with three guards — each one turning a hang into a clear error:

1. **The client must have declared the capability** at `initialize`. Asking a
   client that cannot answer fails immediately rather than waiting for a
   timeout.
2. **The handler must be `streaming=true`.** A server→client request has to
   travel on an SSE stream; if the call was answered as plain JSON there is
   nowhere to send it. The error says exactly this.
3. **The wait is always bounded** — 60 seconds by default. A parked request
   holds a thread from the same pool every ordinary CFML request draws on, so
   there is also a cap of four outstanding requests per session. Beyond it,
   further requests are refused rather than queued.

Design accordingly: a tool that samples is a tool that can fail because the
user closed their editor. Treat the result as you would any remote call.

## Notifying from anywhere

You do not need to be inside a handler to push to a connected client. Any page,
`cfthread` or background job can publish, which is the same emit-from-anywhere
ergonomic `wsPublish` gives realtime channels:

```cfml
mcpNotify( server = "/mcp/docs", method = "notifications/resources/updated"
         , params = { uri = "doc://pages/intro" } );

mcpSessions( "/mcp/docs" );            // live session ids
mcpNotify( server = "/mcp/docs", method = "notifications/announcement"
         , params = { text = "redeployed" }, to = oneSessionId );
```

`mcpNotify` returns how many sessions it reached. A client receives these on the
stream it opened with `GET /mcp/{name}`.

### Resumability

Every SSE event carries an id, and the engine retains a bounded ring of recent
events per stream. A client that drops off the network reconnects with the
standard `Last-Event-ID` header and receives exactly what it missed, in order,
before live traffic resumes. Replay never crosses streams — an id naming a
stream the server no longer holds simply starts fresh.

## Content helpers

For exact control over what the client renders:

```cfml
return [ mcpText( "Here is the chart:" )
       , mcpImage( toBase64( chart ), "image/png" )
       , mcpResourceLink( "doc://pages/intro", "intro", "The introduction", "text/markdown" ) ];
```

`mcpText`, `mcpImage`, `mcpAudio`, `mcpResource` and `mcpResourceLink` build
blocks that pass through the automatic coercion untouched. An omitted optional
field is left out rather than written as null.

## Calling other MCP servers

The other direction: CFML as an MCP *client*, so a page can use anybody's MCP
server as a library.

```cfml
// Launch a server as a subprocess…
client = mcpConnect(
      transport = "stdio"
    , command   = "npx -y @modelcontextprotocol/server-filesystem /tmp"
);

// …or talk to one over HTTP.
client = mcpConnect(
      transport = "http"
    , url       = "https://example.com/mcp"
    , headers   = { Authorization = "Bearer #token#" }
);

tools = client.tools();                                  // what it offers
rows  = client.call( "search", { query = "invoices" } );  // use it
client.close();
```

> **Name every argument.** CFML does not allow positional and named arguments
> in the same call, so `mcpConnect( "stdio", command = "…" )` is a syntax
> error. Either name them all (as above) or pass a struct:
> `mcpConnect( "stdio", { command = "…" } )`.

| Method | Does |
|---|---|
| `client.tools()` / `.resources()` / `.resourceTemplates()` / `.prompts()` | List what the server offers. |
| `client.call( name, arguments )` | Call a tool. |
| `client.read( uri )` | Read a resource; returns its `contents` array. |
| `client.prompt( name, arguments )` | Fetch a prompt's messages. |
| `client.ping()` / `.info()` / `.describe()` / `.isClosed()` | Health, handshake details, and state. |
| `client.request( method, params )` | Escape hatch for anything not wrapped. |
| `client.close()` | Shut the connection down. |

A few behaviours worth knowing:

- **The handshake runs at `mcpConnect()`**, so a bad command or URL fails where
  you wrote it rather than at the first `tools()` call somewhere else.
- **A tool that reports failure throws.** CFML's way of saying "this did not
  work" is an exception (`type = "MCPToolError"`), rather than handing back
  error text that reads like an answer.
- **A tool with structured output returns that data directly** — you get the
  struct, not the protocol envelope.
- **`close()` is not required but is polite.** A client dropped without it is
  closed when it is collected, so a page that throws cannot leak a subprocess.

### Named servers

Rather than repeating a command line, declare servers in `.cfconfig.json`
using the same `mcpServers` shape an editor's client config uses — so a block
can be pasted across without translation:

```jsonc
{
  "mcpServers": {
    "files":  { "command": "npx", "args": ["-y", "@modelcontextprotocol/server-filesystem", "/tmp"] },
    "remote": { "url": "https://example.com/mcp", "headers": { "Authorization": "Bearer …" } }
  }
}
```

```cfml
client = mcpClient( "files" );
```

This client declares **no** capabilities, so it will decline a server's
`sampling` or `elicitation` request rather than leaving the server parked
waiting for an answer it is never going to get.

## Securing an endpoint

An MCP server is a remote-control surface for your application, so the
defaults are closed: **only this machine may connect**, and `secured` handlers
are unreachable until tokens exist to authenticate against.

```jsonc
// .cfconfig.json
{
  "mcp": {
    "enabled": true,
    "allowedIPs": ["127.0.0.1", "::1", "10.0.0.0/8"],
    "authToken": [
      { "token": "admin-token", "roles": ["admin"] },
      { "token": "readonly-token", "roles": [], "includedTools": ["get_*", "search*"] }
    ],
    "excludedTools": ["delete_*"],
    "corsAllowedOrigins": ["https://*.example.com"],
    "stdioRoles": []
  }
}
```

| Setting | Default | Means |
|---|---|---|
| `enabled` | `true` | Off returns 404, as if the endpoint did not exist. |
| `allowedIPs` | `["127.0.0.1", "::1"]` | Addresses or CIDR ranges. **Empty means anywhere** — only sensible behind a proxy that authenticates for you. |
| `authToken` | none | A string, an array of strings, or objects with `roles` and per-token tool filters. With none configured nobody is authenticated, so `secured` handlers stay unreachable. |
| `includedTools` / `excludedTools` | `["*"]` / none | Globs (`*`, `?`). Exclusions apply after inclusions. A token's own filters replace the global ones. |
| `corsAllowedOrigins` | none | Browser origins beyond localhost, which is always allowed. |
| `stdioRoles` | none | Roles granted on the stdio transport. |

### How `secured` gets its identity

A token's `roles` become the identity a handler's `secured` annotation checks —
the same contract a WebSocket handler's `socket.data` uses:

```cfml
function purge() tool="purge" secured="admin" { … }   // needs the admin role
function whoami() tool="whoami" secured { … }         // any authenticated caller
```

On **stdio** the caller is whoever launched the process, so they are
authenticated by construction — but their roles come from `stdioRoles`, which
is empty by default. Running the binary therefore satisfies a bare `secured`
without silently conferring `admin`.

### What a caller may not use, it cannot see

A tool excluded by a filter is left out of `tools/list`, and calling it
directly is refused as **unknown** rather than forbidden — a caller should not
be able to map what it is not allowed to reach. Denials say only
`Not authorized`, so a probe cannot tell a wrong token from a blocked address.

> **These are static bearer tokens, not the spec's OAuth 2.1 flow.** A 401 is
> answered with `WWW-Authenticate: Bearer`, and some clients read that as an
> invitation to start an OAuth handshake; those need the header supplied
> directly (`"headers": { "Authorization": "Bearer …" }` in their config).

## Execution model

Each call runs on a **fresh VM**, exactly as a WebSocket frame does. Nothing
carries between tool calls except through `application` or `server` scope. That
is what makes tools safe to run concurrently — and it means a tool that needs
state should say so explicitly:

```cfml
function counter() tool="counter" {
    application.hits = ( application.hits ?: 0 ) + 1;
    return application.hits;
}
```

## Transports

### stdio

`rustcfml mcp <name> [webroot]` reads newline-delimited JSON-RPC on stdin and
writes it on stdout. The webroot defaults to the current directory.

The protocol requires that **nothing** but MCP messages reaches stdout, so on
unix the engine takes a private handle on the real stdout and points file
descriptor 1 at stderr. `writeOutput` from a tool, engine logging, a panic
message — all of it lands in the client's log instead of corrupting the stream.

### Streamable HTTP

`POST /mcp/{name}` carries client→server messages and answers with JSON.
`DELETE` ends a session. `GET` opens the standalone server→client stream for a
client that accepts `text/event-stream`, and answers `405` — the spec's
sanctioned reply — for one that does not.

- `initialize` mints a session and returns it in the `Mcp-Session-Id` header;
  every later request must send it back. A missing id is `400`; an unknown or
  expired one is `404`, which tells the client to start a new session.
- Cross-origin requests are refused. This is a spec MUST: without it a web page
  could drive a local MCP server through DNS rebinding. Clients that send no
  `Origin` (which is every non-browser client) are unaffected.
- An unsupported `MCP-Protocol-Version` header is rejected with `400`; an absent
  one is fine, and assumes the version the spec pins for older clients.

Protocol revisions `2025-06-18`, `2025-03-26` and `2024-11-05` are negotiated.

## What's not here yet

Phases 1–5 cover the server (tools, resources, prompts, streaming, sampling)
and the client. Still to come:

- **Cancellation** — `notifications/cancelled` is accepted and ignored;
  interrupting a tool already running in the VM is not yet possible, so a
  cancelled call runs to completion and its result is discarded.
- **Completion, subscriptions and pagination** — `completion/complete`
  currently answers "no suggestions" (the shape a client's completion UI
  expects) rather than erroring; `resources/subscribe` is not offered; and every
  list is served in a single page.
- **Client-side sampling** — this client declines a server's `sampling` or
  `elicitation` request. Answering one means giving CFML a way to reach a model,
  which is a larger design question than the transport.
- **OAuth 2.1** — the spec's full authorization extension, with discovery and
  token endpoints. Static bearer tokens cover a server you configure yourself;
  OAuth is what a public multi-tenant endpoint would need.
- **The deprecated HTTP+SSE transport** (protocol 2024-11-05's `/sse` plus a
  POST-back endpoint) for clients that predate Streamable HTTP.
- **A runnable `examples/mcp_demo/`.**
