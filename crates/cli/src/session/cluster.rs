//! Native gossip-backed clustered session store.
//!
//! Enabled by the `cluster` Cargo feature. Native rustcfml nodes only —
//! not available in WASM builds.
//!
//! Architecture (see `SharedSessionImplementation.md` Phase 3):
//! - `memberlist` 0.8 provides cluster membership, Phi-accrual failure
//!   detection, and gossip-grade message delivery over TCP.
//! - `automerge` 0.9 provides per-session conflict-free merging.
//! - One `AutoCommit` Automerge doc per CFML session id, holding a single
//!   `data` field whose value is the JSON-encoded `SessionData`.
//! - Live mutations: encode the doc's incremental change and reliably push
//!   it to every currently-online peer.
//! - Anti-entropy: memberlist's TCP push/pull periodically calls
//!   `local_state` / `merge_remote_state` on our delegate, which round-trip
//!   the union of all docs. This catches anything dropped on the live path
//!   and seeds new joiners.

#[cfg(feature = "cluster")]
mod inner {
    use bincode::{
        config::standard as bincode_std,
        serde::{decode_from_slice, encode_to_vec},
    };
    use cfml_common::dynamic::ValueMap;
    use cfml_vm::{
        SessionData,
        session_store::SessionStore,
        websocket::{Broker, BrokerMsg, WebSocketRegistry},
    };

    use memberlist::{
        Options,
        agnostic::tokio::TokioRuntime,
        bytes::Bytes,
        delegate::{CompositeDelegate, EventKind, NodeDelegate, SubscribleEventDelegate},
        net::{
            NetTransportOptions,
            resolver::socket_addr::SocketAddrResolver,
            stream_layer::tcp::Tcp,
        },
        proto::{MaybeResolvedAddress, Meta, NodeId},
        tokio::TokioTcpMemberlist,
    };
    use serde::{Deserialize, Serialize};
    use indexmap::IndexSet;
    use std::{
        borrow::Cow,
        collections::HashMap,
        net::SocketAddr,
        sync::{Arc, Mutex},
        time::{Duration, Instant},
    };
    use tokio::sync::mpsc;

    const TOMBSTONE_TTL: Duration = Duration::from_secs(60);
    const OUTBOUND_QUEUE_CAP: usize = 1024;

    /// Per-session doc keys (and the opaque `id` on the wire) are the composite
    /// `(application name, session id)` so two apps sharing a CFID map to two
    /// distinct Automerge docs across the whole cluster. Every node computes
    /// the same key, so deltas/tombstones stay consistent. App names are
    /// case-insensitive in CFML — lowercase the app segment.
    fn composite_key(app: &str, id: &str) -> String {
        format!("{}\u{1f}{}", app.to_lowercase(), id)
    }

    /// Recover the bare session id from a composite doc key (everything after
    /// the last unit separator).
    fn id_from_key(key: &str) -> &str {
        key.rsplit('\u{1f}').next().unwrap_or(key)
    }

    // ─────────────────────────────────────────────
    // Wire frames
    // ─────────────────────────────────────────────

    /// One gossip frame on the shared cluster. The cluster multiplexes two
    /// independent subsystems over a single `memberlist` instance/port: session
    /// replication (`Session`) and WebSocket fan-out (`Ws`).
    #[derive(Clone, Serialize, Deserialize)]
    enum ClusterMsg {
        Session(SessionMsg),
        /// A `serde_json`-encoded `cfml_vm::websocket::BrokerMsg`. JSON (not the
        /// surrounding bincode) because the frame payload is a `CfmlValue`, whose
        /// `Deserialize` is `deserialize_any` — it needs a self-describing format;
        /// bincode would fail. Mirrors how `SessionData` is JSON-encoded inside
        /// the Automerge doc.
        Ws(String),
    }

    #[derive(Clone, Serialize, Deserialize)]
    enum SessionMsg {
        /// A change to a session's Automerge doc — incremental bytes from
        /// `AutoCommit::save_incremental()` since the last broadcast.
        Delta { id: String, change: Vec<u8> },
        /// Session removed (invalidate / expire). Receivers drop the local
        /// doc and remember the id briefly so late `Delta`s are ignored.
        Tombstone { id: String },
    }

    /// State serialised by `local_state` and consumed by
    /// `merge_remote_state`. Each entry is the full `AutoCommit::save()` of
    /// one session doc.
    #[derive(Serialize, Deserialize)]
    struct StateBundle {
        sessions: Vec<(String, Vec<u8>)>,
        tombstones: Vec<String>,
    }

    // ─────────────────────────────────────────────
    // Shared state — held by both ClusterStore and the delegate
    // ─────────────────────────────────────────────

    struct SharedState {
        /// Per-session Automerge documents.
        docs: Mutex<HashMap<String, automerge::AutoCommit>>,
        /// Recently-removed session ids; late deltas for these are ignored.
        tombstones: Mutex<HashMap<String, Instant>>,
        /// Stable node identifier (used for logs and tie-breaking).
        node_name: String,
        /// Outbound message queue — drained by the background sender task.
        tx: mpsc::Sender<ClusterMsg>,
        /// The WebSocket registry, so inbound `Ws` frames and peer departures
        /// reach it. `None` if the cluster carries sessions only.
        ws_registry: Option<Arc<WebSocketRegistry>>,
    }

    impl SharedState {
        fn gc_tombstones(&self) {
            if let Ok(mut t) = self.tombstones.lock() {
                let now = Instant::now();
                t.retain(|_, when| now.duration_since(*when) < TOMBSTONE_TTL);
            }
        }

        /// Route an incoming gossip frame to the right subsystem.
        fn dispatch(&self, msg: ClusterMsg) {
            match msg {
                ClusterMsg::Session(s) => self.apply_session(s),
                ClusterMsg::Ws(json) => {
                    let Some(reg) = &self.ws_registry else { return };
                    match serde_json::from_str::<BrokerMsg>(&json) {
                        Ok(m) => reg.apply_broker_msg(m),
                        Err(e) => {
                            eprintln!("[cluster/ws] malformed broker message: {}", e);
                        }
                    }
                }
            }
        }

        /// Apply an incoming live session message to local state. Returns
        /// nothing — the live message is NOT re-broadcast (every peer received
        /// the original from its sender).
        fn apply_session(&self, msg: SessionMsg) {
            match msg {
                SessionMsg::Delta { id, change } => {
                    if self
                        .tombstones
                        .lock()
                        .map(|t| t.contains_key(&id))
                        .unwrap_or(false)
                    {
                        return;
                    }
                    if let Ok(mut docs) = self.docs.lock() {
                        let doc = docs
                            .entry(id.clone())
                            .or_insert_with(automerge::AutoCommit::new);
                        if let Err(e) = doc.load_incremental(&change) {
                            eprintln!(
                                "[session/cluster] failed to apply delta for session {}: {}",
                                id, e
                            );
                        }
                    }
                }
                SessionMsg::Tombstone { id } => {
                    if let Ok(mut docs) = self.docs.lock() {
                        docs.remove(&id);
                    }
                    if let Ok(mut t) = self.tombstones.lock() {
                        t.insert(id, Instant::now());
                    }
                }
            }
        }

        /// Snapshot every local doc plus tombstones for `local_state`.
        fn snapshot(&self) -> Vec<u8> {
            self.gc_tombstones();
            let sessions: Vec<(String, Vec<u8>)> = match self.docs.lock() {
                Ok(mut docs) => docs
                    .iter_mut()
                    .map(|(id, doc)| (id.clone(), doc.save()))
                    .collect(),
                Err(_) => Vec::new(),
            };
            let tombstones: Vec<String> = match self.tombstones.lock() {
                Ok(t) => t.keys().cloned().collect(),
                Err(_) => Vec::new(),
            };
            let bundle = StateBundle { sessions, tombstones };
            encode_to_vec(&bundle, bincode_std()).unwrap_or_default()
        }

        /// Merge a remote `StateBundle` into local docs.
        fn merge_snapshot(&self, buf: &[u8]) {
            let bundle: StateBundle = match decode_from_slice(buf, bincode_std()) {
                Ok((b, _)) => b,
                Err(e) => {
                    eprintln!("[session/cluster] malformed remote state: {}", e);
                    return;
                }
            };
            // Honour tombstones — drop matching ids locally, remember them.
            if let Ok(mut t) = self.tombstones.lock() {
                for id in &bundle.tombstones {
                    t.entry(id.clone()).or_insert_with(Instant::now);
                }
            }
            if let Ok(mut docs) = self.docs.lock() {
                for id in &bundle.tombstones {
                    docs.remove(id);
                }
                for (id, raw) in bundle.sessions {
                    if self
                        .tombstones
                        .lock()
                        .map(|t| t.contains_key(&id))
                        .unwrap_or(false)
                    {
                        continue;
                    }
                    match docs.get_mut(&id) {
                        Some(doc) => {
                            if let Err(e) = doc.load_incremental(&raw) {
                                eprintln!(
                                    "[session/cluster] merge_incremental failed for {}: {}",
                                    id, e
                                );
                            }
                        }
                        None => match automerge::AutoCommit::load(&raw) {
                            Ok(loaded) => {
                                docs.insert(id, loaded);
                            }
                            Err(e) => {
                                eprintln!(
                                    "[session/cluster] AutoCommit::load failed for {}: {}",
                                    id, e
                                );
                            }
                        },
                    }
                }
            }
        }
    }

    // ─────────────────────────────────────────────
    // Delegate
    // ─────────────────────────────────────────────

    struct ClusterDelegate {
        shared: Arc<SharedState>,
    }

    impl NodeDelegate for ClusterDelegate {
        async fn node_meta(&self, _limit: usize) -> Meta {
            Meta::empty()
        }

        async fn notify_message(&self, msg: Cow<'_, [u8]>) {
            let decoded: ClusterMsg = match decode_from_slice(&msg, bincode_std()) {
                Ok((m, _)) => m,
                Err(e) => {
                    eprintln!("[session/cluster] malformed incoming message: {}", e);
                    return;
                }
            };
            self.shared.dispatch(decoded);
        }

        async fn broadcast_messages<F>(
            &self,
            _limit: usize,
            _encoded_len: F,
        ) -> impl Iterator<Item = Bytes> + Send
        where
            F: Fn(Bytes) -> (usize, Bytes) + Send + Sync + 'static,
        {
            // We push live deltas via direct send_reliable from the sender
            // task; we don't piggyback on gossip. Returning an empty
            // iterator is the documented way to opt out.
            std::iter::empty()
        }

        async fn local_state(&self, _join: bool) -> Bytes {
            Bytes::from(self.shared.snapshot())
        }

        async fn merge_remote_state(&self, buf: &[u8], _join: bool) {
            self.shared.merge_snapshot(buf);
        }
    }

    // ─────────────────────────────────────────────
    // SessionData ⇄ Automerge bytes-blob (v1)
    // ─────────────────────────────────────────────
    //
    // For Phase 3 v1 we store the entire `SessionData` as a single
    // JSON-encoded byte field inside each per-session Automerge doc.
    // This gives us:
    // - deterministic last-write-wins merging when two nodes mutate
    //   the same session concurrently (Automerge orders by change hash)
    // - small incremental change records (the only field that ever
    //   changes is `data`)
    // - trivial round-trip in/out of `SessionData`
    //
    // Per-field merging inside SessionData (e.g. so two concurrent
    // `session.foo = ...` and `session.bar = ...` would both survive)
    // is left for v2; sessions are typically pinned to one node at a
    // time so the cost is small in practice.

    const SESSION_FIELD: &str = "data";

    /// Current unix epoch seconds (wall clock), for read-path expiry checks.
    fn now_unix_secs() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
    }

    fn write_session(doc: &mut automerge::AutoCommit, data: &SessionData) -> Result<(), String> {
        use automerge::{ObjType, ReadDoc, ROOT, transaction::Transactable};
        let json = serde_json::to_vec(data).map_err(|e| e.to_string())?;
        doc.put(ROOT, SESSION_FIELD, automerge::ScalarValue::Bytes(json))
            .map_err(|e| e.to_string())?;
        // Make the change observable to incremental save.
        let _ = ObjType::Map; // silence unused-import on some builds
        let _ = doc.commit();
        let _ = <automerge::AutoCommit as ReadDoc>::get(doc, ROOT, SESSION_FIELD);
        Ok(())
    }

    fn read_session(doc: &automerge::AutoCommit) -> Option<SessionData> {
        use automerge::ReadDoc;
        let (val, _) = doc.get(automerge::ROOT, SESSION_FIELD).ok().flatten()?;
        let bytes = match val {
            automerge::Value::Scalar(s) => match s.into_owned() {
                automerge::ScalarValue::Bytes(b) => b,
                _ => return None,
            },
            _ => return None,
        };
        serde_json::from_slice::<SessionData>(&bytes).ok()
    }

    // ─────────────────────────────────────────────
    // ClusterStore
    // ─────────────────────────────────────────────

    /// Treat `0.0.0.0:<port>` and `[::]:<port>` as equivalent to any local
    /// address on `<port>`; this avoids re-joining ourselves when DNS
    /// resolves the cluster hostname to include this node.
    fn is_self_addr(candidate: &SocketAddr, local_bind: &SocketAddr) -> bool {
        if candidate == local_bind {
            return true;
        }
        if candidate.port() != local_bind.port() {
            return false;
        }
        let bind_is_unspec = local_bind.ip().is_unspecified();
        if !bind_is_unspec {
            return false;
        }
        // Best-effort: if the bind is wildcard, drop candidates that point
        // at any address whose port matches and whose IP belongs to one
        // of our local interfaces. Detecting "is this one of our IPs?"
        // without an extra dep is awkward, so we settle for filtering
        // wildcard self-loopbacks.
        candidate.ip().is_loopback() || candidate.ip().is_unspecified()
    }

    /// A live node in the shared gossip cluster. Owns the cluster state and
    /// joins the `memberlist` once; both the session store ([`Self::session_store`])
    /// and the WebSocket fan-out broker ([`Self::ws_broker`]) are thin handles
    /// derived from it, so a single cluster/port carries both subsystems.
    pub struct ClusterNode {
        shared: Arc<SharedState>,
    }

    impl ClusterNode {
        /// Join the gossip cluster and start its background tasks. Must be
        /// called from within a Tokio runtime.
        ///
        /// `discovery` drives both initial bootstrap and ongoing peer
        /// re-discovery. `ws_registry`, when present, receives inbound WebSocket
        /// fan-out frames and peer-departure (`NodeGone`) eviction.
        pub async fn start(
            listen_addr: &str,
            discovery: crate::session::discovery::Discovery,
            node_name: String,
            ws_registry: Option<Arc<WebSocketRegistry>>,
        ) -> Result<Self, String> {
            let bind: SocketAddr = listen_addr
                .parse()
                .map_err(|e| format!("invalid listenAddr {}: {}", listen_addr, e))?;

            let (tx, mut rx) = mpsc::channel::<ClusterMsg>(OUTBOUND_QUEUE_CAP);

            let shared = Arc::new(SharedState {
                docs: Mutex::new(HashMap::new()),
                tombstones: Mutex::new(HashMap::new()),
                node_name: node_name.clone(),
                tx,
                ws_registry: ws_registry.clone(),
            });

            let node_id = NodeId::try_from(node_name.clone())
                .map_err(|e| format!("invalid nodeName {}: {}", node_name, e))?;

            let mut binds: IndexSet<SocketAddr> = IndexSet::new();
            binds.insert(bind);
            let net_opts = NetTransportOptions::<
                NodeId,
                SocketAddrResolver<TokioRuntime>,
                Tcp<TokioRuntime>,
            >::new(node_id)
            .with_bind_addresses(binds);

            // An event subscriber so we learn when peers leave — a departed node's
            // WebSocket presence-roster entries must be evicted (`NodeGone`).
            let (ev_delegate, ev_subscriber) =
                SubscribleEventDelegate::<NodeId, SocketAddr>::unbounded();

            let delegate = CompositeDelegate::<NodeId, SocketAddr>::default()
                .with_node_delegate(ClusterDelegate {
                    shared: shared.clone(),
                })
                .with_event_delegate(ev_delegate);

            let memberlist =
                TokioTcpMemberlist::with_delegate(delegate, net_opts, Options::lan())
                    .await
                    .map_err(|e| format!("memberlist init failed: {}", e))?;

            let memberlist = Arc::new(memberlist);

            // Drain membership events: on a peer leaving, evict the WebSocket
            // roster entries it owned (conn ids carry its `{node}:` prefix).
            if let Some(reg) = ws_registry.clone() {
                tokio::spawn(async move {
                    while let Ok(ev) = ev_subscriber.recv().await {
                        if ev.kind() == EventKind::Leave {
                            let gone = ev.node_state().id().to_string();
                            reg.drop_node(&gone);
                        }
                    }
                });
            }

            // Initial discovery + join.
            let initial = discovery.discover().await;
            if initial.is_empty() {
                println!(
                    "[session/cluster] discovery ({}) returned no peers — this node is starting solo",
                    discovery.label()
                );
            } else {
                let online: std::collections::HashSet<SocketAddr> = memberlist
                    .online_members()
                    .await
                    .iter()
                    .map(|m| *m.address())
                    .collect();
                let to_join: Vec<MaybeResolvedAddress<_, _>> = initial
                    .iter()
                    .filter(|sa| !is_self_addr(sa, &bind))
                    .filter(|sa| !online.contains(sa))
                    .map(|sa| MaybeResolvedAddress::resolved(*sa))
                    .collect();
                if !to_join.is_empty() {
                    let count = to_join.len();
                    match memberlist.join_many(to_join.into_iter()).await {
                        Ok(joined) => println!(
                            "[session/cluster] joined {} of {} initial peer(s) via {}",
                            joined.len(),
                            count,
                            discovery.label()
                        ),
                        Err((joined, e)) => eprintln!(
                            "[session/cluster] partial initial join — {}/{} reached: {}",
                            joined.len(),
                            count,
                            e
                        ),
                    }
                }
            }

            // Periodic re-discovery: memberlist's join_many future is
            // !Send (internal RefCell-backed cache), so we cannot run it
            // on the main multi-thread runtime. Spawn a dedicated OS
            // thread with its own current-thread runtime instead — the
            // memberlist Arc is Send+Sync, so cross-thread access is
            // fine. Static discovery has no interval and skips this.
            if let Some(period) = discovery.interval() {
                let mlist_disc = memberlist.clone();
                let disc = discovery.clone();
                let bind_filter = bind;
                std::thread::Builder::new()
                    .name("cluster-discovery".into())
                    .spawn(move || {
                        let rt = match tokio::runtime::Builder::new_current_thread()
                            .enable_all()
                            .build()
                        {
                            Ok(rt) => rt,
                            Err(e) => {
                                eprintln!(
                                    "[session/cluster] failed to build discovery runtime: {}",
                                    e
                                );
                                return;
                            }
                        };
                        rt.block_on(async move {
                            let mut tick = tokio::time::interval(period);
                            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
                            // First tick fires immediately; skip — already joined.
                            tick.tick().await;
                            loop {
                                tick.tick().await;
                                let addrs = disc.discover().await;
                                if addrs.is_empty() {
                                    continue;
                                }
                                let online: std::collections::HashSet<SocketAddr> = mlist_disc
                                    .online_members()
                                    .await
                                    .iter()
                                    .map(|m| *m.address())
                                    .collect();
                                let to_join: Vec<MaybeResolvedAddress<_, _>> = addrs
                                    .iter()
                                    .filter(|sa| !is_self_addr(sa, &bind_filter))
                                    .filter(|sa| !online.contains(sa))
                                    .map(|sa| MaybeResolvedAddress::resolved(*sa))
                                    .collect();
                                if to_join.is_empty() {
                                    continue;
                                }
                                let count = to_join.len();
                                match mlist_disc.join_many(to_join.into_iter()).await {
                                    Ok(joined) if !joined.is_empty() => println!(
                                        "[session/cluster] re-discovery joined {} new peer(s) (of {})",
                                        joined.len(),
                                        count
                                    ),
                                    Ok(_) => {}
                                    Err((joined, e)) => eprintln!(
                                        "[session/cluster] re-discovery partial join {}/{}: {}",
                                        joined.len(),
                                        count,
                                        e
                                    ),
                                }
                            }
                        });
                    })
                    .ok();
            }

            // Background sender task: drain outbound queue, reliable-send
            // each message to every currently-online peer.
            let mlist_send = memberlist.clone();
            tokio::spawn(async move {
                while let Some(msg) = rx.recv().await {
                    let bytes = match encode_to_vec(&msg, bincode_std()) {
                        Ok(b) => Bytes::from(b),
                        Err(e) => {
                            eprintln!("[session/cluster] encode failed: {}", e);
                            continue;
                        }
                    };
                    let members = mlist_send.online_members().await;
                    let me = mlist_send.local_id().clone();
                    for member in members.iter() {
                        if member.id() == &me {
                            continue;
                        }
                        if let Err(e) = mlist_send
                            .send_reliable(member.address(), bytes.clone())
                            .await
                        {
                            eprintln!(
                                "[session/cluster] send to {} failed: {}",
                                member.id(),
                                e
                            );
                        }
                    }
                }
            });

            println!(
                "[session/cluster] node '{}' listening on {}",
                shared.node_name, bind
            );

            Ok(Self { shared })
        }

        /// A session store backed by this cluster node.
        pub fn session_store(&self) -> ClusterStore {
            ClusterStore { shared: self.shared.clone() }
        }

        /// A WebSocket fan-out broker backed by this cluster node. Install it on
        /// the registry via `WebSocketRegistry::set_broker`.
        pub fn ws_broker(&self) -> Arc<dyn Broker> {
            Arc::new(ClusterWsBroker { shared: self.shared.clone() })
        }
    }

    /// Thin session-store handle over a [`ClusterNode`].
    pub struct ClusterStore {
        shared: Arc<SharedState>,
    }

    impl ClusterStore {
        /// Encode the local doc's incremental change since the last call
        /// and enqueue a Delta for broadcast.
        fn broadcast_delta(&self, id: &str, doc: &mut automerge::AutoCommit) {
            let change = doc.save_incremental();
            if change.is_empty() {
                return;
            }
            let msg = ClusterMsg::Session(SessionMsg::Delta {
                id: id.to_string(),
                change,
            });
            // try_send: if the queue is full, drop the message — anti-entropy
            // will catch it on the next push/pull.
            if let Err(e) = self.shared.tx.try_send(msg) {
                eprintln!(
                    "[session/cluster] outbound queue full or closed, dropping delta: {}",
                    e
                );
            }
        }

        fn broadcast_tombstone(&self, id: &str) {
            let msg = ClusterMsg::Session(SessionMsg::Tombstone { id: id.to_string() });
            let _ = self.shared.tx.try_send(msg);
        }
    }

    impl SessionStore for ClusterStore {
        // Serializes session data across the cluster (Automerge) — data values
        // only (issue #236).
        fn persists_by_serialization(&self) -> bool {
            true
        }

        fn get(&self, app: &str, id: &str) -> Option<SessionData> {
            let key = composite_key(app, id);
            let docs = self.shared.docs.lock().ok()?;
            let doc = docs.get(&key)?;
            let s = read_session(doc)?;
            // Read-path exactness (G1): an expired session reads as absent the
            // instant it times out, independent of the reaper sweep. We don't
            // remove here (would need a write lock + tombstone broadcast);
            // `take_expired` reclaims it on the next reaper tick.
            let now = now_unix_secs();
            if now.saturating_sub(s.last_accessed_secs) > s.timeout_secs {
                return None;
            }
            Some(s)
        }

        fn set(&self, app: &str, id: &str, data: SessionData) {
            let key = composite_key(app, id);
            let mut docs = match self.shared.docs.lock() {
                Ok(d) => d,
                Err(_) => return,
            };
            let doc = docs
                .entry(key.clone())
                .or_insert_with(automerge::AutoCommit::new);
            if let Err(e) = write_session(doc, &data) {
                eprintln!("[session/cluster] write_session failed: {}", e);
                return;
            }
            self.broadcast_delta(&key, doc);
        }

        fn remove(&self, app: &str, id: &str) {
            let key = composite_key(app, id);
            if let Ok(mut docs) = self.shared.docs.lock() {
                docs.remove(&key);
            }
            if let Ok(mut t) = self.shared.tombstones.lock() {
                t.insert(key.clone(), Instant::now());
            }
            self.broadcast_tombstone(&key);
        }

        fn rotate(&self, app: &str, old_id: &str, new_id: &str) {
            let existing = {
                let docs = match self.shared.docs.lock() {
                    Ok(d) => d,
                    Err(_) => return,
                };
                docs.get(&composite_key(app, old_id)).and_then(read_session)
            };
            if let Some(data) = existing {
                self.set(app, new_id, data);
                self.remove(app, old_id);
            }
        }

        fn take_expired(
            &self,
            now_secs: u64,
        ) -> Vec<(String, String, ValueMap)> {
            // Doc keys are composite `(app, id)`; collect the expired keys.
            let expired_keys: Vec<String> = match self.shared.docs.lock() {
                Ok(docs) => docs
                    .iter()
                    .filter_map(|(key, doc)| {
                        let s = read_session(doc)?;
                        if now_secs.saturating_sub(s.last_accessed_secs) > s.timeout_secs {
                            Some(key.clone())
                        } else {
                            None
                        }
                    })
                    .collect(),
                Err(_) => return Vec::new(),
            };
            let mut out = Vec::with_capacity(expired_keys.len());
            for key in expired_keys {
                let drained = {
                    let docs = match self.shared.docs.lock() {
                        Ok(d) => d,
                        Err(_) => continue,
                    };
                    docs.get(&key)
                        .and_then(read_session)
                        .map(|s| (s.app_name, s.variables))
                };
                if let Some((app_name, vars)) = drained {
                    // Report the bare id (the reaper routes onSessionEnd by
                    // app_name), but evict + tombstone by the composite key.
                    out.push((app_name, id_from_key(&key).to_string(), vars));
                    if let Ok(mut docs) = self.shared.docs.lock() {
                        docs.remove(&key);
                    }
                    if let Ok(mut t) = self.shared.tombstones.lock() {
                        t.insert(key.clone(), Instant::now());
                    }
                    self.broadcast_tombstone(&key);
                }
            }
            out
        }

        fn next_expiry(&self, _now_secs: u64) -> Option<u64> {
            let docs = self.shared.docs.lock().ok()?;
            docs.values()
                .filter_map(read_session)
                .map(|s| s.last_accessed_secs.saturating_add(s.timeout_secs))
                .min()
        }
    }

    /// WebSocket fan-out broker over the shared cluster. Each published
    /// [`BrokerMsg`] is `serde_json`-encoded (the payload is a dynamic
    /// `CfmlValue`) and enqueued as `ClusterMsg::Ws` on the same outbound queue
    /// the session store uses — one gossip cluster, two subsystems.
    pub struct ClusterWsBroker {
        shared: Arc<SharedState>,
    }

    impl std::fmt::Debug for ClusterWsBroker {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("ClusterWsBroker")
                .field("node", &self.shared.node_name)
                .finish()
        }
    }

    impl Broker for ClusterWsBroker {
        fn publish(&self, msg: BrokerMsg) {
            let json = match serde_json::to_string(&msg) {
                Ok(j) => j,
                Err(e) => {
                    eprintln!("[cluster/ws] failed to encode broker message: {}", e);
                    return;
                }
            };
            // try_send: drop on overflow (best-effort fan-out, like the session
            // path) — a dropped WS frame is a missed realtime message, not state
            // corruption, and history/replay covers reconnects.
            let _ = self.shared.tx.try_send(ClusterMsg::Ws(json));
        }
    }
}

#[cfg(feature = "cluster")]
pub use inner::ClusterNode;
