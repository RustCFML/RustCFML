//! VM-intercepted realtime / WebSocket builtins.
//!
//! Extracted from `call_function`, which had grown to **7,497 lines** because the
//! documented way to add a VM-intercepted builtin was to append another
//! `if name_lower == "…"` branch to it and nothing ever moved one out. Each domain now
//! lives beside its peers, and `call_function` consults them as a short list of guards.
//!
//! Handlers return `Option<CfmlResult>`: `Some(..)` when this domain handled the call,
//! `None` to fall through to the next. The name set below MUST stay in step with
//! `cfml_common::builtins_meta::VM_INTERCEPTED` — the source-scanning guard in
//! `tests/intercept_declaration_guard.rs` enforces that, so moving code here cannot
//! silently drop an interception.

use super::*;

/// Names this module handles. Kept next to the code that implements them so the two
/// cannot drift, rather than in a list somewhere else.
#[inline]
pub(crate) fn handles(name_lower: &str) -> bool {
    name_lower.starts_with("$sio")
        || matches!(
            name_lower,
            "io" | "wspublish" | "assertbroadcast" | "wssubscribe" | "wsunsubscribe"
                | "wspresence"
        )
}

impl CfmlVirtualMachine {
    /// Dispatch a realtime/WebSocket builtin. `None` means "not mine".
    /// The extracted bodies. Split from the guard above so `?` and the original
    /// `return Ok(..)` statements keep working verbatim — the code below is moved, not
    /// rewritten, which is what makes this refactor reviewable.
    /// Bodies moved VERBATIM: `args` stays an owned `Vec` as in the original scope, so
    /// `?`, `return Ok(..)`, `&args` and `args.into_iter()` all still work. The caller
    /// checks [`handles`] first, so `args` only moves when this domain consumes it.
    pub(crate) fn dispatch_realtime(
        &mut self,
        name_lower: &str,
        args: Vec<CfmlValue>,
    ) -> CfmlResult {
            if name_lower == "io" {
                let named = self.pending_ws_named.take();
                let channel = self
                    .ws_arg(&args, &named, "channel", 0)
                    .map(|v| v.as_string())
                    .filter(|s| !s.is_empty())
                    .or_else(|| self.current_ws_channel.clone());
                let channel = match channel {
                    Some(c) => c,
                    None => {
                        return Err(self.wrap_error(CfmlError::runtime(
                            "io() needs a channel argument outside a socket handler"
                                .to_string(),
                        )))
                    }
                };
                let registry = match self.server_state.as_ref() {
                    Some(ss) => ss.websocket.clone(),
                    None => {
                        return Err(self.wrap_error(CfmlError::runtime(
                            "io() is only available in serve mode".to_string(),
                        )))
                    }
                };
                let emitter = crate::websocket::ServerEmitter::new(channel, registry);
                return Ok(CfmlValue::NativeObject(std::sync::Arc::new(
                    std::sync::RwLock::new(emitter),
                )));
            }
            if name_lower == "wspublish" {
                let named = self.pending_ws_named.take();
                let channel = self
                    .ws_arg(&args, &named, "channel", 0)
                    .map(|v| v.as_string())
                    .filter(|s| !s.is_empty())
                    .or_else(|| self.current_ws_channel.clone())
                    .unwrap_or_default();
                let event = self.ws_arg(&args, &named, "event", 1).map(|v| v.as_string());
                let data = self.ws_arg(&args, &named, "data", 2).unwrap_or(CfmlValue::Null);
                let to = self
                    .ws_arg(&args, &named, "to", 3)
                    .map(|v| v.as_string())
                    .filter(|s| !s.is_empty());
                let except = self
                    .ws_arg(&args, &named, "except", 4)
                    .map(|v| v.as_string())
                    .filter(|s| !s.is_empty());

                // Always record for connection-free testing (P14).
                let mut entry = ValueMap::default();
                entry.insert("channel".to_string(), CfmlValue::string(channel.clone()));
                entry.insert(
                    "event".to_string(),
                    event.clone().map(CfmlValue::string).unwrap_or(CfmlValue::Null),
                );
                entry.insert("data".to_string(), data.clone());
                entry.insert(
                    "to".to_string(),
                    to.clone().map(CfmlValue::string).unwrap_or(CfmlValue::Null),
                );
                entry.insert(
                    "except".to_string(),
                    except.clone().map(CfmlValue::string).unwrap_or(CfmlValue::Null),
                );
                self.ws_test_log.push(CfmlValue::strukt(entry));

                // Deliver if a live registry is present.
                if let Some(ss) = self.server_state.as_ref() {
                    let registry = ss.websocket.clone();
                    let frame = registry.msg(&channel, event, data);
                    match &to {
                        Some(room) => {
                            registry.to_room(&channel, room, frame, except.as_deref())
                        }
                        None => registry.broadcast(&channel, frame, except.as_deref()),
                    }
                }
                return Ok(CfmlValue::Null);
            }
            // assertBroadcast(channel, event[, predicate]) — test helper. True
            // iff a recorded wsPublish matched channel (+event when given), and
            // — when `predicate` is a closure — the closure returns true for its
            // data payload.
            if name_lower == "assertbroadcast" {
                let want_channel = args.first().map(|v| v.as_string()).unwrap_or_default();
                let want_event = args.get(1).map(|v| v.as_string()).filter(|s| !s.is_empty());
                let predicate = args.get(2).cloned();
                let log = self.ws_test_log.clone();
                for entry in &log {
                    let s = match entry {
                        CfmlValue::Struct(s) => s,
                        _ => continue,
                    };
                    let ch = s.get("channel").map(|v| v.as_string()).unwrap_or_default();
                    if !ch.eq_ignore_ascii_case(&want_channel) {
                        continue;
                    }
                    if let Some(ref we) = want_event {
                        let ev = s.get("event").map(|v| v.as_string()).unwrap_or_default();
                        if !ev.eq_ignore_ascii_case(we) {
                            continue;
                        }
                    }
                    match &predicate {
                        Some(p @ CfmlValue::Function(_)) => {
                            let data = s.get("data").unwrap_or(CfmlValue::Null);
                            let r = self.call_function(p, vec![data], &ValueMap::default())?;
                            if r.is_true() {
                                return Ok(CfmlValue::Bool(true));
                            }
                        }
                        _ => return Ok(CfmlValue::Bool(true)),
                    }
                }
                return Ok(CfmlValue::Bool(false));
            }
            // wsSubscribe/wsUnsubscribe — Phase 2 surface; accepted as no-ops so
            // feature-detecting code doesn't error (clients join rooms only via
            // server-side socket.join, design principle P6).
            if matches!(name_lower, "wssubscribe" | "wsunsubscribe") {
                return Ok(CfmlValue::Null);
            }
            // wsPresence([channel]) — flat accessor for the presence roster
            // (parallel to wsPublish). Returns `{}` when no registry/channel.
            if name_lower == "wspresence" {
                let named = self.pending_ws_named.take();
                let channel = self
                    .ws_arg(&args, &named, "channel", 0)
                    .map(|v| v.as_string())
                    .filter(|s| !s.is_empty())
                    .or_else(|| self.current_ws_channel.clone());
                return match (channel, self.server_state.as_ref()) {
                    (Some(channel), Some(ss)) => Ok(ss.websocket.presence_state(&channel)),
                    _ => Ok(CfmlValue::strukt(ValueMap::default())),
                };
            }

            // ── socket.io-lucee compat BIFs ($sio*) ───────────────────────
            // The flat seam the imperative SocketIoServer/Namespace/Socket CFCs
            // call instead of socket.io-lucee's embedded Java server. They store
            // handlers in the process-wide `socketio_compat` store and fan out
            // through the shared `WebSocketRegistry` — same engine as the fluent
            // `io()`/`wsPublish` API, different CFML surface.
            if name_lower.starts_with("$sio") {
                let compat = crate::socketio_compat::compat();
                let arg_s = |i: usize| args.get(i).map(|v| v.as_string()).unwrap_or_default();
                let arg_v = |i: usize| args.get(i).cloned().unwrap_or(CfmlValue::Null);
                match name_lower {
                    "$sioregisternamespace" => {
                        compat.register_namespace(&arg_s(0));
                        return Ok(CfmlValue::Null);
                    }
                    "$sioregisterednamespaces" => {
                        return Ok(CfmlValue::array(
                            compat
                                .registered_namespaces()
                                .into_iter()
                                .map(CfmlValue::string)
                                .collect(),
                        ));
                    }
                    // ($sioRegisterNsHandler ns, event, callback)
                    "$sioregisternshandler" => {
                        let cb = arg_v(2);
                        let fns = self.collect_reachable_fn_arcs(&cb);
                        compat.set_ns_handler(
                            &arg_s(0),
                            crate::socketio_compat::Handler { event: arg_s(1), callback: cb, fns },
                        );
                        return Ok(CfmlValue::Null);
                    }
                    // ($sioRegisterSocketHandler socketId, event, callback)
                    "$sioregistersockethandler" => {
                        let cb = arg_v(2);
                        let fns = self.collect_reachable_fn_arcs(&cb);
                        compat.set_socket_handler(
                            &arg_s(0),
                            crate::socketio_compat::Handler { event: arg_s(1), callback: cb, fns },
                        );
                        return Ok(CfmlValue::Null);
                    }
                    // ($sioBroadcast event, data, rooms, namespace, socketId)
                    // namespace set → broadcast to the whole namespace; socketId
                    // set → broadcast to the sender's namespace excluding it.
                    // rooms (array|string|empty) narrows to those rooms.
                    "$siobroadcast" => {
                        let event = arg_s(0);
                        let data = arg_v(1);
                        let rooms = sio_room_list(&arg_v(2));
                        let namespace = arg_s(3);
                        let socket_id = arg_s(4);
                        if let Some(ss) = self.server_state.as_ref() {
                            let registry = ss.websocket.clone();
                            let (channel, except) = if !namespace.is_empty() {
                                (namespace, None)
                            } else if !socket_id.is_empty() {
                                let ch = registry.channel_of(&socket_id).unwrap_or_default();
                                (ch, Some(socket_id))
                            } else {
                                (String::new(), None)
                            };
                            if !channel.is_empty() {
                                if rooms.is_empty() {
                                    let frame = registry.msg(
                                        &channel,
                                        Some(event),
                                        data,
                                    );
                                    registry.broadcast(&channel, frame, except.as_deref());
                                } else {
                                    for room in &rooms {
                                        let frame = registry.msg(
                                            &channel,
                                            Some(event.clone()),
                                            data.clone(),
                                        );
                                        registry.to_room(
                                            &channel,
                                            room,
                                            frame,
                                            except.as_deref(),
                                        );
                                    }
                                }
                            }
                        }
                        return Ok(CfmlValue::Null);
                    }
                    // ($sioSend socketId, event, data) — direct to one client.
                    "$siosend" => {
                        let socket_id = arg_s(0);
                        let event = arg_s(1);
                        let data = arg_v(2);
                        if let Some(ss) = self.server_state.as_ref() {
                            let registry = ss.websocket.clone();
                            let channel = registry.channel_of(&socket_id).unwrap_or_default();
                            let frame = registry.msg(&channel, Some(event), data);
                            registry.emit_to(&socket_id, frame);
                        }
                        return Ok(CfmlValue::Null);
                    }
                    "$siojoinroom" => {
                        if let Some(ss) = self.server_state.as_ref() {
                            ss.websocket.join(&arg_s(0), &arg_s(1));
                        }
                        return Ok(CfmlValue::Null);
                    }
                    "$sioleaveroom" => {
                        if let Some(ss) = self.server_state.as_ref() {
                            ss.websocket.leave(&arg_s(0), &arg_s(1));
                        }
                        return Ok(CfmlValue::Null);
                    }
                    // Leave every room except the connection's own id-room (which
                    // the registry auto-joins so "send to this socket" works).
                    "$sioleaveallrooms" => {
                        let socket_id = arg_s(0);
                        if let Some(ss) = self.server_state.as_ref() {
                            let registry = ss.websocket.clone();
                            for room in registry.rooms_of(&socket_id) {
                                if room != socket_id {
                                    registry.leave(&socket_id, &room);
                                }
                            }
                        }
                        return Ok(CfmlValue::Null);
                    }
                    "$siodisconnect" => {
                        let socket_id = arg_s(0);
                        if let Some(ss) = self.server_state.as_ref() {
                            ss.websocket.close_conn(&socket_id, 1000, String::new());
                        }
                        compat.drop_conn(&socket_id);
                        return Ok(CfmlValue::Null);
                    }
                    "$siogetdata" => {
                        return Ok(CfmlValue::strukt(compat.get_data(&arg_s(0))));
                    }
                    // ($sioSetData socketId, data-struct)
                    "$siosetdata" => {
                        let data = match arg_v(1) {
                            CfmlValue::Struct(s) => s.snapshot(),
                            _ => ValueMap::default(),
                        };
                        compat.set_data(&arg_s(0), data);
                        return Ok(CfmlValue::Null);
                    }
                    "$siosocketcount" => {
                        let n = self
                            .server_state
                            .as_ref()
                            .map(|ss| ss.websocket.channel_count(&arg_s(0)))
                            .unwrap_or(0);
                        return Ok(CfmlValue::Int(n as i64));
                    }
                    _ => {
                        return Err(self.wrap_error(CfmlError::runtime(format!(
                            "Unknown socket.io compat function [{}]",
                            name_lower
                        ))))
                    }
                }
            }


        // `handles()` and the branches above must agree; a name that passes the guard but
        // matches no branch is a bug in this module, not a user error.
        // Fell out of every branch — exactly what the original `if` chain did.
        // The caller turns this into fall-through; it is never seen by CFML.
        Err(intercepts_common::unhandled())
    }
}
