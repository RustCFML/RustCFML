//! `com.pixl8.socketiolucee.SocketIoServerWrapper` — socket.io-lucee's Java
//! server, shimmed onto RustCFML's own socket.io transport.
//!
//! socket.io-lucee (and Preside's `preside-ext-socket-io`, which vendors it)
//! keeps the whole server/namespace/socket model in CFML and delegates only the
//! wire to an embedded Java/Netty server, loaded as an OSGi bundle:
//!
//! ```cfml
//! variables._javaServer = createObject( "java", "com.pixl8.socketiolucee.SocketIoServerWrapper", "com.pixl8.socketio-lucee" )
//!     .init( this, ExpandPath( "/" ), getPageContext().getApplicationContext(), host, port, ... );
//! ```
//!
//! The engine already speaks socket.io — [`crate::socketio_compat`] plus the
//! shared [`crate::websocket::WebSocketRegistry`] back the `$sio*` BIFs and the
//! bundled `SocketIoServer.cfc`. So this shim is a thin adapter between the two
//! halves of that library's contract:
//!
//! **Outbound** (CFML calls the wrapper):
//!
//! | method | becomes |
//! |--------|---------|
//! | `registerNamespace( ns )`            | a namespace in the compat store |
//! | `startServer` / `stopServer`         | server state (the transport is the app's own server, already listening) |
//! | `isServerRunning` / `getServerState` | that state |
//! | `socketSend( ns, id, event, args )`  | a frame to that connection |
//! | `socketDisconnect( id, close )`      | closing the connection |
//! | `toJsonObj( json )`                  | `deserializeJSON` — the library pre-serializes non-simple args |
//!
//! **Inbound** (the wrapper calls CFML): the handler CFC passed to `init` is
//! stored, and the transport's dispatch calls `onConnect( ns, socketId,
//! request )`, `onSocketEvent( ns, socketId, event, args )`,
//! `onDisconnecting( ns, socketId )` and `onDisconnect( ns, socketId )` on it
//! — see `CfmlVirtualMachine::dispatch_sio_ns` / `dispatch_sio_event`.
//!
//! **One difference worth knowing**: the Java server binds its own `host:port`.
//! RustCFML serves `/socket.io/` on the application's own port, so `host`/`port`
//! are accepted and ignored and clients connect to the app. This is recorded in
//! docs/known-issues.md; an app whose browser client is pointed at a separate
//! socket.io port has to point it at the app instead.

use cfml_common::dynamic::{CfmlValue, ValueMap};

/// The class name, lowercased the way the `createObject( "java", … )` dispatch
/// compares them.
pub const SIO_WRAPPER_CLASS: &str = "com.pixl8.socketiolucee.socketioserverwrapper";

/// Build the wrapper shim. `init()` is a method call on it, as with every other
/// Java shim, so this carries no state of its own — the handler CFC and server
/// state live in the process-wide compat store.
pub fn make_wrapper_shim() -> CfmlValue {
    let mut m = ValueMap::default();
    m.insert("__java_shim".to_string(), CfmlValue::Bool(true));
    m.insert(
        "__java_class".to_string(),
        CfmlValue::string(SIO_WRAPPER_CLASS.to_string()),
    );
    CfmlValue::strukt(m)
}

/// The `initialRequest` object `onConnect` hands to the CFML layer. The library
/// reads it with `.get( "cookies" | "headers" | "uri" | "querystring" |
/// "remoteUser" )`, which a plain struct answers, and walks `cookies` looking
/// for its session cookie — so the handshake has to carry real values, not
/// placeholders.
pub fn initial_request(handshake: Option<ValueMap>) -> CfmlValue {
    let mut m = handshake.unwrap_or_default();
    for (key, empty) in [
        ("headers", CfmlValue::strukt(ValueMap::default())),
        ("cookies", CfmlValue::array(vec![])),
        ("uri", CfmlValue::string(String::new())),
        ("querystring", CfmlValue::string(String::new())),
        ("remoteuser", CfmlValue::string(String::new())),
    ] {
        if !m.iter().any(|(k, _)| k.eq_ignore_ascii_case(key)) {
            m.insert(key.to_string(), empty);
        }
    }
    CfmlValue::strukt(m)
}
