//! The application-lifetime (layer 2) half of the existence cache — the part the
//! CFML suite cannot reach, because the CLI has no `ServerState` and therefore no
//! layer 2 at all.
//!
//! Pins four properties, each of which is a decision rather than an accident:
//!
//! 1. **Production caches a POSITIVE across requests** — that is the whole point.
//! 2. **Production caches a NEGATIVE across requests**, which is what the 2026-08
//!    request-scoped attempt could not do and why it measured zero: on a warm
//!    Preside homepage 14 of every 16 probes repeat *across* a request boundary.
//! 3. **Dev caches neither across requests.** An edit must be visible next
//!    request, the same contract `canonicalize_cache` / `component_path_cache`
//!    take.
//! 4. **The engine's own writes invalidate**, in either direction: a file this
//!    process deletes stops existing, and one it creates starts existing, even
//!    when an earlier request cached the opposite.
//!
//! Property 4 is the one with teeth. A cached *positive* is invalidated per path;
//! a cached *negative* is retired by a process-global generation bump, because
//! creation has no single choke point (the `Vfs` trait is read-only and several
//! creators — `cfdump output=`, `cfhttp file=`, the upload BIFs, `cfexecute` — are
//! VM-intercepted and never pass the builtin dispatcher).

use cfml_codegen::{compiler::CfmlCompiler, BytecodeProgram};
use cfml_common::dynamic::{CfmlValue, ValueMap};
use cfml_compiler::{parser::Parser, tag_parser};
use cfml_stdlib::builtins::{get_builtin_functions, get_builtins};
use cfml_vm::{CfmlVirtualMachine, ServerState};

fn compile_page(source: &str) -> BytecodeProgram {
    let processed = if tag_parser::has_cfml_tags(source) {
        tag_parser::tags_to_script(source)
    } else {
        source.to_string()
    };
    let ast = Parser::new(processed).parse().expect("parse");
    CfmlCompiler::new().compile(ast)
}

/// One "request": a fresh VM sharing the given `ServerState`, exactly as the
/// serve loop builds one per request.
fn run_request(ss: &ServerState, source: &str) -> String {
    let mut vm = CfmlVirtualMachine::new(compile_page(source));
    // A real on-disk page path: these tests probe the REAL filesystem (that is
    // the point), so unlike the in-memory-VFS lifecycle tests they must give the
    // VM a concrete template to resolve relative paths against. Without it,
    // path resolution has no base to work from.
    let page = {
        let mut p = std::env::temp_dir();
        p.push("rustcfml_xreq_exists_page.cfm");
        std::fs::write(&p, b"").ok();
        p.display().to_string()
    };
    vm.source_file = Some(page.clone());
    vm.base_template_path = Some(page);
    for (name, value) in get_builtins() {
        vm.globals.insert(name, value);
    }
    for (name, func) in get_builtin_functions() {
        vm.builtins.insert(name, func);
    }
    for scope in ["url", "cgi", "form"] {
        vm.globals
            .entry(scope.to_string())
            .or_insert_with(|| CfmlValue::strukt(ValueMap::default()));
    }
    vm.server_state = Some(ss.clone());
    vm.execute().expect("execute");
    vm.get_output().trim().to_string()
}

/// A unique temp path that does not exist yet.
fn temp_path(tag: &str) -> std::path::PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!(
        "rustcfml_xreq_exists_{}_{}_{}",
        tag,
        std::process::id(),
        // A counter rather than a clock: deterministic within the run.
        COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    p
}
static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn probe(ss: &ServerState, path: &std::path::Path) -> String {
    run_request(
        ss,
        &format!(
            "<cfoutput>#fileExists('{}')#</cfoutput>",
            path.display().to_string().replace('\\', "\\\\")
        ),
    )
}

#[test]
fn production_caches_a_positive_across_requests() {
    let ss = ServerState::with_production(true);
    let f = temp_path("pos");
    std::fs::write(&f, b"x").unwrap();

    assert_eq!(probe(&ss, &f), "true", "first request sees the file");

    // Remove it behind the engine's back. Production deliberately holds the
    // cached positive: this is the immutable-tree contract, and it is also proof
    // that layer 2 is genuinely serving rather than silently re-stat'ing.
    std::fs::remove_file(&f).unwrap();
    assert_eq!(
        probe(&ss, &f),
        "true",
        "production holds the cross-request positive through an out-of-process delete"
    );
}

#[test]
fn production_caches_a_negative_across_requests() {
    let ss = ServerState::with_production(true);
    let f = temp_path("neg");

    assert_eq!(probe(&ss, &f), "false", "first request: absent");

    // Created behind the engine's back — production holds the cached negative.
    // This is the trade-off `runtime.existenceCacheScope = "request"` exists to
    // opt out of, and it is what makes the warm 14-of-16 cross-request repeats
    // recoverable in the first place.
    std::fs::write(&f, b"x").unwrap();
    assert_eq!(
        probe(&ss, &f),
        "false",
        "production holds the cross-request negative through an out-of-process create"
    );
    std::fs::remove_file(&f).ok();
}

#[test]
fn dev_caches_nothing_across_requests() {
    let ss = ServerState::with_production(false);
    let f = temp_path("dev");

    assert_eq!(probe(&ss, &f), "false", "dev: absent");
    std::fs::write(&f, b"x").unwrap();
    assert_eq!(
        probe(&ss, &f),
        "true",
        "dev must see an out-of-process CREATE on the next request"
    );
    std::fs::remove_file(&f).unwrap();
    assert_eq!(
        probe(&ss, &f),
        "false",
        "dev must see an out-of-process DELETE on the next request"
    );
}

#[test]
fn request_scope_setting_disables_layer_two() {
    let mut cfg = cfml_config::RustCfmlConfig::default();
    cfg.runtime.existence_cache_scope = "request".into();
    let ss = ServerState::with_config(true, std::sync::Arc::new(cfg));
    let f = temp_path("scoped");

    assert_eq!(probe(&ss, &f), "false", "absent");
    std::fs::write(&f, b"x").unwrap();
    // Production, but scoped to the request: the negative must NOT survive.
    assert_eq!(
        probe(&ss, &f),
        "true",
        "existenceCacheScope=request re-probes on the next request even in production"
    );
    std::fs::remove_file(&f).ok();
}

#[test]
fn own_delete_invalidates_a_cached_positive_in_production() {
    let ss = ServerState::with_production(true);
    let f = temp_path("owndel");
    std::fs::write(&f, b"x").unwrap();
    let p = f.display().to_string().replace('\\', "\\\\");

    assert_eq!(probe(&ss, &f), "true", "cached positive established");

    // The engine's OWN delete, in a later request. Unlike the out-of-process
    // delete above this must be seen immediately, and by the same request.
    let out = run_request(
        &ss,
        &format!("<cfset fileDelete('{p}')><cfoutput>#fileExists('{p}')#</cfoutput>"),
    );
    assert_eq!(out, "false", "our own delete invalidates the cached positive");
    assert_eq!(
        probe(&ss, &f),
        "false",
        "and the invalidation outlives the request that did it"
    );
}

#[test]
fn own_create_invalidates_a_cached_negative_in_production() {
    let ss = ServerState::with_production(true);
    let f = temp_path("owncreate");
    let p = f.display().to_string().replace('\\', "\\\\");

    assert_eq!(probe(&ss, &f), "false", "cached negative established");

    let out = run_request(
        &ss,
        &format!("<cfset fileWrite('{p}', 'v')><cfoutput>#fileExists('{p}')#</cfoutput>"),
    );
    assert_eq!(out, "true", "our own write retires the cached negative");
    assert_eq!(
        probe(&ss, &f),
        "true",
        "and the retirement outlives the request that did it"
    );
    std::fs::remove_file(&f).ok();
}

#[test]
fn a_write_does_not_discard_an_unrelated_cached_positive() {
    // The "a write only invalidates caches relative to that file" rule. Before
    // this, every filesystem-mutating BIF cleared the whole map — 861 flushes per
    // Preside request, 835 of them from `fileClose`, which cannot remove anything
    // (commit c9fa07f). Positives for untouched paths must survive.
    let ss = ServerState::with_production(true);
    let keep = temp_path("keep");
    let touch = temp_path("touch");
    std::fs::write(&keep, b"keep").unwrap();

    assert_eq!(probe(&ss, &keep), "true", "positive for `keep` cached");

    // Delete `keep` out of process so ONLY a cached positive can answer "true",
    // then have the engine write an unrelated path in a later request.
    std::fs::remove_file(&keep).unwrap();
    let tp = touch.display().to_string().replace('\\', "\\\\");
    run_request(&ss, &format!("<cfset fileWrite('{tp}', 'v')>"));

    assert_eq!(
        probe(&ss, &keep),
        "true",
        "writing an unrelated path must not discard `keep`'s cached positive"
    );
    std::fs::remove_file(&touch).ok();
}
