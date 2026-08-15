//! WebSocket / realtime core — registry, rooms, wire envelope, and the live
//! `socket` / `io()` NativeObjects.
//!
//! This module is deliberately **axum/tokio-free**. The only contact with the
//! async world is the [`FrameSink`] trait: the connection driver in
//! `crates/cli` implements it over a bounded `tokio::mpsc::Sender`, but nothing
//! here knows that. That keeps `cfml-vm` (and therefore `cfml-worker` /
//! `rustcfml-wasm`, which build for `wasm32` and never run a server) compiling
//! on every target — the registry is just data + trait-bounded sends.
//!
//! Design rationale and the cross-ecosystem principle catalog live in
//! `docs/websocket-design.md`; the build order and wire spec in
//! `docs/websocket-implementation-plan.md`. Cluster-readiness is baked in from
//! the start (decision 3): connection ids are **node-qualified**
//! (`{nodeId}:{ulid}`), per-channel message ids are monotonic, and all fan-out
//! routes through the registry so a distributed `Broker` (Phase 2) can slot in
//! at the same call sites with no id/wire changes.

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use cfml_common::dynamic::{CfmlNative, CfmlStruct, CfmlValue, ValueMap};
use cfml_common::vm::{CfmlError, CfmlResult};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
/// `CfmlValue::NativeObject` wraps `std::sync::RwLock` (not parking_lot's),
/// so emitter/handle objects handed back to CFML must use this one.
use std::sync::RwLock as StdRwLock;

/// A node-qualified connection id: `"{nodeId}:{ulid}"`. Kept as a plain
/// `String` so it drops straight into `CfmlValue::String` for the CFML surface
/// (`socket.id()`) and into `HashMap` keys with no newtype friction. The node
/// prefix lets "send to connection X" route to the owning node once the
/// distributed `Broker` lands.
pub type ConnId = String;

/// The single outbound primitive. The connection driver task (in `cli`)
/// implements this over a bounded channel to the WebSocket sink; the registry
/// only ever calls `send`/`close`, so no async types leak into `cfml-vm`.
pub trait FrameSink: Send + Sync + std::fmt::Debug {
    /// Enqueue a frame for delivery to this connection. Implementations must be
    /// non-blocking (bounded channel; drop/close on overflow) — the registry is
    /// called from synchronous VM code.
    fn send(&self, frame: WireEnvelope);
    /// Request the connection be closed with the given code/reason.
    fn close(&self, code: u16, reason: String);
}

/// The cross-node fan-out primitive (decision 3: cluster-ready from day one).
/// A single-node server never installs one — the registry's broker stays
/// `None` and every method is byte-for-byte the Phase-1 local path. When the
/// distributed adapter (in `crates/cli`, over the shared-session `memberlist`
/// gossip cluster) is installed via [`WebSocketRegistry::set_broker`], the
/// three fan-out methods additionally `publish` to peers, who re-deliver to
/// their own local connections (the Socket.IO Redis-adapter model). Like
/// [`FrameSink`], this is **pure** — no axum/tokio/cluster types leak into
/// `cfml-vm`, so the `wasm32` builds (`cfml-worker`/`rustcfml-wasm`) stay green.
pub trait Broker: Send + Sync + std::fmt::Debug {
    /// Hand a fan-out event to every *other* node. Implementations must be
    /// non-blocking (the registry is called from synchronous VM code) — enqueue
    /// and return; drop on overflow, exactly like the session cluster.
    fn publish(&self, msg: BrokerMsg);
}

/// A cross-node fan-out event. Carries live [`CfmlValue`] payloads in-process;
/// the cli adapter serializes it (via `serde_json`, since [`CfmlValue`]'s
/// `Deserialize` is `deserialize_any` and needs a self-describing format —
/// bincode would fail) before it crosses the gossip wire, and feeds received
/// ones straight into [`WebSocketRegistry::apply_broker_msg`].
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum BrokerMsg {
    /// Re-run a channel-wide broadcast on the receiving node's local conns.
    Broadcast {
        channel: String,
        frame: WireEnvelope,
        except: Option<String>,
    },
    /// Re-run a room fan-out on the receiving node's local room members.
    ToRoom {
        channel: String,
        room: String,
        frame: WireEnvelope,
        except: Option<String>,
    },
    /// Deliver to a specific (node-qualified) connection — only the owning node
    /// has it locally; everyone else no-ops.
    EmitTo { conn_id: String, frame: WireEnvelope },
    /// Replicate a presence-roster entry. Roster state only — the client-facing
    /// `presence_diff` rides the separate `Broadcast` of the diff frame, so this
    /// merges state without re-broadcasting (no double delivery).
    PresenceTrack {
        channel: String,
        key: String,
        conn_id: String,
        meta: CfmlValue,
    },
    /// Remove a replicated presence-roster entry (roster state only).
    PresenceUntrack {
        channel: String,
        key: String,
        conn_id: String,
    },
    /// A peer left the cluster — evict every roster entry it owned (its
    /// node-qualified conn ids) and emit leave diffs to local clients.
    NodeGone { node_id: String },
}

/// One realtime frame on the wire. Raw-WS transports serialize this to JSON;
/// the socket.io transport (Phase 3) maps the same fields onto Engine.IO
/// packets. Designed once so ids never have to change when the distributed
/// `Broker` switches on. `d` stays a live [`CfmlValue`] until the driver
/// serializes it, so `encoding="json"` round-trips structs/arrays unchanged.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WireEnvelope {
    /// Frame type: `msg|ack|join|leave|presence|err|ping|pong`.
    pub t: String,
    /// Channel (handler + auth boundary), e.g. `/chat`.
    pub ch: String,
    /// Event name (routes to `on="message"` / `onMessage`). `None` for a raw
    /// `socket.send()` text/binary frame.
    pub ev: Option<String>,
    /// Payload — auto-(de)serialized when the channel declares `encoding="json"`.
    pub d: CfmlValue,
    /// Node-qualified, monotonic-per-channel id → resumability + routing.
    pub id: String,
    /// Ack correlation id (set when a reply is expected).
    pub ref_id: Option<String>,
}

impl WireEnvelope {
    /// Render the envelope as a CFML struct (for JSON serialization by the
    /// driver). Keys mirror the wire spec; absent optionals are omitted.
    pub fn to_value_map(&self) -> ValueMap {
        let mut m = ValueMap::default();
        m.insert("t".to_string(), CfmlValue::string(self.t.clone()));
        m.insert("ch".to_string(), CfmlValue::string(self.ch.clone()));
        if let Some(ev) = &self.ev {
            m.insert("ev".to_string(), CfmlValue::string(ev.clone()));
        }
        m.insert("d".to_string(), self.d.clone());
        m.insert("id".to_string(), CfmlValue::string(self.id.clone()));
        if let Some(r) = &self.ref_id {
            m.insert("ref".to_string(), CfmlValue::string(r.clone()));
        }
        m
    }
}

/// Per-connection record held only by the owning node.
struct ConnEntry {
    channel: String,
    sink: Arc<dyn FrameSink>,
    rooms: HashSet<String>,
    /// Live, reference-typed per-connection state. The same handle is handed to
    /// CFML as `socket.data`, so mutations there persist for the connection's
    /// life with no get/set/write-back ceremony (design principle P7).
    data: CfmlStruct,
    /// Handshake query parameters (`?userId=42`), read by `socket.param(name)`.
    params: ValueMap,
    /// CFID from the handshake cookie — identity is resolved once at connect
    /// and then ambient on the socket (design principle P6).
    session_id: Option<String>,
    /// Presence keys this connection has tracked itself under (design principle
    /// P11). Recorded so disconnect can emit leave diffs and never leak a stale
    /// roster entry.
    presence_keys: HashSet<String>,
}

// `CfmlStruct` is not `Debug`, so derive can't reach through `data`. The other
// fields are all the registry's `{:?}` ever needs.
impl std::fmt::Debug for ConnEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConnEntry")
            .field("channel", &self.channel)
            .field("rooms", &self.rooms)
            .field("params", &self.params.keys().collect::<Vec<_>>())
            .field("session_id", &self.session_id)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Default)]
struct Inner {
    conns: HashMap<ConnId, ConnEntry>,
    /// `(channel, room)` → member connection ids. Local membership index; the
    /// distributed `Broker` merges remote membership on top (Phase 2).
    rooms: HashMap<(String, String), HashSet<ConnId>>,
    /// Discovered channel → CFC file path (bytecode-cached at dispatch time).
    channels: HashMap<String, String>,
    /// Presence roster: `(channel, key)` → `conn → meta`. A key (e.g. a user id)
    /// can have several metas — one per connection/device — exactly like Phoenix
    /// Presence. `BTreeMap` keeps the metas list deterministic. The distributed
    /// `Broker` merges remote presence on top (Phase 2).
    presence: HashMap<(String, String), BTreeMap<ConnId, CfmlValue>>,
    /// Best-effort, in-memory resumability history (design principle P12): per
    /// channel, a bounded ring of the channel-wide fan-out frames in send order.
    /// A reconnecting client replays everything newer than its `lastEventId`.
    /// Opt-in per channel via the `history="N"` component attribute — only
    /// channels with a non-zero cap retain anything. The distributed `Broker`
    /// makes this cluster-correct (Phase 2); single-node + same-node-only today.
    history: HashMap<String, VecDeque<WireEnvelope>>,
    /// Per-channel retention cap for `history` (0 / absent = disabled).
    history_cap: HashMap<String, usize>,
}

/// The realtime connection registry. Lives on `ServerState` so it crosses
/// requests (emit-from-anywhere, design principle P1). Single `RwLock` for
/// Phase 1 — correctness over contention; a sharded/`DashMap` form is a drop-in
/// later if profiling demands it.
#[derive(Debug)]
pub struct WebSocketRegistry {
    /// Node-qualifies conn ids and message ids. Interior-mutable so the cli can
    /// unify it with the cluster node name at startup (before any connection) —
    /// `NodeGone` eviction keys off this matching the gossip node id. Set-once;
    /// the read cost is negligible at WebSocket frame rates.
    node_id: RwLock<Arc<str>>,
    inner: RwLock<Inner>,
    seq: AtomicU64,
    /// Cheap, lock-free gate so the broadcast/to_room hot path never takes the
    /// write lock for history when no channel has opted in.
    history_enabled: AtomicBool,
    /// The cross-node fan-out adapter, installed by the cli when a distributed
    /// cluster is active (see [`Broker`]). `None` on a single node.
    broker: RwLock<Option<Arc<dyn Broker>>>,
    /// Lock-free gate so the fan-out hot path never takes the broker read lock
    /// when no broker is installed (the common single-node case).
    has_broker: AtomicBool,
}

impl WebSocketRegistry {
    pub fn new(node_id: impl Into<Arc<str>>) -> Self {
        Self {
            node_id: RwLock::new(node_id.into()),
            inner: RwLock::new(Inner::default()),
            seq: AtomicU64::new(1),
            history_enabled: AtomicBool::new(false),
            broker: RwLock::new(None),
            has_broker: AtomicBool::new(false),
        }
    }

    /// Install the distributed fan-out adapter. Called once at server start when
    /// a clustered session store is active; absent that, the registry is
    /// single-node and this is never called.
    pub fn set_broker(&self, broker: Arc<dyn Broker>) {
        *self.broker.write() = Some(broker);
        self.has_broker.store(true, Ordering::Relaxed);
    }

    /// Publish a fan-out event to peers, if a broker is installed. Lock-free
    /// no-op on a single node.
    fn publish(&self, msg: BrokerMsg) {
        if !self.has_broker.load(Ordering::Relaxed) {
            return;
        }
        if let Some(b) = self.broker.read().as_ref() {
            b.publish(msg);
        }
    }

    pub fn node_id(&self) -> String {
        self.node_id.read().to_string()
    }

    /// Override the node id. Call once at startup, before accepting connections,
    /// to align with the gossip cluster's node name (single-node never calls it).
    pub fn set_node_id(&self, node_id: impl Into<Arc<str>>) {
        *self.node_id.write() = node_id.into();
    }

    /// Mint the next monotonic, node-qualified message id.
    pub fn next_id(&self) -> String {
        let n = self.seq.fetch_add(1, Ordering::Relaxed);
        format!("{}:{}", self.node_id.read(), n)
    }

    fn new_conn_id(&self) -> ConnId {
        format!("{}:{}", self.node_id.read(), uuid::Uuid::new_v4())
    }

    /// Build a `msg` envelope with a fresh id.
    pub fn msg(&self, channel: &str, event: Option<String>, data: CfmlValue) -> WireEnvelope {
        WireEnvelope {
            t: "msg".to_string(),
            ch: channel.to_string(),
            ev: event,
            d: data,
            id: self.next_id(),
            ref_id: None,
        }
    }

    // ── channel discovery cache ───────────────────────────────────────────

    pub fn set_channel(&self, channel: &str, cfc_path: &str) {
        self.inner
            .write()
            .channels
            .insert(channel.to_lowercase(), cfc_path.to_string());
    }

    pub fn channel_cfc(&self, channel: &str) -> Option<String> {
        self.inner.read().channels.get(&channel.to_lowercase()).cloned()
    }

    // ── connection lifecycle ──────────────────────────────────────────────

    /// Register a freshly-upgraded connection. Returns its node-qualified id.
    /// The connection auto-joins a room named after its own id (design
    /// principle P2: "send to a user" and "send to a room" are one primitive).
    pub fn register(
        &self,
        channel: &str,
        sink: Arc<dyn FrameSink>,
        session_id: Option<String>,
        params: ValueMap,
    ) -> ConnId {
        let channel = channel.to_lowercase();
        let conn_id = self.new_conn_id();
        let mut rooms = HashSet::new();
        rooms.insert(conn_id.clone());
        let mut inner = self.inner.write();
        inner
            .rooms
            .entry((channel.clone(), conn_id.clone()))
            .or_default()
            .insert(conn_id.clone());
        inner.conns.insert(
            conn_id.clone(),
            ConnEntry {
                channel,
                sink,
                rooms,
                data: CfmlStruct::empty(),
                params,
                session_id,
                presence_keys: HashSet::new(),
            },
        );
        conn_id
    }

    /// Remove a connection from the registry and every room it belonged to.
    /// Returns `(channel, rooms)` so the driver can fire `onDisconnect` and any
    /// presence diffs. Cleanup is unconditional (design principle P10: the #1
    /// realtime leak is impossible by default).
    pub fn unregister(&self, conn_id: &str) -> Option<(String, Vec<String>)> {
        // Mutate under the write lock, then release it *before* broadcasting the
        // presence-leave diffs — `broadcast` takes a read lock and parking_lot's
        // RwLock is not reentrant.
        let (channel, rooms, leaves) = {
            let mut inner = self.inner.write();
            let entry = inner.conns.remove(conn_id)?;
            let channel = entry.channel.clone();
            let rooms: Vec<String> = entry.rooms.iter().cloned().collect();
            for room in &rooms {
                if let Some(set) = inner.rooms.get_mut(&(channel.clone(), room.clone())) {
                    set.remove(conn_id);
                    if set.is_empty() {
                        inner.rooms.remove(&(channel.clone(), room.clone()));
                    }
                }
            }
            // Drop every presence entry this connection held, capturing the
            // removed metas so the channel learns it left (design principle P10).
            let mut leaves = Vec::new();
            for key in entry.presence_keys.iter() {
                let map_key = (channel.clone(), key.clone());
                if let Some(map) = inner.presence.get_mut(&map_key) {
                    if let Some(meta) = map.remove(conn_id) {
                        leaves.push((key.clone(), meta));
                    }
                    if map.is_empty() {
                        inner.presence.remove(&map_key);
                    }
                }
            }
            (channel, rooms, leaves)
        };
        for (key, meta) in leaves {
            let frame = self.presence_diff_frame(&channel, false, &key, &meta);
            // Broadcast auto-publishes the leave diff to remote clients; the
            // PresenceUntrack drops the entry from remote rosters too.
            self.broadcast(&channel, frame, None);
            self.publish(BrokerMsg::PresenceUntrack {
                channel: channel.clone(),
                key,
                conn_id: conn_id.to_string(),
            });
        }
        Some((channel, rooms))
    }

    pub fn join(&self, conn_id: &str, room: &str) {
        let room = room.to_lowercase();
        let mut inner = self.inner.write();
        let channel = match inner.conns.get(conn_id) {
            Some(e) => e.channel.clone(),
            None => return,
        };
        if let Some(e) = inner.conns.get_mut(conn_id) {
            e.rooms.insert(room.clone());
        }
        inner
            .rooms
            .entry((channel, room))
            .or_default()
            .insert(conn_id.to_string());
    }

    pub fn leave(&self, conn_id: &str, room: &str) {
        let room = room.to_lowercase();
        let mut inner = self.inner.write();
        let channel = match inner.conns.get(conn_id) {
            Some(e) => e.channel.clone(),
            None => return,
        };
        if let Some(e) = inner.conns.get_mut(conn_id) {
            e.rooms.remove(&room);
        }
        if let Some(set) = inner.rooms.get_mut(&(channel.clone(), room.clone())) {
            set.remove(conn_id);
            if set.is_empty() {
                inner.rooms.remove(&(channel, room));
            }
        }
    }

    pub fn rooms_of(&self, conn_id: &str) -> Vec<String> {
        self.inner
            .read()
            .conns
            .get(conn_id)
            .map(|e| e.rooms.iter().cloned().collect())
            .unwrap_or_default()
    }

    /// The live per-connection `data` struct (reference-typed handle).
    pub fn data_of(&self, conn_id: &str) -> Option<CfmlStruct> {
        self.inner.read().conns.get(conn_id).map(|e| e.data.clone())
    }

    pub fn channel_of(&self, conn_id: &str) -> Option<String> {
        self.inner.read().conns.get(conn_id).map(|e| e.channel.clone())
    }

    /// Handshake query params for a connection.
    pub fn params_of(&self, conn_id: &str) -> Option<ValueMap> {
        self.inner.read().conns.get(conn_id).map(|e| e.params.clone())
    }

    /// CFID captured at the handshake (identity is ambient on the socket, P6).
    pub fn session_id_of(&self, conn_id: &str) -> Option<String> {
        self.inner.read().conns.get(conn_id).and_then(|e| e.session_id.clone())
    }

    // ── delivery ──────────────────────────────────────────────────────────

    /// Deliver to a single connection (by id). If the connection is local, send
    /// directly; otherwise (a node-qualified id we don't own) route it to the
    /// owning node through the `Broker`. On a single node every id is local.
    pub fn emit_to(&self, conn_id: &str, frame: WireEnvelope) {
        {
            let inner = self.inner.read();
            if let Some(entry) = inner.conns.get(conn_id) {
                entry.sink.send(frame);
                return;
            }
        }
        // Not ours — only the owning node has it. No-op on a single node.
        self.publish(BrokerMsg::EmitTo {
            conn_id: conn_id.to_string(),
            frame,
        });
    }

    /// Deliver to every connection on `channel`, optionally excluding one id
    /// (self-echo control — design principle P4). Channel-wide fan-out is
    /// recorded for resumability and published to peers (cluster fan-out); peers
    /// re-deliver via [`broadcast_local`](Self::broadcast_local).
    pub fn broadcast(&self, channel: &str, frame: WireEnvelope, except: Option<&str>) {
        // Record on the originating node only (replay is same-node, decision 3);
        // peers re-fan-out via broadcast_local, which never records.
        self.record_history(&channel.to_lowercase(), &frame);
        self.publish(BrokerMsg::Broadcast {
            channel: channel.to_string(),
            frame: frame.clone(),
            except: except.map(str::to_string),
        });
        self.broadcast_local(channel, frame, except);
    }

    /// Local-only channel broadcast — no history, no publish. Used by the
    /// originating node (after recording + publishing) and by every peer that
    /// receives a `Broadcast` message (re-fan-out to its own conns).
    pub fn broadcast_local(&self, channel: &str, frame: WireEnvelope, except: Option<&str>) {
        let channel = channel.to_lowercase();
        let inner = self.inner.read();
        for (id, entry) in inner.conns.iter() {
            if entry.channel != channel {
                continue;
            }
            if Some(id.as_str()) == except {
                continue;
            }
            entry.sink.send(frame.clone());
        }
    }

    /// Deliver to every member of `(channel, room)`, optionally excluding one id.
    /// Recorded as channel history (replay is channel-wide) and published to
    /// peers; peers re-deliver via [`to_room_local`](Self::to_room_local).
    pub fn to_room(&self, channel: &str, room: &str, frame: WireEnvelope, except: Option<&str>) {
        self.record_history(&channel.to_lowercase(), &frame);
        self.publish(BrokerMsg::ToRoom {
            channel: channel.to_string(),
            room: room.to_string(),
            frame: frame.clone(),
            except: except.map(str::to_string),
        });
        self.to_room_local(channel, room, frame, except);
    }

    /// Local-only room fan-out — no history, no publish. Used by the originating
    /// node and by peers receiving a `ToRoom` message. A peer only knows its own
    /// local room members (membership is not replicated; delivery is re-fan-out),
    /// so each node delivers to whoever it has in the room.
    pub fn to_room_local(
        &self,
        channel: &str,
        room: &str,
        frame: WireEnvelope,
        except: Option<&str>,
    ) {
        let channel = channel.to_lowercase();
        let room = room.to_lowercase();
        let inner = self.inner.read();
        if let Some(set) = inner.rooms.get(&(channel, room)) {
            for id in set.iter() {
                if Some(id.as_str()) == except {
                    continue;
                }
                if let Some(entry) = inner.conns.get(id) {
                    entry.sink.send(frame.clone());
                }
            }
        }
    }

    pub fn close_conn(&self, conn_id: &str, code: u16, reason: String) {
        let inner = self.inner.read();
        if let Some(entry) = inner.conns.get(conn_id) {
            entry.sink.close(code, reason);
        }
    }

    /// Relay a client-originated "whisper" frame (a `client-*` event) to other
    /// connections **without running any CFML code and without recording
    /// history** (design principle: typing/cursor chatter never enters the
    /// request path — Phase 4). Like a normal broadcast it crosses nodes via the
    /// `Broker` (peers re-fan-out), but it is ephemeral, so it is never added to
    /// the replay log. `room = Some(r)` narrows the fan-out to a single room;
    /// `None` is channel-wide. `except` is the sender (whispers are never echoed
    /// to their author).
    pub fn relay(
        &self,
        channel: &str,
        room: Option<&str>,
        frame: WireEnvelope,
        except: Option<&str>,
    ) {
        match room {
            Some(r) => {
                self.publish(BrokerMsg::ToRoom {
                    channel: channel.to_string(),
                    room: r.to_string(),
                    frame: frame.clone(),
                    except: except.map(str::to_string),
                });
                self.to_room_local(channel, r, frame, except);
            }
            None => {
                self.publish(BrokerMsg::Broadcast {
                    channel: channel.to_string(),
                    frame: frame.clone(),
                    except: except.map(str::to_string),
                });
                self.broadcast_local(channel, frame, except);
            }
        }
    }

    // ── resumability / history (design principle P12) ─────────────────────

    /// Enable (or update) the retained-history cap for a channel. Called once at
    /// channel discovery / first connect from the `history="N"` attribute. A cap
    /// of 0 leaves history disabled — the common case stays lock-free.
    pub fn set_history_cap(&self, channel: &str, cap: usize) {
        if cap == 0 {
            return;
        }
        self.history_enabled.store(true, Ordering::Relaxed);
        self.inner
            .write()
            .history_cap
            .insert(channel.to_lowercase(), cap);
    }

    /// Append a channel-wide fan-out frame to the channel's history ring,
    /// trimming to the cap. No-op unless the channel opted in. Takes the write
    /// lock briefly; callers must not hold any registry lock across this.
    fn record_history(&self, channel: &str, frame: &WireEnvelope) {
        if !self.history_enabled.load(Ordering::Relaxed) {
            return;
        }
        let channel = channel.to_lowercase();
        let mut inner = self.inner.write();
        let cap = inner.history_cap.get(&channel).copied().unwrap_or(0);
        if cap == 0 {
            return;
        }
        let ring = inner.history.entry(channel).or_default();
        ring.push_back(frame.clone());
        while ring.len() > cap {
            ring.pop_front();
        }
    }

    /// Replay the channel-history frames a reconnecting connection missed, in
    /// order, ahead of live traffic. `last_event_id` is the client's cursor
    /// (`{nodeId}:{seq}`): every retained frame with a larger `seq` is re-sent
    /// to `conn_id`, keeping its original id so the client can keep advancing.
    ///
    /// Best-effort, single-node: a `last_event_id` minted by a *different* node
    /// (failover) is skipped — cluster-correct replay arrives with the
    /// distributed `Broker`. If the cursor is older than the oldest retained
    /// frame the client has provably lost messages, so a `t="reset"` hint frame
    /// is sent first (Socket.IO "recovered: false"). Returns `false` when replay
    /// was skipped (unparseable id or cross-node), `true` otherwise.
    pub fn replay_since(&self, channel: &str, last_event_id: &str, conn_id: &str) -> bool {
        let (node, last_seq) = match parse_event_id(last_event_id) {
            Some(p) => p,
            None => return false,
        };
        if node != &**self.node_id.read() {
            return false;
        }
        let channel_id = channel.to_string();
        let (reset, frames): (bool, Vec<WireEnvelope>) = {
            let channel_l = channel.to_lowercase();
            let inner = self.inner.read();
            match inner.history.get(&channel_l) {
                Some(ring) if !ring.is_empty() => {
                    let oldest = ring
                        .iter()
                        .filter_map(|f| parse_event_id(&f.id).map(|(_, s)| s))
                        .min()
                        .unwrap_or(0);
                    let frames = ring
                        .iter()
                        .filter(|f| {
                            parse_event_id(&f.id).map(|(_, s)| s > last_seq).unwrap_or(false)
                        })
                        .cloned()
                        .collect();
                    (last_seq < oldest, frames)
                }
                _ => (false, Vec::new()),
            }
        };
        if reset {
            self.emit_to(
                conn_id,
                WireEnvelope {
                    t: "reset".to_string(),
                    ch: channel_id,
                    ev: None,
                    d: CfmlValue::Null,
                    id: self.next_id(),
                    ref_id: None,
                },
            );
        }
        for f in frames {
            self.emit_to(conn_id, f);
        }
        true
    }

    // ── presence (design principle P11) ───────────────────────────────────

    /// Track `conn_id` in its channel's presence roster under `key`, carrying
    /// `meta`. The tracking connection immediately gets the full `presence_state`
    /// snapshot (so it sees who is already here); everyone else gets a
    /// `presence_diff` join. Re-tracking under the same key replaces the meta
    /// (an update). Cluster-correctness comes free once the distributed `Broker`
    /// fans these through the shared-session cluster.
    pub fn track(&self, conn_id: &str, key: &str, meta: CfmlValue) {
        let channel = {
            let mut inner = self.inner.write();
            let channel = match inner.conns.get(conn_id) {
                Some(e) => e.channel.clone(),
                None => return,
            };
            if let Some(e) = inner.conns.get_mut(conn_id) {
                e.presence_keys.insert(key.to_string());
            }
            inner
                .presence
                .entry((channel.clone(), key.to_string()))
                .or_default()
                .insert(conn_id.to_string(), meta.clone());
            channel
        };
        // Snapshot to the tracking connection, join diff to the rest.
        let state = self.presence_state(&channel);
        let state_frame = WireEnvelope {
            t: "presence".to_string(),
            ch: channel.clone(),
            ev: Some("presence_state".to_string()),
            d: state,
            id: self.next_id(),
            ref_id: None,
        };
        self.emit_to(conn_id, state_frame);
        let join = self.presence_diff_frame(&channel, true, key, &meta);
        // The join diff reaches remote clients via this broadcast's auto-publish;
        // the separate PresenceTrack replicates the *roster* so remote
        // `presence_state()` is cluster-correct (it merges state, no re-broadcast).
        self.broadcast(&channel, join, Some(conn_id));
        self.publish(BrokerMsg::PresenceTrack {
            channel,
            key: key.to_string(),
            conn_id: conn_id.to_string(),
            meta,
        });
    }

    /// Remove `conn_id`'s presence under `key` and broadcast a `presence_diff`
    /// leave to the channel. No-op if it wasn't tracked.
    pub fn untrack(&self, conn_id: &str, key: &str) {
        let removed = {
            let mut inner = self.inner.write();
            let channel = match inner.conns.get(conn_id) {
                Some(e) => e.channel.clone(),
                None => return,
            };
            if let Some(e) = inner.conns.get_mut(conn_id) {
                e.presence_keys.remove(key);
            }
            let map_key = (channel.clone(), key.to_string());
            let meta = inner.presence.get_mut(&map_key).and_then(|m| m.remove(conn_id));
            if inner.presence.get(&map_key).is_some_and(|m| m.is_empty()) {
                inner.presence.remove(&map_key);
            }
            meta.map(|m| (channel, m))
        };
        if let Some((channel, meta)) = removed {
            let leave = self.presence_diff_frame(&channel, false, key, &meta);
            self.broadcast(&channel, leave, None);
            self.publish(BrokerMsg::PresenceUntrack {
                channel,
                key: key.to_string(),
                conn_id: conn_id.to_string(),
            });
        }
    }

    /// The full presence roster for a channel as a CFML struct:
    /// `{ "<key>": { metas: [ meta, … ] }, … }`. Same shape as a
    /// `presence_state` frame's payload.
    pub fn presence_state(&self, channel: &str) -> CfmlValue {
        let channel = channel.to_lowercase();
        let inner = self.inner.read();
        let mut out = ValueMap::default();
        for ((ch, key), metas) in inner.presence.iter() {
            if ch != &channel {
                continue;
            }
            let arr: Vec<CfmlValue> = metas.values().cloned().collect();
            let mut entry = ValueMap::default();
            entry.insert("metas".to_string(), CfmlValue::array(arr));
            out.insert(key.clone(), CfmlValue::strukt(entry));
        }
        CfmlValue::strukt(out)
    }

    /// Build a `presence_diff` frame. `join == true` puts the affected key under
    /// `joins`, otherwise under `leaves` — the side not in play is an empty struct
    /// (stable shape for clients).
    fn presence_diff_frame(
        &self,
        channel: &str,
        join: bool,
        key: &str,
        meta: &CfmlValue,
    ) -> WireEnvelope {
        let mut side = ValueMap::default();
        let mut entry = ValueMap::default();
        entry.insert("metas".to_string(), CfmlValue::array(vec![meta.clone()]));
        side.insert(key.to_string(), CfmlValue::strukt(entry));
        let mut d = ValueMap::default();
        if join {
            d.insert("joins".to_string(), CfmlValue::strukt(side));
            d.insert("leaves".to_string(), CfmlValue::strukt(ValueMap::default()));
        } else {
            d.insert("joins".to_string(), CfmlValue::strukt(ValueMap::default()));
            d.insert("leaves".to_string(), CfmlValue::strukt(side));
        }
        WireEnvelope {
            t: "presence".to_string(),
            ch: channel.to_string(),
            ev: Some("presence_diff".to_string()),
            d: CfmlValue::strukt(d),
            id: self.next_id(),
            ref_id: None,
        }
    }

    // ── distributed broker inbound (Phase 2 — cluster fan-out) ────────────

    /// Apply a [`BrokerMsg`] received from a peer node. The cli cluster adapter
    /// decodes the gossip frame and hands it here; this is the *only* inbound
    /// entry point. Every variant routes to a local-only method so re-delivery
    /// never re-publishes (no fan-out loop).
    pub fn apply_broker_msg(&self, msg: BrokerMsg) {
        match msg {
            BrokerMsg::Broadcast { channel, frame, except } => {
                self.broadcast_local(&channel, frame, except.as_deref());
            }
            BrokerMsg::ToRoom { channel, room, frame, except } => {
                self.to_room_local(&channel, &room, frame, except.as_deref());
            }
            BrokerMsg::EmitTo { conn_id, frame } => {
                // Deliver only if we own the connection; other nodes no-op.
                let inner = self.inner.read();
                if let Some(entry) = inner.conns.get(&conn_id) {
                    entry.sink.send(frame);
                }
            }
            BrokerMsg::PresenceTrack { channel, key, conn_id, meta } => {
                self.apply_remote_track(channel, key, conn_id, meta);
            }
            BrokerMsg::PresenceUntrack { channel, key, conn_id } => {
                self.apply_remote_untrack(channel, key, conn_id);
            }
            BrokerMsg::NodeGone { node_id } => self.drop_node(&node_id),
        }
    }

    /// Merge a remote presence-roster entry. Roster state only — the
    /// client-facing `presence_diff` arrived via the peer's separate `Broadcast`
    /// of the diff frame, so we must NOT re-broadcast here (that would double the
    /// diff for our local clients). `presence_state()` now reflects the merged
    /// cluster-wide roster.
    fn apply_remote_track(&self, channel: String, key: String, conn_id: String, meta: CfmlValue) {
        let channel = channel.to_lowercase();
        let mut inner = self.inner.write();
        inner
            .presence
            .entry((channel, key))
            .or_default()
            .insert(conn_id, meta);
    }

    /// Remove a remote presence-roster entry (roster state only; the leave diff
    /// rode the peer's `Broadcast`).
    fn apply_remote_untrack(&self, channel: String, key: String, conn_id: String) {
        let channel = channel.to_lowercase();
        let map_key = (channel, key);
        let mut inner = self.inner.write();
        if let Some(map) = inner.presence.get_mut(&map_key) {
            map.remove(&conn_id);
            if map.is_empty() {
                inner.presence.remove(&map_key);
            }
        }
    }

    /// A peer left the cluster: evict every roster entry it owned (conn ids with
    /// its `{node_id}:` prefix) and emit a leave diff to our local clients so
    /// their rosters converge. Membership/rooms for that node need no cleanup —
    /// we never replicated remote room membership (delivery is re-fan-out).
    pub fn drop_node(&self, node_id: &str) {
        let prefix = format!("{}:", node_id);
        let leaves: Vec<(String, String, CfmlValue)> = {
            let mut inner = self.inner.write();
            let mut leaves = Vec::new();
            let mut empty_keys = Vec::new();
            for ((channel, key), map) in inner.presence.iter_mut() {
                let gone: Vec<ConnId> =
                    map.keys().filter(|c| c.starts_with(&prefix)).cloned().collect();
                for c in gone {
                    if let Some(meta) = map.remove(&c) {
                        leaves.push((channel.clone(), key.clone(), meta));
                    }
                }
                if map.is_empty() {
                    empty_keys.push((channel.clone(), key.clone()));
                }
            }
            for k in empty_keys {
                inner.presence.remove(&k);
            }
            leaves
        };
        for (channel, key, meta) in leaves {
            let leave = self.presence_diff_frame(&channel, false, &key, &meta);
            self.broadcast_local(&channel, leave, None);
        }
    }

    // ── introspection ─────────────────────────────────────────────────────

    pub fn channel_count(&self, channel: &str) -> usize {
        let channel = channel.to_lowercase();
        self.inner
            .read()
            .conns
            .values()
            .filter(|e| e.channel == channel)
            .count()
    }

    pub fn room_count(&self, channel: &str, room: &str) -> usize {
        let channel = channel.to_lowercase();
        let room = room.to_lowercase();
        self.inner
            .read()
            .rooms
            .get(&(channel, room))
            .map(|s| s.len())
            .unwrap_or(0)
    }

    pub fn channel_sockets(&self, channel: &str) -> Vec<String> {
        let channel = channel.to_lowercase();
        self.inner
            .read()
            .conns
            .iter()
            .filter(|(_, e)| e.channel == channel)
            .map(|(id, _)| id.clone())
            .collect()
    }

    pub fn room_sockets(&self, channel: &str, room: &str) -> Vec<String> {
        let channel = channel.to_lowercase();
        let room = room.to_lowercase();
        self.inner
            .read()
            .rooms
            .get(&(channel, room))
            .map(|s| s.iter().cloned().collect())
            .unwrap_or_default()
    }
}

// ──────────────────────────────────────────────────────────────────────────
// CFML-facing NativeObjects
// ──────────────────────────────────────────────────────────────────────────

/// The live `socket` handle passed to channel lifecycle/event methods.
/// Wraps `(conn_id, channel, registry)` and maps `id/send/emit/broadcast/
/// join/leave/rooms/to/close/data` onto registry ops. Same surface for raw-WS
/// and socket.io.
#[derive(Debug)]
pub struct SocketHandle {
    pub conn_id: ConnId,
    pub channel: String,
    pub registry: Arc<WebSocketRegistry>,
}

impl SocketHandle {
    pub fn new(conn_id: ConnId, channel: String, registry: Arc<WebSocketRegistry>) -> Self {
        Self { conn_id, channel, registry }
    }
}

/// Split a `{nodeId}:{seq}` wire/cursor id into its node prefix and numeric
/// sequence. The node prefix is everything before the final `:` (node ids are
/// uuids and carry no colons, so the split is unambiguous).
fn parse_event_id(id: &str) -> Option<(&str, u64)> {
    let (node, seq) = id.rsplit_once(':')?;
    let seq = seq.parse::<u64>().ok()?;
    Some((node, seq))
}

fn arg_string(args: &[CfmlValue], idx: usize) -> String {
    args.get(idx).map(|v| v.as_string()).unwrap_or_default()
}

fn arg_value(args: &[CfmlValue], idx: usize) -> CfmlValue {
    args.get(idx).cloned().unwrap_or(CfmlValue::Null)
}

impl CfmlNative for SocketHandle {
    fn class_name(&self) -> &str {
        "Socket"
    }

    fn call_method(&mut self, name: &str, args: Vec<CfmlValue>) -> CfmlResult {
        match name.to_ascii_lowercase().as_str() {
            "id" => Ok(CfmlValue::string(self.conn_id.clone())),
            "channel" => Ok(CfmlValue::string(self.channel.clone())),
            "send" => {
                let frame = self.registry.msg(&self.channel, None, arg_value(&args, 0));
                self.registry.emit_to(&self.conn_id, frame);
                Ok(CfmlValue::Null)
            }
            "emit" => {
                let frame =
                    self.registry
                        .msg(&self.channel, Some(arg_string(&args, 0)), arg_value(&args, 1));
                self.registry.emit_to(&self.conn_id, frame);
                Ok(CfmlValue::Null)
            }
            "broadcast" => {
                // To everyone in the channel except the sender.
                let frame =
                    self.registry
                        .msg(&self.channel, Some(arg_string(&args, 0)), arg_value(&args, 1));
                self.registry.broadcast(&self.channel, frame, Some(&self.conn_id));
                Ok(CfmlValue::Null)
            }
            "join" => {
                self.registry.join(&self.conn_id, &arg_string(&args, 0));
                Ok(CfmlValue::Null)
            }
            "leave" => {
                self.registry.leave(&self.conn_id, &arg_string(&args, 0));
                Ok(CfmlValue::Null)
            }
            "rooms" => Ok(CfmlValue::array(
                self.registry
                    .rooms_of(&self.conn_id)
                    .into_iter()
                    .map(CfmlValue::string)
                    .collect(),
            )),
            // Presence (P11). `track(meta)` keys on the connection id;
            // `track(key, meta)` groups several connections (e.g. a user's tabs)
            // under one key. The tracking client gets a `presence_state`
            // snapshot, others a `presence_diff` join.
            "track" => {
                let (key, meta) = match args.as_slice() {
                    [CfmlValue::Struct(_), ..] => (self.conn_id.clone(), arg_value(&args, 0)),
                    [k, m, ..] => (k.as_string(), m.clone()),
                    [k] => (k.as_string(), CfmlValue::strukt(ValueMap::default())),
                    [] => (self.conn_id.clone(), CfmlValue::strukt(ValueMap::default())),
                };
                self.registry.track(&self.conn_id, &key, meta);
                Ok(CfmlValue::Null)
            }
            "untrack" => {
                let key = if args.is_empty() {
                    self.conn_id.clone()
                } else {
                    arg_string(&args, 0)
                };
                self.registry.untrack(&self.conn_id, &key);
                Ok(CfmlValue::Null)
            }
            "presence" => Ok(self.registry.presence_state(&self.channel)),
            "to" | "in" => {
                let emitter = ServerEmitter {
                    channel: self.channel.clone(),
                    room: Some(arg_string(&args, 0)),
                    except: None,
                    registry: self.registry.clone(),
                };
                Ok(CfmlValue::NativeObject(Arc::new(StdRwLock::new(emitter))))
            }
            "close" => {
                let code = args
                    .first()
                    .map(|v| v.as_string().parse::<u16>().unwrap_or(1000))
                    .unwrap_or(1000);
                let reason = arg_string(&args, 1);
                self.registry.close_conn(&self.conn_id, code, reason);
                Ok(CfmlValue::Null)
            }
            "data" => Ok(self
                .registry
                .data_of(&self.conn_id)
                .map(CfmlValue::Struct)
                .unwrap_or(CfmlValue::Null)),
            // Handshake query parameter, e.g. socket.param("userId") for ?userId=42.
            "param" => {
                let key = arg_string(&args, 0);
                let val = self
                    .registry
                    .params_of(&self.conn_id)
                    .and_then(|p| {
                        p.iter()
                            .find(|(k, _)| k.eq_ignore_ascii_case(&key))
                            .map(|(_, v)| v.clone())
                    })
                    .unwrap_or(CfmlValue::Null);
                Ok(val)
            }
            "params" => Ok(self
                .registry
                .params_of(&self.conn_id)
                .map(CfmlValue::strukt)
                .unwrap_or_else(|| CfmlValue::strukt(ValueMap::default()))),
            // Session identity resolved at the handshake (P6). The full live
            // session scope + Preside auth helpers (isWebUser/...) ride the
            // socket.io compat layer in a later phase.
            "sessionid" => Ok(self
                .registry
                .session_id_of(&self.conn_id)
                .map(CfmlValue::string)
                .unwrap_or(CfmlValue::Null)),
            other => Err(CfmlError::runtime(format!("Socket has no method [{}]", other))),
        }
    }

    fn get_property(&self, name: &str) -> Option<CfmlValue> {
        match name.to_ascii_lowercase().as_str() {
            "id" => Some(CfmlValue::string(self.conn_id.clone())),
            "channel" => Some(CfmlValue::string(self.channel.clone())),
            "data" => self.registry.data_of(&self.conn_id).map(CfmlValue::Struct),
            "sessionid" => Some(
                self.registry
                    .session_id_of(&self.conn_id)
                    .map(CfmlValue::string)
                    .unwrap_or(CfmlValue::Null),
            ),
            _ => None,
        }
    }
}

/// A scoped emitter — what `io(channel)`, `io(channel).to(room)`, and
/// `socket.to(room)` return. The fluent target chain (design principle P4):
/// `io("/chat").to("lobby").except(id).emit(event, data)`.
#[derive(Debug)]
pub struct ServerEmitter {
    pub channel: String,
    pub room: Option<String>,
    pub except: Option<ConnId>,
    pub registry: Arc<WebSocketRegistry>,
}

impl ServerEmitter {
    pub fn new(channel: String, registry: Arc<WebSocketRegistry>) -> Self {
        Self { channel, room: None, except: None, registry }
    }

    fn deliver(&self, frame: WireEnvelope) {
        match &self.room {
            Some(room) => self.registry.to_room(&self.channel, room, frame, self.except.as_deref()),
            None => self.registry.broadcast(&self.channel, frame, self.except.as_deref()),
        }
    }
}

impl CfmlNative for ServerEmitter {
    fn class_name(&self) -> &str {
        "WsEmitter"
    }

    fn call_method(&mut self, name: &str, args: Vec<CfmlValue>) -> CfmlResult {
        match name.to_ascii_lowercase().as_str() {
            "to" | "in" => {
                let emitter = ServerEmitter {
                    channel: self.channel.clone(),
                    room: Some(arg_string(&args, 0)),
                    except: self.except.clone(),
                    registry: self.registry.clone(),
                };
                Ok(CfmlValue::NativeObject(Arc::new(StdRwLock::new(emitter))))
            }
            "except" | "exclude" => {
                let emitter = ServerEmitter {
                    channel: self.channel.clone(),
                    room: self.room.clone(),
                    except: Some(arg_string(&args, 0)),
                    registry: self.registry.clone(),
                };
                Ok(CfmlValue::NativeObject(Arc::new(StdRwLock::new(emitter))))
            }
            "emit" => {
                let frame =
                    self.registry
                        .msg(&self.channel, Some(arg_string(&args, 0)), arg_value(&args, 1));
                self.deliver(frame);
                Ok(CfmlValue::Null)
            }
            "send" => {
                let frame = self.registry.msg(&self.channel, None, arg_value(&args, 0));
                self.deliver(frame);
                Ok(CfmlValue::Null)
            }
            "count" => {
                let n = match &self.room {
                    Some(room) => self.registry.room_count(&self.channel, room),
                    None => self.registry.channel_count(&self.channel),
                };
                Ok(CfmlValue::Int(n as i64))
            }
            "sockets" => {
                let ids = match &self.room {
                    Some(room) => self.registry.room_sockets(&self.channel, room),
                    None => self.registry.channel_sockets(&self.channel),
                };
                Ok(CfmlValue::array(ids.into_iter().map(CfmlValue::string).collect()))
            }
            // The full presence roster for the channel (room scoping is ignored —
            // presence is channel-level in this phase).
            "presence" => Ok(self.registry.presence_state(&self.channel)),
            other => Err(CfmlError::runtime(format!("WsEmitter has no method [{}]", other))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[derive(Debug, Default)]
    struct CapturingSink {
        frames: Arc<Mutex<Vec<WireEnvelope>>>,
        closed: Arc<Mutex<Option<(u16, String)>>>,
    }

    impl FrameSink for CapturingSink {
        fn send(&self, frame: WireEnvelope) {
            self.frames.lock().unwrap().push(frame);
        }
        fn close(&self, code: u16, reason: String) {
            *self.closed.lock().unwrap() = Some((code, reason));
        }
    }

    fn sink() -> (Arc<CapturingSink>, Arc<dyn FrameSink>) {
        let s = Arc::new(CapturingSink::default());
        let dyn_s: Arc<dyn FrameSink> = s.clone();
        (s, dyn_s)
    }

    #[test]
    fn conn_id_is_node_qualified() {
        let reg = WebSocketRegistry::new("n1");
        let (_s, sink) = sink();
        let id = reg.register("/chat", sink, None, ValueMap::default());
        assert!(id.starts_with("n1:"), "conn id should be node-qualified: {id}");
    }

    #[test]
    fn broadcast_excludes_sender() {
        let reg = Arc::new(WebSocketRegistry::new("n1"));
        let (a, a_dyn) = sink();
        let (b, b_dyn) = sink();
        let id_a = reg.register("/chat", a_dyn, None, ValueMap::default());
        let _id_b = reg.register("/chat", b_dyn, None, ValueMap::default());
        let frame = reg.msg("/chat", Some("hi".into()), CfmlValue::string("x"));
        reg.broadcast("/chat", frame, Some(&id_a));
        assert_eq!(a.frames.lock().unwrap().len(), 0, "sender excluded");
        assert_eq!(b.frames.lock().unwrap().len(), 1, "other receives");
    }

    #[test]
    fn relay_whisper_skips_history_and_excludes_sender() {
        // A `client-*` whisper relays to others (channel-wide or room-scoped)
        // but is NEVER added to the replay log (it is ephemeral).
        let reg = Arc::new(WebSocketRegistry::new("n1"));
        reg.set_history_cap("/chat", 50);
        let (a, a_dyn) = sink();
        let (b, b_dyn) = sink();
        let id_a = reg.register("/chat", a_dyn, None, ValueMap::default());
        let _id_b = reg.register("/chat", b_dyn, None, ValueMap::default());

        let mut frame = reg.msg("/chat", Some("client-typing".into()), CfmlValue::Bool(true));
        frame.t = "client".into();
        reg.relay("/chat", None, frame, Some(&id_a));

        assert_eq!(a.frames.lock().unwrap().len(), 0, "whisper not echoed to sender");
        assert_eq!(b.frames.lock().unwrap().len(), 1, "peer receives the whisper");
        assert_eq!(b.frames.lock().unwrap()[0].t, "client", "frame typed as a client relay");

        // A reconnecting client must NOT replay the whisper (history untouched).
        let (rejoin, rejoin_dyn) = sink();
        let id_r = reg.register("/chat", rejoin_dyn, None, ValueMap::default());
        reg.replay_since("/chat", "n1:0", &id_r);
        assert_eq!(rejoin.frames.lock().unwrap().len(), 0, "whisper never retained");
    }

    #[test]
    fn rooms_fan_out_and_cleanup() {
        let reg = Arc::new(WebSocketRegistry::new("n1"));
        let (a, a_dyn) = sink();
        let id_a = reg.register("/chat", a_dyn, None, ValueMap::default());
        reg.join(&id_a, "lobby");
        assert_eq!(reg.room_count("/chat", "lobby"), 1);
        let frame = reg.msg("/chat", Some("e".into()), CfmlValue::Null);
        reg.to_room("/chat", "lobby", frame, None);
        assert_eq!(a.frames.lock().unwrap().len(), 1);
        // Disconnect removes from every room unconditionally.
        let (ch, rooms) = reg.unregister(&id_a).unwrap();
        assert_eq!(ch, "/chat");
        assert!(rooms.contains(&"lobby".to_string()));
        assert_eq!(reg.room_count("/chat", "lobby"), 0);
        assert_eq!(reg.channel_count("/chat"), 0);
    }

    #[test]
    fn presence_track_snapshot_and_leave_on_disconnect() {
        let reg = Arc::new(WebSocketRegistry::new("n1"));
        let (a, a_dyn) = sink();
        let (b, b_dyn) = sink();
        let id_a = reg.register("/chat", a_dyn, None, ValueMap::default());
        let id_b = reg.register("/chat", b_dyn, None, ValueMap::default());

        // A tracks → A gets a presence_state, B gets a join diff.
        let mut meta_a = ValueMap::default();
        meta_a.insert("user", CfmlValue::string("alice"));
        reg.track(&id_a, "alice", CfmlValue::strukt(meta_a));
        assert_eq!(a.frames.lock().unwrap().last().unwrap().ev.as_deref(), Some("presence_state"));
        assert_eq!(b.frames.lock().unwrap().last().unwrap().ev.as_deref(), Some("presence_diff"));

        // Roster lists alice.
        let state = reg.presence_state("/chat");
        let CfmlValue::Struct(s) = state else { panic!("state is a struct") };
        assert!(s.get_ci("alice").is_some(), "alice is present");

        // A disconnects → B gets a leave diff and the roster empties.
        b.frames.lock().unwrap().clear();
        reg.unregister(&id_a);
        let last = b.frames.lock().unwrap().last().unwrap().clone();
        assert_eq!(last.ev.as_deref(), Some("presence_diff"));
        if let CfmlValue::Struct(d) = &last.d {
            let CfmlValue::Struct(leaves) = d.get_ci("leaves").unwrap() else { panic!() };
            assert!(leaves.get_ci("alice").is_some(), "leave diff names alice");
        } else {
            panic!("diff payload is a struct");
        }
        let CfmlValue::Struct(empty) = reg.presence_state("/chat") else { panic!() };
        assert_eq!(empty.keys().len(), 0, "roster empty after disconnect");
    }

    #[test]
    fn replay_since_seq_compare_and_node_skip() {
        let reg = Arc::new(WebSocketRegistry::new("n1"));
        reg.set_history_cap("/chat", 50);
        let (sender, sender_dyn) = sink();
        let id_s = reg.register("/chat", sender_dyn, None, ValueMap::default());

        // Three channel-wide broadcasts are retained in history.
        let mut ids = Vec::new();
        for n in 0..3 {
            let frame = reg.msg("/chat", Some("say".into()), CfmlValue::Int(n));
            ids.push(frame.id.clone());
            reg.broadcast("/chat", frame, None);
        }
        // (sender received all three live)
        assert_eq!(sender.frames.lock().unwrap().len(), 3);

        // A reconnecting client replays only frames newer than its cursor.
        let (rejoin, rejoin_dyn) = sink();
        let id_r = reg.register("/chat", rejoin_dyn, None, ValueMap::default());
        assert!(reg.replay_since("/chat", &ids[0], &id_r), "same-node replay runs");
        let got: Vec<i64> = rejoin
            .frames
            .lock()
            .unwrap()
            .iter()
            .filter_map(|f| if let CfmlValue::Int(i) = f.d { Some(i) } else { None })
            .collect();
        assert_eq!(got, vec![1, 2], "only frames after the cursor replay, in order");

        // A cursor minted by a different node (failover) is skipped entirely.
        let (other, other_dyn) = sink();
        let id_o = reg.register("/chat", other_dyn, None, ValueMap::default());
        assert!(!reg.replay_since("/chat", "n2:1", &id_o), "cross-node cursor skipped");
        assert_eq!(other.frames.lock().unwrap().len(), 0, "nothing replayed cross-node");

        // A cursor older than the retained window gets a reset hint first.
        let (gap, gap_dyn) = sink();
        let id_g = reg.register("/chat", gap_dyn, None, ValueMap::default());
        assert!(reg.replay_since("/chat", "n1:0", &id_g));
        let frames = gap.frames.lock().unwrap();
        assert_eq!(frames.first().unwrap().t, "reset", "gap signalled with a reset frame");

        // Keep the connections referenced to end-of-test (avoid clippy noise).
        let _ = (id_s, id_r, id_o, id_g);
    }

    #[test]
    fn self_room_named_after_id() {
        let reg = Arc::new(WebSocketRegistry::new("n1"));
        let (_a, a_dyn) = sink();
        let id_a = reg.register("/chat", a_dyn, None, ValueMap::default());
        // Auto-joined a room named after its own id.
        assert_eq!(reg.room_count("/chat", &id_a), 1);
    }

    // ── distributed Broker (Phase 2) ──────────────────────────────────────

    #[derive(Debug, Default)]
    struct CapturingBroker {
        published: Arc<Mutex<Vec<BrokerMsg>>>,
    }

    impl Broker for CapturingBroker {
        fn publish(&self, msg: BrokerMsg) {
            self.published.lock().unwrap().push(msg);
        }
    }

    fn broker() -> (Arc<CapturingBroker>, Arc<dyn Broker>) {
        let b = Arc::new(CapturingBroker::default());
        let dyn_b: Arc<dyn Broker> = b.clone();
        (b, dyn_b)
    }

    #[test]
    fn no_broker_means_no_publish_and_local_delivery_unchanged() {
        // The single-node path is byte-for-byte the same: broadcast still
        // delivers locally and nothing is published (no broker installed).
        let reg = Arc::new(WebSocketRegistry::new("n1"));
        let (a, a_dyn) = sink();
        let _id = reg.register("/chat", a_dyn, None, ValueMap::default());
        let frame = reg.msg("/chat", Some("e".into()), CfmlValue::Null);
        reg.broadcast("/chat", frame, None);
        assert_eq!(a.frames.lock().unwrap().len(), 1, "local delivery still happens");
    }

    #[test]
    fn fan_out_publishes_to_broker() {
        let reg = Arc::new(WebSocketRegistry::new("n1"));
        let (b, b_dyn) = broker();
        reg.set_broker(b_dyn);
        let (_s, s_dyn) = sink();
        let id = reg.register("/chat", s_dyn, None, ValueMap::default());
        reg.join(&id, "lobby");

        reg.broadcast("/chat", reg.msg("/chat", Some("e".into()), CfmlValue::Null), None);
        reg.to_room("/chat", "lobby", reg.msg("/chat", Some("e".into()), CfmlValue::Null), None);
        // emit_to a *local* conn must NOT publish (it's delivered directly).
        reg.emit_to(&id, reg.msg("/chat", None, CfmlValue::Null));
        // emit_to a remote (unknown) conn routes through the broker.
        reg.emit_to("n2:ghost", reg.msg("/chat", None, CfmlValue::Null));

        let pubd = b.published.lock().unwrap();
        assert!(matches!(pubd[0], BrokerMsg::Broadcast { .. }), "broadcast published");
        assert!(matches!(pubd[1], BrokerMsg::ToRoom { .. }), "to_room published");
        // 3rd publish is the remote EmitTo; the local emit_to did not publish.
        assert!(
            matches!(pubd.last().unwrap(), BrokerMsg::EmitTo { conn_id, .. } if conn_id == "n2:ghost"),
            "remote emit_to routed via broker"
        );
        assert!(
            !pubd.iter().any(|m| matches!(m, BrokerMsg::EmitTo { conn_id, .. } if conn_id == &id)),
            "local emit_to never published"
        );
    }

    #[test]
    fn apply_broadcast_delivers_locally_without_republishing() {
        let reg = Arc::new(WebSocketRegistry::new("n2"));
        let (b, b_dyn) = broker();
        reg.set_broker(b_dyn);
        let (a, a_dyn) = sink();
        let _id = reg.register("/chat", a_dyn, None, ValueMap::default());

        // A frame minted on node n1 arrives via the broker.
        let frame = WireEnvelope {
            t: "msg".into(),
            ch: "/chat".into(),
            ev: Some("say".into()),
            d: CfmlValue::string("hi"),
            id: "n1:7".into(),
            ref_id: None,
        };
        reg.apply_broker_msg(BrokerMsg::Broadcast {
            channel: "/chat".into(),
            frame,
            except: None,
        });
        assert_eq!(a.frames.lock().unwrap().len(), 1, "remote frame delivered to local conn");
        assert!(b.published.lock().unwrap().is_empty(), "re-delivery must not re-publish");
    }

    #[test]
    fn presence_track_publishes_roster_and_remote_merge_is_visible() {
        let reg = Arc::new(WebSocketRegistry::new("n1"));
        let (b, b_dyn) = broker();
        reg.set_broker(b_dyn);
        let (_a, a_dyn) = sink();
        let id_a = reg.register("/chat", a_dyn, None, ValueMap::default());

        let mut meta = ValueMap::default();
        meta.insert("user", CfmlValue::string("alice"));
        reg.track(&id_a, "alice", CfmlValue::strukt(meta));
        assert!(
            b.published.lock().unwrap().iter().any(|m| matches!(m, BrokerMsg::PresenceTrack { .. })),
            "track replicates the roster entry"
        );

        // A remote node's user shows up in our roster after a PresenceTrack apply.
        let mut bob = ValueMap::default();
        bob.insert("user", CfmlValue::string("bob"));
        reg.apply_broker_msg(BrokerMsg::PresenceTrack {
            channel: "/chat".into(),
            key: "bob".into(),
            conn_id: "n2:xyz".into(),
            meta: CfmlValue::strukt(bob),
        });
        let CfmlValue::Struct(roster) = reg.presence_state("/chat") else { panic!() };
        assert!(roster.get_ci("alice").is_some(), "local alice present");
        assert!(roster.get_ci("bob").is_some(), "remote bob merged into roster");

        // The owning node leaving evicts its entries and emits a local leave diff.
        reg.drop_node("n2");
        let CfmlValue::Struct(after) = reg.presence_state("/chat") else { panic!() };
        assert!(after.get_ci("bob").is_none(), "bob evicted on node-gone");
        assert!(after.get_ci("alice").is_some(), "alice (local) survives");
    }

    /// A broker that forwards every published message straight into a peer
    /// registry's `apply_broker_msg` — an in-process stand-in for the gossip
    /// transport. Deterministically exercises the full cross-node path (no
    /// timing). Two registries wired to each other model a two-node cluster.
    #[derive(Debug)]
    struct LoopbackBroker {
        peer: Arc<WebSocketRegistry>,
    }
    impl Broker for LoopbackBroker {
        fn publish(&self, msg: BrokerMsg) {
            self.peer.apply_broker_msg(msg);
        }
    }

    #[test]
    fn broker_msg_round_trips_through_json() {
        // The cluster adapter encodes BrokerMsg with serde_json (CfmlValue's
        // Deserialize is deserialize_any → needs a self-describing format). Guard
        // that the frame + a struct payload survive the round trip intact.
        let mut payload = ValueMap::default();
        payload.insert("text", CfmlValue::string("hi"));
        payload.insert("n", CfmlValue::Int(42));
        let msg = BrokerMsg::Broadcast {
            channel: "/chat".into(),
            frame: WireEnvelope {
                t: "msg".into(),
                ch: "/chat".into(),
                ev: Some("say".into()),
                d: CfmlValue::strukt(payload),
                id: "nodeA:7".into(),
                ref_id: Some("nodeA:6".into()),
            },
            except: Some("nodeA:abc".into()),
        };
        let json = serde_json::to_string(&msg).expect("encode");
        let back: BrokerMsg = serde_json::from_str(&json).expect("decode");
        match back {
            BrokerMsg::Broadcast { channel, frame, except } => {
                assert_eq!(channel, "/chat");
                assert_eq!(frame.id, "nodeA:7");
                assert_eq!(frame.ref_id.as_deref(), Some("nodeA:6"));
                assert_eq!(except.as_deref(), Some("nodeA:abc"));
                let CfmlValue::Struct(d) = frame.d else { panic!("payload struct") };
                assert_eq!(d.get_ci("text").unwrap().as_string(), "hi");
                assert_eq!(d.get_ci("n").unwrap().as_string(), "42");
            }
            _ => panic!("variant preserved"),
        }
    }

    #[test]
    fn two_node_cluster_fan_out_and_presence() {
        let a = Arc::new(WebSocketRegistry::new("nodeA"));
        let b = Arc::new(WebSocketRegistry::new("nodeB"));
        // Bidirectional loopback (an Arc cycle — fine, the test process exits).
        a.set_broker(Arc::new(LoopbackBroker { peer: b.clone() }));
        b.set_broker(Arc::new(LoopbackBroker { peer: a.clone() }));

        let (sa, sa_dyn) = sink();
        let (sb, sb_dyn) = sink();
        let id_a = a.register("/chat", sa_dyn, None, ValueMap::default());
        let id_b = b.register("/chat", sb_dyn, None, ValueMap::default());

        // Broadcast from node A reaches the client on node B.
        a.broadcast("/chat", a.msg("/chat", Some("say".into()), CfmlValue::string("hi")), None);
        assert_eq!(sa.frames.lock().unwrap().len(), 1, "local A client receives");
        assert_eq!(sb.frames.lock().unwrap().len(), 1, "remote B client receives via broker");

        // Room fan-out crosses nodes: both join "lobby" on their own node.
        a.join(&id_a, "lobby");
        b.join(&id_b, "lobby");
        sa.frames.lock().unwrap().clear();
        sb.frames.lock().unwrap().clear();
        a.to_room("/chat", "lobby", a.msg("/chat", Some("e".into()), CfmlValue::Null), None);
        assert_eq!(sa.frames.lock().unwrap().len(), 1, "A lobby member receives");
        assert_eq!(sb.frames.lock().unwrap().len(), 1, "B lobby member receives cross-node");

        // Presence roster is cluster-correct: A's tracked user shows on node B.
        let mut meta = ValueMap::default();
        meta.insert("user", CfmlValue::string("alice"));
        a.track(&id_a, "alice", CfmlValue::strukt(meta));
        let CfmlValue::Struct(roster_b) = b.presence_state("/chat") else { panic!() };
        assert!(roster_b.get_ci("alice").is_some(), "alice present in node B roster");

        // A client disconnecting on node A drops it from node B's roster too.
        a.unregister(&id_a);
        let CfmlValue::Struct(roster_b2) = b.presence_state("/chat") else { panic!() };
        assert!(roster_b2.get_ci("alice").is_none(), "alice gone from B after A-side disconnect");
    }
}
