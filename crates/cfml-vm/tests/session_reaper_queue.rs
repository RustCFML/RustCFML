//! Unit coverage for the reaper's pending-`onSessionEnd` queue on
//! `ServerState`. The reaper drains expired session *data* off the request
//! path and queues a hook delivery per session keyed by the owning
//! application; the next request for that application drains its queue. These
//! tests exercise that queue directly (queueing, per-app keying, bounded
//! capacity, drain semantics) without standing up a full server.

use cfml_common::dynamic::{CfmlValue, ValueMap};
use cfml_vm::ServerState;

fn vars(tag: &str) -> ValueMap {
    let mut m = ValueMap::default();
    m.insert("marker".to_string(), CfmlValue::string(tag.to_string()));
    m
}

fn marker(v: &ValueMap) -> String {
    v.get("marker").map(|x| x.as_string()).unwrap_or_default()
}

#[test]
fn queue_and_drain_round_trip_in_order() {
    let state = ServerState::new();
    state.queue_session_end("shop", vars("s1"), 100);
    state.queue_session_end("shop", vars("s2"), 100);

    let drained = state.drain_session_ends("shop");
    let got: Vec<String> = drained.iter().map(marker).collect();
    assert_eq!(got, vec!["s1", "s2"], "delivered in arrival order");

    // Draining empties the queue.
    assert!(state.drain_session_ends("shop").is_empty());
}

#[test]
fn queues_are_keyed_per_application() {
    let state = ServerState::new();
    state.queue_session_end("shop", vars("s1"), 100);
    state.queue_session_end("blog", vars("b1"), 100);

    // A request for one app drains only its own app's hooks.
    let shop = state.drain_session_ends("shop");
    assert_eq!(shop.len(), 1);
    assert_eq!(marker(&shop[0]), "s1");

    // The other app's hook is untouched.
    let blog = state.drain_session_ends("blog");
    assert_eq!(blog.len(), 1);
    assert_eq!(marker(&blog[0]), "b1");
}

#[test]
fn bounded_capacity_drops_oldest_beyond_batch_max() {
    let state = ServerState::new();
    // Cap at 2: pushing a third evicts the oldest and reports a drop.
    assert!(!state.queue_session_end("idle", vars("1"), 2));
    assert!(!state.queue_session_end("idle", vars("2"), 2));
    let dropped = state.queue_session_end("idle", vars("3"), 2);
    assert!(dropped, "third push past the cap must report a drop");

    let got: Vec<String> = state
        .drain_session_ends("idle")
        .iter()
        .map(marker)
        .collect();
    assert_eq!(got, vec!["2", "3"], "oldest ('1') dropped, newest retained");
}

#[test]
fn drain_unknown_app_is_empty() {
    let state = ServerState::new();
    assert!(state.drain_session_ends("never-seen").is_empty());
}
