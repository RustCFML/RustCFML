//! Node-to-node application messaging for CFML — the engine side of the
//! `cbjgroups` shim (`org.pixl8.cbjgroups.CbJGroupsClusterWrapper`).
//!
//! A *subscription* is one CFML "channel" object (JGroups calls it a `JChannel`):
//! it joins a named cluster, sends opaque payloads to the other members, and
//! has a listener CFC whose `receive(msg)` / `viewAccepted(view)` run when a
//! payload or a membership change arrives. The listener runs on a fresh VM
//! seeded from the request that created the channel ([`ThreadSeed`]), so it
//! sees that application's scopes — the same way an executor task does.
//!
//! The wire is whatever [`ClusterTransport`] the server installed (the gossip
//! cluster in `rustcfml-cli`). With none, a channel is a working ONE-node
//! cluster, exactly like a JGroups channel that finds no peers: it connects, is
//! its own coordinator, and loops back only when `discardOwnMessages` is off.
//!
//! Ordering: JGroups delivers one sender's messages in order (NAKACK2), and
//! callers depend on it (a cache `set` followed by a `clear`). Each subscription
//! therefore has ONE worker thread that runs its deliveries sequentially.

use crate::{ThreadSeed, ThreadSpawnFn};
use cfml_common::dynamic::{CfmlValue, ValueMap};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex, OnceLock, RwLock};

/// One node as the cluster sees it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Member {
    pub name: String,
    /// Unix epoch millis the node's cluster runtime started. JGroups makes the
    /// OLDEST member the coordinator; this is how we know who that is.
    pub started_ms: u64,
    /// Channel (cluster) names this node currently has connected.
    pub channels: Vec<String>,
}

/// The wire, supplied by the server. All methods are synchronous and cheap:
/// `members()` answers from a cached view, `publish()` enqueues.
pub trait ClusterTransport: Send + Sync {
    fn node_name(&self) -> String;
    fn started_ms(&self) -> u64;
    /// Online members, this node included.
    fn members(&self) -> Vec<Member>;
    /// Send `payload` for `channel` to every OTHER node.
    fn publish(&self, channel: &str, payload: Vec<u8>);
    /// Announce the channels this node has connected (gossiped node metadata).
    fn set_channels(&self, channels: Vec<String>);
}

static TRANSPORT: RwLock<Option<Arc<dyn ClusterTransport>>> = RwLock::new(None);

/// Install the server's transport. Called once at server start when a cluster
/// is configured.
pub fn install_transport(t: Arc<dyn ClusterTransport>) {
    if let Ok(mut g) = TRANSPORT.write() {
        *g = Some(t);
    }
}

pub fn transport() -> Option<Arc<dyn ClusterTransport>> {
    TRANSPORT.read().ok().and_then(|g| g.clone())
}

/// Start time of this process's single-node fallback, so a lone node still has
/// a stable `started_ms`.
fn local_started_ms() -> u64 {
    static T: OnceLock<u64> = OnceLock::new();
    *T.get_or_init(now_ms)
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn local_node_name() -> String {
    if let Some(t) = transport() {
        return t.node_name();
    }
    static N: OnceLock<String> = OnceLock::new();
    N.get_or_init(|| {
        std::env::var("HOSTNAME")
            .ok()
            .filter(|h| !h.is_empty())
            .unwrap_or_else(|| "localhost".to_string())
    })
    .clone()
}

/// Message/byte counters, in the keys JGroups' `dumpStats()` channel section
/// reports (`received_msgs`, `received_bytes`, `sent_msgs`, `sent_bytes`).
#[derive(Default)]
pub struct Stats {
    pub sent_msgs: AtomicU64,
    pub sent_bytes: AtomicU64,
    pub received_msgs: AtomicU64,
    pub received_bytes: AtomicU64,
}

enum Delivery {
    Message(Vec<u8>),
    View,
}

struct Subscription {
    channel: Option<String>,
    discard_own: bool,
    closed: bool,
    stats: Arc<Stats>,
    tx: mpsc::Sender<Delivery>,
}

static SUBS: OnceLock<Mutex<HashMap<u64, Subscription>>> = OnceLock::new();
static NEXT_ID: AtomicU64 = AtomicU64::new(1);

fn subs() -> &'static Mutex<HashMap<u64, Subscription>> {
    SUBS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Create a (not yet connected) channel whose deliveries run `listener`'s
/// `receive` / `viewAccepted` on VMs spawned from `seed` via `spawn`.
pub fn subscribe(
    listener: CfmlValue,
    seed: ThreadSeed,
    spawn: ThreadSpawnFn,
    discard_own: bool,
) -> u64 {
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let (tx, rx) = mpsc::channel::<Delivery>();
    let stats = Arc::new(Stats::default());
    std::thread::Builder::new()
        .name(format!("cluster-channel-{}", id))
        .spawn(move || deliver_loop(id, listener, seed, spawn, rx))
        .ok();
    if let Ok(mut m) = subs().lock() {
        m.insert(
            id,
            Subscription { channel: None, discard_own, closed: false, stats, tx },
        );
    }
    id
}

/// Sequential delivery worker for one subscription. Each delivery runs on its
/// own seeded VM (a `cfthread`-style spawn) and is awaited before the next.
fn deliver_loop(
    id: u64,
    listener: CfmlValue,
    seed: ThreadSeed,
    spawn: ThreadSpawnFn,
    rx: mpsc::Receiver<Delivery>,
) {
    while let Ok(d) = rx.recv() {
        let (method, arg) = match d {
            Delivery::Message(bytes) => ("receive", message_shim(bytes)),
            Delivery::View => {
                let channel = channel_of(id).unwrap_or_default();
                ("viewAccepted", view_shim(&channel))
            }
        };
        let mut body = ValueMap::default();
        body.insert("__async_invoke_target".to_string(), listener.clone());
        body.insert("__async_invoke_method".to_string(), CfmlValue::string(method.to_string()));
        body.insert("__async_invoke_args".to_string(), CfmlValue::array(vec![arg]));
        let mut s = seed.clone();
        s.closure = CfmlValue::strukt(body);
        // A delivery is not part of the request that created the channel: it
        // gets its own request scope and no session.
        s.request_scope = cfml_common::dynamic::CfmlStruct::new(ValueMap::default());
        s.session_scope = None;
        s.session_id = None;
        s.thread_name = Some(format!("cluster-channel-{}", id));
        let mut handle = spawn(s);
        // Wait for completion so the next delivery cannot overtake this one.
        let _ = handle.rx.recv();
        if let Some(j) = handle.join.take() {
            let _ = j.join();
        }
        let closed = subs()
            .lock()
            .map(|m| m.get(&id).map(|s| s.closed).unwrap_or(true))
            .unwrap_or(true);
        if closed {
            break;
        }
    }
}

fn channel_of(id: u64) -> Option<String> {
    subs().lock().ok()?.get(&id)?.channel.clone()
}

/// Channels this node has connected, for the gossiped metadata.
fn connected_channels(m: &HashMap<u64, Subscription>) -> Vec<String> {
    let mut v: Vec<String> = m
        .values()
        .filter(|s| !s.closed)
        .filter_map(|s| s.channel.clone())
        .collect();
    v.sort();
    v.dedup();
    v
}

fn announce_channels() {
    let Some(t) = transport() else { return };
    let chans = subs().lock().map(|m| connected_channels(&m)).unwrap_or_default();
    t.set_channels(chans);
}

/// Connect the subscription to `channel`. Idempotent.
pub fn connect(id: u64, channel: &str) {
    if let Ok(mut m) = subs().lock() {
        if let Some(s) = m.get_mut(&id) {
            s.channel = Some(channel.to_string());
            s.closed = false;
        }
    }
    announce_channels();
    // The first view, as JGroups delivers on connect.
    notify_view_changed_for(channel);
}

pub fn is_connected(id: u64) -> bool {
    subs()
        .lock()
        .map(|m| m.get(&id).is_some_and(|s| !s.closed && s.channel.is_some()))
        .unwrap_or(false)
}

/// Disconnect and stop the subscription's worker.
pub fn close(id: u64) {
    let channel = if let Ok(mut m) = subs().lock() {
        m.remove(&id).and_then(|s| s.channel)
    } else {
        None
    };
    announce_channels();
    if let Some(c) = channel {
        notify_view_changed_for(&c);
    }
}

/// Send `payload` to the channel's other members (and to this subscription
/// too when `discardOwnMessages` is off).
pub fn send(id: u64, payload: Vec<u8>) -> Result<(), String> {
    let (channel, discard_own, stats) = {
        let m = subs().lock().map_err(|_| "cluster registry poisoned".to_string())?;
        let s = m.get(&id).ok_or_else(|| "channel is closed".to_string())?;
        let c = s
            .channel
            .clone()
            .ok_or_else(|| "channel is not connected".to_string())?;
        (c, s.discard_own, s.stats.clone())
    };
    stats.sent_msgs.fetch_add(1, Ordering::Relaxed);
    stats.sent_bytes.fetch_add(payload.len() as u64, Ordering::Relaxed);
    if let Some(t) = transport() {
        t.publish(&channel, payload.clone());
    }
    // Other channels on THIS node with the same name are separate members, as
    // two JChannels in one JVM are; this one hears itself only if asked to.
    deliver_local(&channel, payload, Some(id), discard_own);
    Ok(())
}

fn deliver_local(channel: &str, payload: Vec<u8>, sender: Option<u64>, sender_discards: bool) {
    let Ok(m) = subs().lock() else { return };
    for (sid, s) in m.iter() {
        if s.closed || s.channel.as_deref() != Some(channel) {
            continue;
        }
        if Some(*sid) == sender && sender_discards {
            continue;
        }
        s.stats.received_msgs.fetch_add(1, Ordering::Relaxed);
        s.stats.received_bytes.fetch_add(payload.len() as u64, Ordering::Relaxed);
        let _ = s.tx.send(Delivery::Message(payload.clone()));
    }
}

/// A payload for `channel` arrived from another node.
pub fn deliver_remote(channel: &str, payload: Vec<u8>) {
    deliver_local(channel, payload, None, false);
}

/// Membership changed somewhere in the cluster: every connected channel gets a
/// `viewAccepted`. (JGroups delivers a new view to each member on any change.)
pub fn notify_view_changed() {
    let Ok(m) = subs().lock() else { return };
    for s in m.values() {
        if !s.closed && s.channel.is_some() {
            let _ = s.tx.send(Delivery::View);
        }
    }
}

fn notify_view_changed_for(channel: &str) {
    let Ok(m) = subs().lock() else { return };
    for s in m.values() {
        if !s.closed && s.channel.as_deref() == Some(channel) {
            let _ = s.tx.send(Delivery::View);
        }
    }
}

/// The members of `channel`, coordinator (oldest) first — JGroups' view order.
pub fn view(channel: &str) -> Vec<Member> {
    let mut members: Vec<Member> = match transport() {
        Some(t) => t
            .members()
            .into_iter()
            .filter(|m| m.channels.iter().any(|c| c == channel))
            .collect(),
        None => Vec::new(),
    };
    // This node belongs to the view as soon as one of its channels is
    // connected, even before its own gossip round-trips.
    let me = local_node_name();
    let connected_here = subs()
        .lock()
        .map(|m| {
            m.values()
                .any(|s| !s.closed && s.channel.as_deref() == Some(channel))
        })
        .unwrap_or(false);
    if connected_here && !members.iter().any(|m| m.name == me) {
        members.push(Member {
            name: me,
            started_ms: transport().map(|t| t.started_ms()).unwrap_or_else(local_started_ms),
            channels: vec![channel.to_string()],
        });
    }
    members.sort_by(|a, b| a.started_ms.cmp(&b.started_ms).then_with(|| a.name.cmp(&b.name)));
    members
}

/// Whether this node is the channel's coordinator: the oldest member, or the
/// only one.
pub fn is_coordinator(id: u64) -> bool {
    let Some(channel) = channel_of(id) else { return true };
    let v = view(&channel);
    v.first().map(|m| m.name == local_node_name()).unwrap_or(true)
}

/// JGroups-shaped statistics for a channel.
pub fn stats(id: u64) -> ValueMap {
    let (channel, stats, connected) = {
        let m = subs().lock().ok();
        match m.as_ref().and_then(|m| m.get(&id)) {
            Some(s) => (s.channel.clone(), Some(s.stats.clone()), !s.closed && s.channel.is_some()),
            None => (None, None, false),
        }
    };
    let mut out = ValueMap::default();
    if let Some(st) = stats {
        out.insert("sent_msgs".to_string(), CfmlValue::Int(st.sent_msgs.load(Ordering::Relaxed) as i64));
        out.insert("sent_bytes".to_string(), CfmlValue::Int(st.sent_bytes.load(Ordering::Relaxed) as i64));
        out.insert("received_msgs".to_string(), CfmlValue::Int(st.received_msgs.load(Ordering::Relaxed) as i64));
        out.insert("received_bytes".to_string(), CfmlValue::Int(st.received_bytes.load(Ordering::Relaxed) as i64));
    }
    let members: Vec<CfmlValue> = channel
        .as_deref()
        .map(view)
        .unwrap_or_default()
        .into_iter()
        .map(|m| CfmlValue::string(m.name))
        .collect();
    out.insert("members".to_string(), CfmlValue::array(members));
    out.insert("self".to_string(), CfmlValue::string(local_node_name()));
    out.insert("is_coordinator".to_string(), CfmlValue::Bool(is_coordinator(id)));
    out.insert(
        "connection".to_string(),
        CfmlValue::string(if connected { "CONNECTED" } else { "CLOSED" }.to_string()),
    );
    out
}

/// `org.jgroups.Message`, as the listener's `receive(msg)` sees it.
pub const MESSAGE_CLASS: &str = "org.jgroups.Message";
/// `org.jgroups.View`, as `viewAccepted(view)` sees it.
pub const VIEW_CLASS: &str = "org.jgroups.View";

fn shim(class: &str) -> ValueMap {
    let mut m = ValueMap::default();
    m.insert("__java_shim".to_string(), CfmlValue::Bool(true));
    m.insert("__java_class".to_string(), CfmlValue::string(class.to_string()));
    m
}

fn message_shim(bytes: Vec<u8>) -> CfmlValue {
    let mut m = shim(MESSAGE_CLASS);
    m.insert("__buffer".to_string(), CfmlValue::Binary(bytes));
    CfmlValue::strukt(m)
}

fn view_shim(channel: &str) -> CfmlValue {
    let mut m = shim(VIEW_CLASS);
    let members: Vec<CfmlValue> = view(channel)
        .into_iter()
        .map(|mb| CfmlValue::string(mb.name))
        .collect();
    m.insert("__members".to_string(), CfmlValue::array(members));
    CfmlValue::strukt(m)
}
