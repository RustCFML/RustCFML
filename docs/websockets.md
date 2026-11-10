# WebSockets / Realtime

RustCFML has native WebSocket support: a long-lived, full-duplex connection
served from the **same process and port** as your HTTP traffic — no servlet
container, no embedded Jetty, no Node sidecar. You write a **channel component**
(one CFC = one channel) with convention-named lifecycle methods, and the engine
bridges each inbound frame to a fresh VM exactly as it does an HTTP request.

> Rooms, `join`/`leave`, the fluent `io()` emitter, ack-by-return, binary + JSON
> codecs, emit-from-anywhere, `on="event"` routing, presence, authorization,
> `lastEventId` resumability, **multi-node fan-out**, and
> [whisper / client events](#whisper--client-events) all work — over **two
> wires**: the [raw WebSocket](#quick-start) endpoint (`/ws/<channel>`) and a
> [socket.io endpoint](#socketio-transport) (`/socket.io/`, namespace per
> channel), the latter also exposing a
> [socket.io-lucee compatibility layer](#socketio-lucee-compatibility-layer) so
> existing socket.io CFML apps run unchanged.

## Quick start

Create `websockets/chat.cfc` under your web root:

```cfml
component socket="/chat" encoding="json" {

    function onConnect( socket ) {
        socket.join( "lobby" );
        socket.emit( "welcome", { id = socket.id() } );
        socket.broadcast( "userJoined", { id = socket.id() } );   // everyone else
    }

    function onMessage( socket, message ) {          // `message` is a parsed struct
        io().to( "lobby" ).emit( "message", { from = socket.id(), text = message.text } );
        return { delivered = true };                  // becomes the client's ack
    }

    function onDisconnect( socket, reason ) {
        io().to( "lobby" ).emit( "userLeft", { id = socket.id() } );
    }
}
```

Connect from the browser at `ws://host/ws/chat`:

```js
const ws = new WebSocket(`ws://${location.host}/ws/chat`);
ws.onmessage = (e) => {
  const frame = JSON.parse(e.data);          // { t, ch, ev, d, id }
  if (frame.ev === "message") console.log(frame.d.from, frame.d.text);
};
ws.onopen = () => ws.send(JSON.stringify({ text: "hello" }));
```

Two runnable examples ship in the repo:

```bash
# Minimal two-tab chat (~25 lines of CFML) — start here.
rustcfml --serve examples/websocket_chat   # then open http://localhost:8500/ in two tabs

# Kitchen-sink demo — one channel exercising the whole surface (lifecycle,
# on= routing, every socket.* method, presence, canJoin, whisper, history,
# emit-from-anywhere) with a live wire-log UI.
rustcfml --serve examples/websocket_demo
```

See [`examples/websocket_chat/`](../examples/websocket_chat/) and
[`examples/websocket_demo/`](../examples/websocket_demo/).

## Channels & discovery

- A **channel** is one CFC under `/websockets/`. `websockets/chat.cfc` is
  reachable at `ws://host/ws/chat`.
- The optional `socket="/chat"` component attribute names the **wire channel**
  (the `ch` field in frames, and the target for `io("/chat")` / `wsPublish`).
  Without it the channel id defaults to `/<filename>`.
- `encoding="json"` makes `onMessage` receive a **parsed value** instead of raw
  text (see [Codec](#codec)).

## Lifecycle methods

All are optional and matched by convention (case-insensitive), like
`Application.cfc`.

| Method | When | Notes |
|---|---|---|
| `onConnect( socket )` | After the handshake | **Reject gate** — see below. |
| `onMessage( socket, message )` | Each inbound frame | Return value → client ack. |
| `onDisconnect( socket, reason )` | Connection closed | Always fires; rooms are auto-left regardless. |
| `onError( socket, err )` | A handler threw | `err` is `{ message, type }`. The connection survives. |

### The connect reject gate

`onConnect` decides whether the connection is allowed and what rooms it joins:

```cfml
function onConnect( socket ) {
    if ( socket.param( "token" ) != getSecretToken() ) {
        return false;                      // reject → handshake closed
    }
    return [ "lobby", "user:" & socket.param( "userId" ) ];   // auto-join these rooms
}
```

- Return `false` (or throw) → the connection is **rejected and closed**.
- Return an **array of room names** → the socket auto-joins them.
- Return nothing → the connection is accepted with no extra rooms.

Clients can **never** join rooms themselves — only server code calls
`socket.join()` — so authorization is enforced at join time.

## Authorization

Two declarative gates, both reading the identity you attach to `socket.data` at
connect (a real app resolves this from the session — see `socket.sessionId()`):

```cfml
function onConnect( socket ) {
    var user = lookUpUser( socket.sessionId() );   // your own resolution
    socket.data.authenticated = !isNull( user );
    socket.data.roles = isNull( user ) ? [] : user.roles;   // e.g. ["admin"]
}
```

**`secured` on a handler** gates it *before it runs*:

```cfml
function purge( socket, data ) on="purge" secured="admin" {  // needs the admin role
    ...
}
function whoami( socket, data ) on="whoami" secured {        // any authenticated socket
    return io().presence();
}
```

- bare `secured` → requires a truthy `socket.data.authenticated`.
- `secured="admin,editor"` → requires `socket.data.roles` (array) or
  `socket.data.role` (string) to include one of the listed roles (case-insensitive).
- `secured="false"` → explicitly opts out.

A denied call doesn't run the handler — it surfaces through `onError(socket, err)`
(an `onConnect` denial rejects the handshake), so the client gets a clean error
rather than silence.

**`canJoin( socket, room )`** gates room joins. Whenever server code calls
`socket.join( room )`, the channel's `canJoin` (if defined) is consulted first; a
falsey return (or a throw) rejects the join *loudly* — so a join derived from
client-supplied input can't slip a user into a room they shouldn't see:

```cfml
function canJoin( socket, room ) {
    return room.startsWith( "public-" ) || socket.data.roles.contains( "admin" );
}
```

## Event routing (`on="event"`)

For an `encoding="json"` channel, an inbound frame can name an **event** and be
routed to a dedicated handler instead of the catch-all `onMessage`. Annotate a
function with `on="<event>"`:

```cfml
component socket="/chat" encoding="json" {

    function say( socket, data ) on="say" {
        socket.broadcast( "said", data );
        return { delivered = true };          // → ack (see below)
    }

    function typing( socket, data ) on="typing" {
        socket.to( data.room ).emit( "typing", { who = socket.id() } );
    }

    function onMessage( socket, message ) {   // fallback: any frame with no matching event
        // ...
    }
}
```

A client sends an event frame matching the wire shape — `ev` is the event name,
`d` the payload, and an optional `id` rides back on the ack's `ref` so the
client can correlate the reply:

```js
ws.send(JSON.stringify({ ev: "say", d: { text: "hi" }, id: "req-1" }));
// → handler `say(socket, { text:"hi" })` runs; ack frame comes back with ref:"req-1"
```

Routing is case-insensitive. A frame with **no** `ev` (or whose event matches no
`on=` handler) falls through to `onMessage`, so un-annotated channels behave
exactly as before. The handler receives the `d` payload as its second argument.

## Whisper / client events

Some realtime chatter — typing indicators, cursor positions, presence pings — is
high-frequency and low-value: you want to relay it to other clients without
spinning up a VM or running any handler. A **whisper** does exactly that. Any
inbound event whose name starts with `client-` (the Pusher convention) is
relayed by the hub straight to peers, with **no CFML code running** and **no
history retained**.

```js
// browser — no server handler, no ack comes back
ws.send( JSON.stringify({ ev: "client-typing", d: { who: "alice" } }) );
```

Every *other* client on the channel receives it as a `t:"client"` frame:

```jsonc
{ "t": "client", "ch": "/chat", "ev": "client-typing", "d": { "who": "alice" }, "id": "node:51" }
```

The sender never receives its own whisper. By default a whisper fans out
**channel-wide** (everyone else on the channel). Add a `room` to scope it to a
single room — the sender must already be a member of that room, so a client
can't whisper into a room it hasn't joined (anything else is silently dropped):

```js
ws.send( JSON.stringify({ ev: "client-cursor", d: { x: x, y: y }, room: "doc-42" }) );
```

Whispers require an `encoding="json"` channel (the JSON envelope is what carries
the event name and optional `room`). They cross nodes like any broadcast, but are
ephemeral — never added to the `history` replay log, and no handler ever sees them.

### Whisper over socket.io

socket.io has no catch-all event handler, so the relayed client-event names must
be **declared up front** via a `clientEvents` attribute (comma-separated; the
`client-` prefix is optional and added for you):

```cfml
component socket="/chat" encoding="json" clientEvents="typing,cursor" {
    // ...no handler for the client events — the hub relays them
}
```

A socket.io client then `socket.emit("client-typing", { who: "alice" })` and
peers receive a `client-typing` event. Over socket.io the relay is channel-wide
(the payload is the event data itself, with no envelope to carry a `room`);
room-scoped whispers are a raw-WebSocket refinement. The raw-WS transport relays
any `client-*` event dynamically and ignores `clientEvents`.

## The `socket` object

The live handle passed to every lifecycle method:

| Method | Description |
|---|---|
| `socket.id()` | Stable per-connection id (node-qualified). |
| `socket.send( data )` | Raw frame to this client. A binary value is sent as a binary frame; anything else as JSON text. |
| `socket.emit( event, data )` | Named event to this client. |
| `socket.broadcast( event, data )` | To everyone in the channel **except** the sender. |
| `socket.join( room )` / `socket.leave( room )` | Room membership. |
| `socket.rooms()` | Array of rooms this socket is in. |
| `socket.to( room )` | Returns a scoped emitter → `.emit()` / `.send()` / `.except()`. |
| `socket.track( [key], meta )` / `socket.untrack( [key] )` | Presence — see below. |
| `socket.close( [code], [reason] )` | Close this connection. |
| `socket.data` | A **live, mutable struct** for per-connection state (see below). |
| `socket.param( name )` / `socket.params()` | Handshake query parameters (`?userId=42`). |
| `socket.sessionId()` | The `CFID` captured at the handshake. |

### Per-connection state: `socket.data`

`socket.data` is a plain CFML struct the engine keeps alive for the connection's
lifetime. Because structs are reference types, you just read and write it — no
`get`/`set`, no write-back ceremony:

```cfml
function onConnect( socket )            { socket.data.joinedAt = now(); }
function onMessage( socket, message )   { socket.data.lastSeen  = now(); }
```

## Rooms

A **room** is an ad-hoc fan-out group inside a channel. Joining an unknown room
creates it; emptying it destroys it — no registration. Every connection also
auto-joins a room named after **its own id**, so "send to one user" and "send to
a room" are the same primitive.

```cfml
socket.join( "room-42" );
io().to( "room-42" ).emit( "update", payload );      // everyone in the room
io().to( socket.id() ).emit( "private", payload );   // just one connection
```

On disconnect the engine removes the connection from **every** room
automatically, whether or not you define `onDisconnect` — the most common
realtime leak is impossible by default.

## Presence

Presence answers "who's here?" — *set my bit of state, read everyone else's*.
A connection **tracks** itself with some metadata; the engine keeps a per-channel
roster and ships **diffs** as people come and go (the Phoenix Presence model).

```cfml
function onConnect( socket ) {
    socket.track( { user = socket.param( "userId" ), status = "online" } );
}
```

- **`socket.track( meta )`** — adds this connection to the roster keyed by its
  own id. **`socket.track( key, meta )`** groups several connections under one
  key (e.g. a user's multiple tabs/devices → one roster entry with many `metas`).
  Re-tracking under the same key updates the meta.
- **`socket.untrack()`** / **`socket.untrack( key )`** — removes it. Disconnect
  untracks automatically, so the roster never leaks a ghost.

When a connection tracks, **it** receives the full roster as a `presence_state`
frame, and **everyone else** gets a `presence_diff` join. Leaves (including
disconnects) broadcast a `presence_diff` leave. Read the roster anytime:

```cfml
io().presence();            // inside a handler — the current channel's roster
io( "/chat" ).presence();   // by channel, from anywhere
wsPresence( "/chat" );      // flat-BIF equivalent
```

The roster shape (also the payload of a `presence_state` frame):

```jsonc
{
  "user-42": { "metas": [ { "user": "user-42", "status": "online" } ] },
  "user-99": { "metas": [ { "user": "user-99", "status": "away" } ] }
}
```

A `presence_diff` frame carries `{ "joins": { … }, "leaves": { … } }` in the same
per-key shape. Presence is channel-scoped and the roster is replicated across a
multi-node cluster (see [Clustering / multi-node](#clustering--multi-node)).

## Emit from anywhere

You don't need a `socket` handle to push to clients. Any ordinary `.cfm` page,
`cfthread`, or scheduled task can publish — this is the primary realtime
ergonomic for a request/response engine.

**`wsPublish()`** — the flat BIF, the canonical path:

```cfml
wsPublish( channel="/chat", event="announcement", data={ text="Maintenance in 5m" } );
wsPublish( channel="/chat", event="ping", data=now(), to="lobby" );    // a single room
wsPublish( channel="/chat", event="x", data=d, except=excludeConnId ); // self-echo control
```

**`io()`** — the fluent accessor (sugar over the same registry):

```cfml
io( "/chat" ).emit( "ping", now() );                       // whole channel
io( "/chat" ).to( "lobby" ).emit( "announcement", data );  // a room
io( "/chat" ).to( "lobby" ).except( cid ).emit( "x", d );  // exclude one connection
io( "/chat" ).in( "lobby" ).count();                       // member count
io( "/chat" ).sockets();                                   // connection ids
io( "/chat" ).presence();                                  // the presence roster
```

Inside a channel handler, `io()` with **no argument** refers to the current
channel.

## Codec

By default `onMessage` receives the raw text (or a `Binary` value for binary
frames). Declaring `encoding="json"` on the channel parses inbound text once, so
the handler gets a struct/array directly:

```cfml
component socket="/chat" encoding="json" {
    function onMessage( socket, message ) {   // already deserialized
        io().to( "lobby" ).emit( "chat", message.text );
    }
}
```

Malformed JSON degrades to the raw string rather than dropping the message, so
your handler can validate it.

## Acknowledgements

The return value of `onMessage` (and any `on="event"` handler) is shipped back
to the sending client as an `ack` frame (`ev:"ack"`). Return nothing for no ack:

```cfml
function onMessage( socket, message ) {
    var id = store( message );
    return { ok = true, id = id };     // client receives this as the ack
}
```

When the inbound frame carried an `id`, the ack echoes it back as `ref` so the
client can match the reply to its request.

## Resumability (`history` / `lastEventId`)

A client that briefly drops off the network can reconnect and **replay the
messages it missed**, in order, before live traffic resumes. Opt in per channel
with the `history="N"` attribute — the engine then retains the last `N`
channel-wide frames:

```cfml
component socket="/feed" encoding="json" history="100" {
    function publish( socket, data ) on="post" {
        io().emit( "post", data );   // channel-wide → retained in history
    }
}
```

Every outbound frame carries a monotonic `id` (`{nodeId}:{seq}`). The client
remembers the `id` of the last frame it processed and, on reconnect, sends it as
a `lastEventId` query parameter:

```js
const last = localStorage.getItem("lastEventId");
const url  = "/ws/feed" + (last ? "?lastEventId=" + encodeURIComponent(last) : "");
const ws   = new WebSocket(url);
ws.onmessage = e => { const f = JSON.parse(e.data); if (f.id) localStorage.setItem("lastEventId", f.id); /* … */ };
```

The engine replays every retained frame newer than that cursor (keeping each
frame's original `id`, so the client keeps advancing). What gets retained is the
**channel-wide fan-out** — `io().emit()`, `io().to(room).emit()`,
`socket.broadcast()`. Per-connection sends (acks, a `presence_state` snapshot) are
not history and are never replayed.

If the cursor is older than the oldest frame still retained, the client has
provably lost messages: the engine first sends a `{ "t": "reset" }` frame so the
app knows to resync from its own source of truth, then replays what it still has.

```jsonc
{ "t": "reset", "ch": "/feed", "id": "node:512" }   // gap — you missed messages; resync
```

**Caveats.** History is **best-effort, in-memory, and per node**: it is lost on
restart, capped at `N` frames, and a `lastEventId` minted by a *different* node
(after a failover) is skipped (the client gets the `reset` hint). Replay is
channel-wide, not room-precise — the socket re-establishes its rooms via
`onConnect` auto-join anyway.

## socket.io transport

Every channel CFC is **also** reachable over [socket.io](https://socket.io)
(Engine.IO v4: namespaces, acks, binary, and automatic polling↔websocket
fallback) at the `/socket.io/` endpoint — served from the same process and port,
sharing the **same registry** as the raw endpoint. You write the channel CFC
once; it answers both wires. Use this when you want the battle-tested socket.io
JS client (reconnection, backoff, presence, transports) for free.

The mapping is convention-only:

- A channel is a socket.io **namespace**: `websockets/chat.cfc` → namespace
  `/chat`. The client connects with `io("/chat")`.
- A client **`emit("event", data)`** dispatches the channel's `on="event"`
  handler (falling back to `onMessage` for the conventional `"message"` event),
  exactly like the raw transport's event routing.
- The handler's **return value is the native socket.io ack** (the client's
  `emit(..., (ack) => …)` callback / `emitWithAck` promise) — ack-by-return, with
  no separate frame.
- Server pushes (`socket.emit`, `socket.broadcast`, `io().to(room).emit`,
  `wsPublish`, presence) arrive as socket.io **events** named by the frame's
  event (a raw `socket.send()` → the `"message"` event).
- `onConnect`/`onDisconnect`/`onError` and the `onConnect` reject gate behave
  identically. Auth (`secured=` / `canJoin`), rooms, presence, and `lastEventId`
  replay (via the socket.io `query` option) all work unchanged.

```js
import { io } from "socket.io-client";

const sock = io(`${location.origin}/chat`, { query: { user: "alice" } });

sock.on("welcome", (d) => console.log("connected as", d.id));
sock.on("message", (d) => console.log(d.from, d.text));

// emit with a native ack (the handler's return value)
const ack = await sock.emitWithAck("say", { text: "hello" });
console.log(ack.delivered);
```

> Identity at the handshake: the `CFID` cookie (if same-origin) attaches the
> session, and socket.io `query` params are readable via `socket.param(name)` —
> the same as the raw transport. Because the connect handler runs as soon as the
> socket connects, a client should wait for the `connect` event before its first
> `emit` (the socket.io client does this by default).

For the **imperative** socket.io-lucee CFML surface (`new SocketIoServer()` /
`io.of(ns).on("connect", …)` / `socket.on/emit/joinRoom`), see
[socket.io-lucee compatibility layer](#socketio-lucee-compatibility-layer) below
— it rides this same transport and registry.

## socket.io-lucee compatibility layer

Alongside the convention API (one CFC per channel under `websockets/`), RustCFML
ships an **imperative** surface that mirrors
[socket.io-lucee](https://github.com/pixl8/socket.io-lucee) — so a
`preside-ext-socket-io`-style handler runs largely unchanged. Both surfaces share
the one `/socket.io/` transport and the one registry; a namespace is owned by
whichever surface registered it.

The engine bundles the `SocketIoServer` / `SocketIoNamespace` / `SocketIoSocket`
CFCs (no install needed). Create the server once and store it somewhere
long-lived (application scope is the norm) so its handler closures survive across
requests — typically in `onApplicationStart`, which the first socket.io
connection runs if no request has yet:

```cfml
component {
    function onApplicationStart() {
        application.io = new SocketIoServer();

        application.io.of( "/chat" ).on( "connect", function( socket ){
            socket.emit( "welcome", { id = socket.getId() } );

            // Inbound event; the return value is the client's native ack.
            socket.on( "say", function( msg ){
                socket.broadcast( "said", msg );   // everyone else in the namespace
                return { delivered = true };
            } );

            socket.on( "joinRoom", function( data ){
                socket.joinRoom( data.room );
                application.io.of( "/chat" ).emit( "roomNews", { room = data.room }, [ data.room ] );
            } );
        } );

        return true;
    }
}
```

The JS side is a stock socket.io client connecting to the namespace
(`io("/chat")`). Surface supported:

- **`SocketIoServer`** — `of(ns)` / `namespace(ns)` (register + get a namespace),
  `on(event, cb)` (shortcut for the `/` namespace), `getRegisteredNamespaces()`,
  and `this.sockets` (the root namespace). `start()`/`stop()`/`close()`/`shutdown()`
  are no-ops (the transport is the engine's own server, always running);
  `isRunning()` is `true` and `getState()` is `"RUNNING"`. Constructor args
  (host/port/cors/ping…) are accepted for compatibility and ignored.
- **namespace** — `on(event, cb)` for `connect` / `disconnect` / `disconnecting`
  (the callback receives the `socket`), `broadcast(event, args, rooms)` /
  `emit(...)` (whole namespace, or narrowed to room(s)), `getSocketCount()`.
- **socket** — `on(event, cb)` (inbound), `emit(event, args)` / `send(message)`
  (direct to this client), `broadcast(event, args, rooms)` (everyone else),
  `joinRoom` / `leaveRoom` / `leaveAllRooms`, `disconnect(close)`,
  `getId()` / `getNamespace()`, and `getSocketData()` / `setSocketData(struct)`
  (per-connection state held engine-side across messages).

Notes and current limits:

- **Bootstrap before connections.** The app owns registration — run it (e.g. in
  `onApplicationStart`) before clients connect, exactly as socket.io-lucee
  requires. A connection to an unregistered namespace falls through to the
  convention `websockets/` discovery.
- **Register `socket.on` handlers inside the connect listener.** socketioxide
  has no catch-all, so the transport subscribes to exactly the events the connect
  listener registered. Frames the connect listener emits (e.g. `welcome`) are
  buffered until those handlers are wired, so a client that emits in reaction to
  the greeting is routed correctly; a `socket.on` added *after* connect returns
  is not wired.
- **One payload per event.** `emit`/`broadcast` deliver a single payload value
  (socket.io's multi-argument form is not split out), matching the rest of the
  realtime engine.
- **Server-initiated ack callbacks** (`socket.emit(event, args, ackCallback)`)
  are accepted for API compatibility but not delivered back — client→server acks
  (the handler return value) work, as in the example above.
- Preside's own `isWebUser()` / `getWebsiteLoggedInUserId()` stay in the
  extension; they read the session the engine attaches at the handshake.

## Wire format

Raw-WS frames are JSON with a stable shape (designed once so ids never change
when clustering is enabled later). The socket.io transport maps the same `ev`/`d`
onto socket.io event packets, so the field names below are the raw-WS encoding:

```jsonc
{
  "t":  "msg",        // frame type: msg | ack | client (whisper) | presence | reset | ...
  "ch": "/chat",      // channel
  "ev": "message",    // event name (routes to on="message"; absent for a raw send())
  "d":  { },          // payload
  "id": "node:42",    // node-qualified, monotonic message id
  "ref":"req-1"       // ack correlation — echoes an inbound frame's id (acks only)
}
```

## Clustering / multi-node

When the server runs as part of a **clustered session store** (`--features
cluster`, with `sessionStorage` pointing at a `provider: "cluster"` cache),
WebSocket fan-out automatically rides the same gossip cluster — no extra config
beyond the cluster's own (including its required `secretKey`; see
[Configuration](configuration.md)).
A `wsPublish` / `io().broadcast()` / `socket.to(room)` on one node reaches
clients connected to **any** node, and the presence roster is cluster-wide. The
CFML API is identical to single-node; nothing in your channel code changes.

How it works: every channel-wide broadcast and room fan-out is published to peer
nodes over the cluster, and each node re-delivers to its own connected clients
(the Socket.IO Redis-adapter model — no sticky sessions needed). Connection ids
are node-qualified, so a send to a specific connection routes to its owning node.
Presence `track`/`untrack` is replicated, and when a node leaves the cluster its
roster entries are evicted (a leave diff is emitted to remaining clients).

Caveats (best-effort, in line with the realtime contract):

- **Fan-out is best-effort.** A cross-node frame dropped under load is a missed
  realtime message, not state corruption — exactly like a single dropped frame to
  a slow client.
- **`lastEventId` replay stays node-local.** History is retained on the node that
  produced the frames; a client that reconnects to a *different* node receives a
  `reset` hint (no replay) rather than a gap. Pin reconnects to the same node, or
  treat `reset` as "resync from scratch".
- **Room `count()` / `sockets()` are node-local.** They report this node's members
  only — room *membership* is not replicated (delivery does not need it). For a
  cluster-wide "who's here", use **presence** (`io(channel).presence()`), which is
  replicated.

## Concurrency & safety

The engine owns the connection's event loop, so your handlers never see frame
loops or backpressure:

- Per-connection dispatch is **serialized**; each message runs on a fresh VM
  (like an HTTP request), so handlers may safely block.
- Each dispatch enters the **same application a page request does**: the
  governing `Application.cfc` applies, `onApplicationStart` runs if it has not
  yet, and `application` scope is the one live scope the whole site shares — so
  a handler can cache in it and a page sees what the handler wrote.
  `onRequestStart`/`onRequest`/`onRequestEnd` do not run; a frame is not a page.
- Outbound frames go through a **bounded queue** — a slow client cannot stall a
  handler; on overflow the connection is closed.
- Ping/pong is answered by the engine and **never** dispatched to CFML.

## Testing without a live socket

Realtime logic is testable from `tests/runner.cfm` with no WebSocket client.
`wsPublish` records every broadcast, and `assertBroadcast()` inspects it:

```cfml
wsPublish( channel="/chat", event="message", data={ text="hi" } );

assertBroadcast( "/chat", "message" );                                  // true
assertBroadcast( "/chat", "message", function( d ){ return d.text == "hi"; } );  // predicate
assertBroadcast( "/chat", "nope" );                                     // false
```

(Live-socket behaviour — echo, broadcast, rooms, reject, binary — is covered by
the Rust integration suites in `crates/cli/tests/websocket_raw.rs` (raw WS),
`crates/cli/tests/websocket_socketio.rs` (the socket.io transport: connect, emit,
native ack, server-pushed events, broadcast, reject), and
`crates/cli/tests/websocket_sio_compat.rs` (the imperative socket.io-lucee
surface: connect listener, per-socket `socket.on` with native ack, broadcast,
room-scoped `namespace.emit`).)

## Not yet supported

A couple of declarative conveniences are not built yet:

- **Domain-event auto-broadcast** — a `component broadcast=true` / model mixin
  that publishes automatically on `save`/`delete`.
- **Naming-convention sugar** — optional `/topic`·`/user` channel conventions.

Everything else described on this page is implemented and tested.
