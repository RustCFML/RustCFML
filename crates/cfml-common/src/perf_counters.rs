//! Process-global diagnostic counters for sizing performance levers before
//! designing them (the counter-first rule: never size a lever from a profile
//! share). Increments are single relaxed atomic adds — cheap enough to leave
//! compiled in unconditionally. Nothing is ever printed unless the embedder
//! asks for a [`report`] (the CLI does so at graceful shutdown when
//! `RUSTCFML_COUNTERS=1`).

use std::sync::atomic::{AtomicU64, Ordering::Relaxed};

/// `CfmlStruct::new` calls (cycle-GC-tracked structs: scopes, literals, CFCs).
pub static STRUCT_NEW: AtomicU64 = AtomicU64::new(0);
/// `CfmlStruct::new_untracked` calls (frame-confined scopes, Lever C opt 1).
pub static STRUCT_NEW_UNTRACKED: AtomicU64 = AtomicU64::new(0);

/// Flyweight CFC instances produced (`make_instance_value`) — the single
/// instantiation choke point. Part 1 Step 0.5: the shape-based instance track
/// (roadmap 3B) is sold on FOOTPRINT, so it has to be gated on how many
/// instances actually exist and how wide they are, not on a profile share.
pub static INSTANCES_CREATED: AtomicU64 = AtomicU64::new(0);
/// Public (`this`) data members across every instance produced — divide by
/// [`INSTANCES_CREATED`] for the mean declared width a shape would replace.
pub static INSTANCE_THIS_KEYS: AtomicU64 = AtomicU64::new(0);
/// Private (`variables`) data members across every instance produced.
pub static INSTANCE_VARS_KEYS: AtomicU64 = AtomicU64::new(0);

/// Eager-vs-lazy `arguments` scope per frame (Lever A, v0.512-517, was -7.7%).
/// `function_needs_arguments_scope` forces the EAGER path on any function whose
/// bytecode contains a `LoadLocal("arguments")` — and the default-parameter
/// preamble emits exactly that op (`compiler.rs` ~4008), so a single defaulted
/// param opts the whole function out of Lever A for every call, whether or not
/// the default ever fires. These size that.
pub static FRAMES_ARGS_EAGER: AtomicU64 = AtomicU64::new(0);
/// Frames that took the lazy skip path (Lever A working as intended).
pub static FRAMES_ARGS_LAZY: AtomicU64 = AtomicU64::new(0);
/// Eager frames whose callee declares at least one defaulted parameter — the
/// upper bound on "forced eager by a default".
pub static FRAMES_ARGS_EAGER_WITH_DEFAULTS: AtomicU64 = AtomicU64::new(0);

/// Parameter binding shape per call frame. The Part 1 reconciliation priced
/// frames from a ZERO-ARG callee (257 ns), but frame cost scales hard with
/// arity — measured +78 ns per positional argument, and a Preside-shaped
/// `required string` / defaulted / typed 4-param callee costs 1,637 ns, 6.4x
/// the zero-arg frame. These say which of those two numbers the real workload
/// looks like.
pub static BIND_FRAMES: AtomicU64 = AtomicU64::new(0);
/// Declared parameters summed over every bound frame.
pub static BIND_PARAMS_DECLARED: AtomicU64 = AtomicU64::new(0);
/// Arguments actually supplied, summed over every bound frame.
pub static BIND_ARGS_SUPPLIED: AtomicU64 = AtomicU64::new(0);
/// Declared-type validations performed, summed over every bound frame.
pub static BIND_TYPECHECKS: AtomicU64 = AtomicU64::new(0);

/// Return-time parent-scope DIFF ("diff-out") volume — the second half of the
/// copy-in/diff-out scope model that Part 3A proposes replacing with a frame
/// arena. Copy-in was already sized at ~0.88 ms/render (97% of seeded keys are
/// the four structural names, read 33:1 never). Diff-out has never been sized,
/// and it is the half that decides whether 3A is a ~2 ms bundle deliverable in
/// stages or a multi-week arena migration. Counter-first, per Part 6's rule:
/// these run before any ablation so the A/B has a predicted magnitude to hit.
pub static WB_FRAMES: AtomicU64 = AtomicU64::new(0);
/// Frames that reached the diff and were skipped by the v0.600.0 futility guard
/// (locals untouched since entry) — already-harvested win, not available again.
pub static WB_SKIPPED_FUTILE: AtomicU64 = AtomicU64::new(0);
/// Locals entries walked by the diff, summed over every frame that ran one.
pub static WB_KEYS_SCANNED: AtomicU64 = AtomicU64::new(0);
/// Entries that survived the filters and cost a `values_equal_shallow` compare.
pub static WB_KEYS_COMPARED: AtomicU64 = AtomicU64::new(0);
/// Entries that actually propagated to the caller — the diff's real output.
pub static WB_KEYS_WRITTEN: AtomicU64 = AtomicU64::new(0);
/// Wall-clock nanoseconds spent inside the diff, summed over both exit paths.
/// Timing is affordable HERE (unlike per-op) because the diff runs only ~1,462
/// times per warm render: at the ~17 ns instrument floor the two `Instant::now()`
/// calls contribute ~0.025 ms, several times below the expected signal. The
/// ablation route was tried first and is not viable — removing the diff stops
/// Preside booting (`COLDBOX_APP_MAPPING` is one of the 26 values it propagates).
pub static WB_NANOS: AtomicU64 = AtomicU64::new(0);
/// Copy-in ("parent-scope seed") frames and nanoseconds — 3A's FIRST half, the
/// one already sized at ~0.88 ms/render by counters + the call-phases clock back
/// in 2026-08-14. Re-measured here with the same instrument as WB_NANOS so both
/// halves of 3A carry a same-version, same-method number, which is what 3A's own
/// gate demands before the migration is started.
pub static SEED_FRAMES: AtomicU64 = AtomicU64::new(0);
pub static SEED_NANOS: AtomicU64 = AtomicU64::new(0);

/// Bare-name scope-chain resolution depth (`lookup_name_in_scopes`). The Part 1
/// verdict sized an unslotted by-name access at ~33 ns over a slot read when it
/// hits `locals` on the first probe, and ~53 ns when it has to reach
/// `__variables`. Which of those applies to the 30,289 by-name accesses in a
/// warm render decides whether any slotting work is worth doing — a name that
/// resolves past `locals` can never become a slot, because there is no local to
/// slot. Counter-first: these split the population before anything is designed.
pub static SCOPE_LOOKUP_TOTAL: AtomicU64 = AtomicU64::new(0);
/// Resolved by the first `locals.get(name)` probe — the slottable population.
pub static SCOPE_HIT_LOCALS: AtomicU64 = AtomicU64::new(0);
/// Resolved in the `arguments` struct (extra/`argumentCollection` args only —
/// declared params are already copied into `locals`).
pub static SCOPE_HIT_ARGUMENTS: AtomicU64 = AtomicU64::new(0);
/// Resolved in a web request scope (url/form/cgi/cookie) via globals.
pub static SCOPE_HIT_WEBSCOPE: AtomicU64 = AtomicU64::new(0);
/// Resolved in `__variables` — the component/page scope. NOT slottable.
pub static SCOPE_HIT_VARIABLES: AtomicU64 = AtomicU64::new(0);
/// Resolved in `globals` (page scope / builtins).
pub static SCOPE_HIT_GLOBALS: AtomicU64 = AtomicU64::new(0);
/// Walked the whole chain and found nothing.
pub static SCOPE_MISS: AtomicU64 = AtomicU64::new(0);

/// Entries into `resolve_component_template`.
pub static RESOLVE_CALLS: AtomicU64 = AtomicU64::new(0);
/// Resolutions answered by the two-layer path cache (request or production).
pub static RESOLVE_CACHE_HITS: AtomicU64 = AtomicU64::new(0);
/// Resolutions that fell through to the candidate-path probe walk.
pub static RESOLVE_PROBE_WALKS: AtomicU64 = AtomicU64::new(0);

/// Case-insensitive builtin resolutions served by the lowercased index.
pub static BUILTIN_LOOKUP_CI: AtomicU64 = AtomicU64::new(0);
/// Of those, the ones whose call-site spelling differed from the registry key
/// (`Len()` vs the registry's `len`) — i.e. how often the case-insensitive
/// fallback is taken. Before v0.596 each of these was an unmemoized linear
/// scan over all ~730 builtins plus a fresh `Arc<CfmlFunction>`; sizing that on
/// a real app is what this pair exists to answer.
pub static BUILTIN_LOOKUP_CI_MISCASED: AtomicU64 = AtomicU64::new(0);
/// Resolutions that could NOT use the index because an embedder inserted into
/// the public `builtins` field without calling `refresh_builtin_index()` —
/// these still pay the O(n) scan and should be zero in a healthy process.
pub static BUILTIN_LOOKUP_CI_SCAN: AtomicU64 = AtomicU64::new(0);

/// (`probe-sites` builds only — these sit on the hottest path in the engine,
/// so the shipped binary must not pay even a relaxed atomic for them.)
/// Keyed lookups whose probe carried a PRE-COMPUTED hash — a `Key` or a
/// bytecode `Name` (interned at compile time). These do no hashing at all.
pub static PROBE_PRECOMPUTED: AtomicU64 = AtomicU64::new(0);
/// Keyed lookups that had to fold-hash a `&str`/`String` probe at the call
/// site. The ratio of these two is what says how much of the interned-key
/// lever has actually been captured.
pub static PROBE_HASHED: AtomicU64 = AtomicU64::new(0);

// ---------------------------------------------------------------------------
// Component-metadata derivation. v0.596.0 added a REQUEST-scoped memo to both
// the path-string form of `getComponentMetaData("a.b.C")` and the inheritance
// walk behind it. These counters size what is LEFT: the memo is dropped with
// the request VM, so every request re-pays the cold misses, and even a hit
// pays a `deep_copy` (metadata structs are reference-typed and ColdBox mutates
// what it is handed). Nanos are only taken when `RUSTCFML_COUNTERS=1`.
// ---------------------------------------------------------------------------
/// Path-string `getComponentMetaData("a.b.C")` calls that were memoizable.
pub static META_PATH_CALLS: AtomicU64 = AtomicU64::new(0);
/// Of those, answered from the request memo (still pays the deep copy).
pub static META_PATH_HITS: AtomicU64 = AtomicU64::new(0);
/// Nanoseconds spent deriving path metadata on a memo MISS.
pub static META_PATH_MISS_NANOS: AtomicU64 = AtomicU64::new(0);
/// Nanoseconds spent deep-copying a memo HIT out to the caller.
pub static META_PATH_HIT_NANOS: AtomicU64 = AtomicU64::new(0);
/// Sub-split of `META_PATH_MISS_NANOS`, to find which stage of the miss
/// actually costs. The suspicion under test is that stages 2+3 are pure waste:
/// they materialise the whole merged component and then only four keys are
/// read off it before `build_inheritance_metadata` re-derives everything from
/// the raw templates. Splitting the phase BEFORE designing its fix is the rule
/// here — the last phase diagnosed by inspection alone was wrong.
pub static META_MISS_RESOLVE_NANOS: AtomicU64 = AtomicU64::new(0);
pub static META_MISS_INHERIT_NANOS: AtomicU64 = AtomicU64::new(0);
pub static META_MISS_SNAPSHOT_NANOS: AtomicU64 = AtomicU64::new(0);
pub static META_MISS_BUILD_NANOS: AtomicU64 = AtomicU64::new(0);

/// `resolve_component_template` calls whose (class, source dir, base template,
/// mappings) key had ALREADY been resolved earlier in the same request — i.e.
/// the pseudo-constructor was re-executed and the parent chain re-walked to
/// rebuild a template this request had already built once. This is the number
/// that decides whether an executed-template cache is worth building: the
/// existing path cache only memoises the FILENAME, never the execution.
pub static RESOLVE_TEMPLATE_REPEAT: AtomicU64 = AtomicU64::new(0);
/// Distinct keys behind those calls (the floor an ideal cache could reach).
pub static RESOLVE_TEMPLATE_DISTINCT: AtomicU64 = AtomicU64::new(0);
/// Template resolutions served whole from the metadata executed-template
/// cache — each one is a pseudo-constructor NOT run and a parent chain NOT
/// walked.
pub static META_TEMPLATE_CACHE_HITS: AtomicU64 = AtomicU64::new(0);

/// Entries into the inheritance-metadata builder (per chain LEVEL).
pub static META_INHERIT_CALLS: AtomicU64 = AtomicU64::new(0);
/// Of those, answered from the request memo.
pub static META_INHERIT_HITS: AtomicU64 = AtomicU64::new(0);

/// Adds its own lifetime, in nanoseconds, to `target` when dropped — so a
/// block with several `return` paths is timed correctly without threading a
/// stopwatch through each one. Construct it directly and bind it to a `let`;
/// never build one behind `Option::then_some`, which evaluates (and instantly
/// drops) its argument eagerly.
/// Reads no clock at all unless `RUSTCFML_COUNTERS=1` — a timed phase should
/// not make the shipped engine pay for an instrument nobody is reading.
pub struct ScopedNanos<'a> {
    target: &'a AtomicU64,
    start: Option<std::time::Instant>,
}

impl<'a> ScopedNanos<'a> {
    #[inline]
    pub fn new(target: &'a AtomicU64) -> Self {
        Self {
            target,
            start: enabled().then(std::time::Instant::now),
        }
    }
}

impl Drop for ScopedNanos<'_> {
    #[inline]
    fn drop(&mut self) {
        if let Some(start) = self.start {
            self.target
                .fetch_add(start.elapsed().as_nanos() as u64, Relaxed);
        }
    }
}

/// Existence memo answers served without touching the filesystem.
pub static EXISTS_MEMO_HITS: AtomicU64 = AtomicU64::new(0);
/// Actual VFS existence probes (each is ≥1 `stat` syscall).
pub static EXISTS_FS_PROBES: AtomicU64 = AtomicU64::new(0);

// ---------------------------------------------------------------------------
// Slot-locals coverage (perf plan T3.1 / P2 sizing). The COMPILE-side counters
// classify every finalized function once; the RUNTIME pair weights those
// classes by how often frames of each kind are actually entered. Sizing P2
// needs both: static coverage says what fraction of code is reachable, frame
// counts say whether that code is the code that runs.
// ---------------------------------------------------------------------------
/// Functions that came out of `assign_local_slots` with ≥1 slot.
pub static SLOT_FN_SLOTTED: AtomicU64 = AtomicU64::new(0);
/// Functions disqualified WHOLESALE because the body defines a closure.
pub static SLOT_FN_DISQ_CLOSURE: AtomicU64 = AtomicU64::new(0);
/// Functions disqualified wholesale for any other reason (include, dynamic
/// var set, `structDelete(local)`, the reflective `evaluate` family).
pub static SLOT_FN_DISQ_OTHER: AtomicU64 = AtomicU64::new(0);
/// Per-reason split of `SLOT_FN_DISQ_OTHER` (functions / static ops / ops
/// before the first offending op — the prefix a spill-instead-of-disqualify
/// design would recover).
pub static SLOT_FN_DISQ_INCLUDE: AtomicU64 = AtomicU64::new(0);
pub static SLOT_OPS_DISQ_INCLUDE: AtomicU64 = AtomicU64::new(0);
pub static SLOT_OPS_INCLUDE_PREFIX: AtomicU64 = AtomicU64::new(0);
pub static SLOT_FN_DISQ_DYNVAR: AtomicU64 = AtomicU64::new(0);
pub static SLOT_OPS_DISQ_DYNVAR: AtomicU64 = AtomicU64::new(0);
pub static SLOT_FN_DISQ_REFLECTIVE: AtomicU64 = AtomicU64::new(0);
pub static SLOT_OPS_DISQ_REFLECTIVE: AtomicU64 = AtomicU64::new(0);
pub static SLOT_FN_DISQ_DELSCOPE: AtomicU64 = AtomicU64::new(0);
/// Eligible functions that simply had no slottable name (no `var`, or every
/// candidate excluded per-name).
pub static SLOT_FN_NO_CANDIDATES: AtomicU64 = AtomicU64::new(0);
/// Split of the above: bodies with no `var` declaration AT ALL (nothing stage 1
/// could ever slot — these are the params-and-`variables`-only methods that
/// P2 item 2 targets) vs bodies whose every declared name was excluded per-name.
pub static SLOT_FN_NO_DECLARES: AtomicU64 = AtomicU64::new(0);
pub static SLOT_FN_ALL_EXCLUDED: AtomicU64 = AtomicU64::new(0);
/// Declared params living in functions that ended up with no slots — the
/// upper bound on what "params into slots" would add.
pub static SLOT_PARAMS_UNSLOTTED_FNS: AtomicU64 = AtomicU64::new(0);
/// Frame entries into an unslotted function that HAS declared params (i.e. the
/// entries P2 item 2 could convert), and into one with none (unreachable by
/// any slot widening).
pub static FRAMES_UNSLOTTED_WITH_PARAMS: AtomicU64 = AtomicU64::new(0);
/// Op counts for the same three classes (static body length — the same
/// weighting the 2026-08 JIT admission scan used).
pub static SLOT_OPS_SLOTTED: AtomicU64 = AtomicU64::new(0);
pub static SLOT_OPS_DISQ_CLOSURE: AtomicU64 = AtomicU64::new(0);
pub static SLOT_OPS_DISQ_OTHER: AtomicU64 = AtomicU64::new(0);
/// Of `SLOT_OPS_DISQ_CLOSURE`, the ops that sit BEFORE the body's first
/// `DefineFunction`. That prefix is all a spill-on-DefineFunction design can
/// recover; the remainder needs per-name exclusion instead.
pub static SLOT_OPS_CLOSURE_PREFIX: AtomicU64 = AtomicU64::new(0);
/// Frame entries into a function that has slots vs one that has none.
pub static FRAMES_SLOTTED: AtomicU64 = AtomicU64::new(0);
pub static FRAMES_UNSLOTTED: AtomicU64 = AtomicU64::new(0);

#[inline]
pub fn add(c: &AtomicU64, n: u64) {
    c.fetch_add(n, Relaxed);
}

#[inline]
pub fn bump(c: &AtomicU64) {
    c.fetch_add(1, Relaxed);
}

/// Snapshot every counter as a one-block human-readable report.
pub fn report() -> String {
    let g = |c: &AtomicU64| c.load(Relaxed);
    #[allow(unused_mut)]
    let mut out = report_totals(g);
    #[cfg(feature = "alloc-sizing")]
    {
        out.push('\n');
        out.push_str(&alloc_sites::report(60));
    }
    #[cfg(feature = "probe-sites")]
    {
        out.push('\n');
        out.push_str(&probe_sites::totals());
        out.push('\n');
        out.push_str(&probe_sites::report(40));
    }
    #[cfg(feature = "exists-census")]
    {
        out.push('\n');
        out.push_str(&exists_census::report(25));
    }
    out
}

fn report_totals(g: impl Fn(&AtomicU64) -> u64) -> String {
    format!(
        "=== RUSTCFML_COUNTERS ===\n\
         struct_new (tracked):        {:>12}\n\
         struct_new_untracked:        {:>12}\n\
         --- arguments scope: eager vs lazy (Lever A) ---\n\
         frames eager:                {:>12}\n\
           .. callee has a default:   {:>12}\n\
         frames lazy (Lever A skip):  {:>12}\n\
         --- param binding shape (per bound frame) ---\n\
         bound frames:                {:>12}\n\
           .. params declared:        {:>12}\n\
           .. args supplied:          {:>12}\n\
           .. type validations:       {:>12}\n\
         --- return-time parent-scope diff (3A second half) ---\n\
         frames reaching the diff:    {:>12}\n\
           .. skipped futile:         {:>12}\n\
           .. locals entries scanned: {:>12}\n\
           .. entries compared:       {:>12}\n\
           .. entries written back:   {:>12}\n\
           .. total time (us):        {:>12}\n\
         --- parent-scope seed copy (3A first half) ---\n\
         frames seeding:              {:>12}\n\
           .. total time (us):        {:>12}\n\
         --- bare-name scope-chain resolution depth ---\n\
         lookups (total):             {:>12}\n\
           .. hit locals (slottable): {:>12}\n\
           .. hit arguments struct:   {:>12}\n\
           .. hit web scope:          {:>12}\n\
           .. hit __variables:        {:>12}\n\
           .. hit globals:            {:>12}\n\
           .. resolved nothing:       {:>12}\n\
         instances created:           {:>12}\n\
           .. `this` data keys:       {:>12}\n\
           .. `variables` data keys:  {:>12}\n\
         resolve_component calls:     {:>12}\n\
           .. path-cache hits:        {:>12}\n\
           .. candidate probe walks:  {:>12}\n\
         exists memo hits:            {:>12}\n\
         exists FS probes (stats):    {:>12}\n\
         --- component metadata (request-scoped memo) ---\n\
         getComponentMetaData(path):  {:>12}\n\
           .. memo hits:              {:>12}\n\
           .. miss cost (ms):         {:>12}\n\
           .. hit deep-copy (ms):     {:>12}\n\
           .. miss: resolve tmpl (ms):{:>12}\n\
           .. miss: resolve inh  (ms):{:>12}\n\
           .. miss: snapshot     (ms):{:>12}\n\
           .. miss: build meta   (ms):{:>12}\n\
         template exec: distinct:     {:>12}\n\
         template exec: REPEATS:      {:>12}\n\
         template exec: meta-cached:  {:>12}\n\
         inheritance builder levels:  {:>12}\n\
           .. memo hits:              {:>12}\n\
         builtin CI lookups:          {:>12}\n\
           .. miscased (CI fallback): {:>12}\n\
           .. index-stale O(n) scans: {:>12}\n\
         --- slot-locals coverage (functions / static ops) ---\n\
         slotted:                     {:>12} {:>12}\n\
         disq. closure-defining:      {:>12} {:>12}\n\
           .. ops before 1st closure: {:>25}\n\
         disq. other:                 {:>12} {:>12}\n\
           .. include:                {:>12} {:>12}\n\
           .... ops before 1st incl:  {:>25}\n\
           .. dynamic var set:        {:>12} {:>12}\n\
           .. reflective builtin:     {:>12} {:>12}\n\
           .. structDelete(local):    {:>12}\n\
         eligible, no candidates:     {:>12}\n\
           .. no `var` at all:        {:>12}\n\
           .. all names excluded:     {:>12}\n\
           .. params in unslotted fns:{:>12}\n\
         frame entries slotted:       {:>12}\n\
         frame entries unslotted:     {:>12}\n\
           .. of those, has params:   {:>12}",
        g(&STRUCT_NEW),
        g(&STRUCT_NEW_UNTRACKED),
        g(&FRAMES_ARGS_EAGER),
        g(&FRAMES_ARGS_EAGER_WITH_DEFAULTS),
        g(&FRAMES_ARGS_LAZY),
        g(&BIND_FRAMES),
        g(&BIND_PARAMS_DECLARED),
        g(&BIND_ARGS_SUPPLIED),
        g(&BIND_TYPECHECKS),
        g(&WB_FRAMES),
        g(&WB_SKIPPED_FUTILE),
        g(&WB_KEYS_SCANNED),
        g(&WB_KEYS_COMPARED),
        g(&WB_KEYS_WRITTEN),
        g(&WB_NANOS) / 1_000,
        g(&SEED_FRAMES),
        g(&SEED_NANOS) / 1_000,
        g(&SCOPE_LOOKUP_TOTAL),
        g(&SCOPE_HIT_LOCALS),
        g(&SCOPE_HIT_ARGUMENTS),
        g(&SCOPE_HIT_WEBSCOPE),
        g(&SCOPE_HIT_VARIABLES),
        g(&SCOPE_HIT_GLOBALS),
        g(&SCOPE_MISS),
        g(&INSTANCES_CREATED),
        g(&INSTANCE_THIS_KEYS),
        g(&INSTANCE_VARS_KEYS),
        g(&RESOLVE_CALLS),
        g(&RESOLVE_CACHE_HITS),
        g(&RESOLVE_PROBE_WALKS),
        g(&EXISTS_MEMO_HITS),
        g(&EXISTS_FS_PROBES),
        g(&META_PATH_CALLS),
        g(&META_PATH_HITS),
        g(&META_PATH_MISS_NANOS) / 1_000_000,
        g(&META_PATH_HIT_NANOS) / 1_000_000,
        g(&META_MISS_RESOLVE_NANOS) / 1_000_000,
        g(&META_MISS_INHERIT_NANOS) / 1_000_000,
        g(&META_MISS_SNAPSHOT_NANOS) / 1_000_000,
        g(&META_MISS_BUILD_NANOS) / 1_000_000,
        g(&RESOLVE_TEMPLATE_DISTINCT),
        g(&RESOLVE_TEMPLATE_REPEAT),
        g(&META_TEMPLATE_CACHE_HITS),
        g(&META_INHERIT_CALLS),
        g(&META_INHERIT_HITS),
        g(&BUILTIN_LOOKUP_CI),
        g(&BUILTIN_LOOKUP_CI_MISCASED),
        g(&BUILTIN_LOOKUP_CI_SCAN),
        g(&SLOT_FN_SLOTTED),
        g(&SLOT_OPS_SLOTTED),
        g(&SLOT_FN_DISQ_CLOSURE),
        g(&SLOT_OPS_DISQ_CLOSURE),
        g(&SLOT_OPS_CLOSURE_PREFIX),
        g(&SLOT_FN_DISQ_OTHER),
        g(&SLOT_OPS_DISQ_OTHER),
        g(&SLOT_FN_DISQ_INCLUDE),
        g(&SLOT_OPS_DISQ_INCLUDE),
        g(&SLOT_OPS_INCLUDE_PREFIX),
        g(&SLOT_FN_DISQ_DYNVAR),
        g(&SLOT_OPS_DISQ_DYNVAR),
        g(&SLOT_FN_DISQ_REFLECTIVE),
        g(&SLOT_OPS_DISQ_REFLECTIVE),
        g(&SLOT_FN_DISQ_DELSCOPE),
        g(&SLOT_FN_NO_CANDIDATES),
        g(&SLOT_FN_NO_DECLARES),
        g(&SLOT_FN_ALL_EXCLUDED),
        g(&SLOT_PARAMS_UNSLOTTED_FNS),
        g(&FRAMES_SLOTTED),
        g(&FRAMES_UNSLOTTED),
        g(&FRAMES_UNSLOTTED_WITH_PARAMS),
    )
}

/// Dynamic op census (`op-census` builds only) — how many times each bytecode
/// op is actually EXECUTED, as opposed to how many times it appears in compiled
/// bodies. The 2026-08 JIT admission scan weighted ops statically; a Tier-0
/// design has to be sized against the dynamic mix, because loops mean a
/// function's ops do not execute once per frame entry. Indexed by
/// `BytecodeOp::census_index()`; the VM's dispatch loop bumps one relaxed add
/// per op, which is why this lives behind a feature and never ships on.
#[cfg(feature = "op-census")]
pub mod op_census {
    use std::sync::atomic::{AtomicU64, Ordering::Relaxed};

    /// Room for every `BytecodeOp` variant (121 today) plus headroom, so adding
    /// an op cannot silently start writing out of bounds.
    pub const SLOTS: usize = 192;

    #[allow(clippy::declare_interior_mutable_const)]
    const ZERO: AtomicU64 = AtomicU64::new(0);
    pub static COUNTS: [AtomicU64; SLOTS] = [ZERO; SLOTS];

    #[inline]
    pub fn bump(idx: usize) {
        if let Some(c) = COUNTS.get(idx) {
            c.fetch_add(1, Relaxed);
        }
    }

    /// Snapshot every non-zero slot as `(index, count)`, so callers can diff two
    /// snapshots to isolate one request's mix from cumulative boot totals.
    pub fn snapshot() -> Vec<(usize, u64)> {
        COUNTS
            .iter()
            .enumerate()
            .map(|(i, c)| (i, c.load(Relaxed)))
            .filter(|(_, n)| *n > 0)
            .collect()
    }

    /// Human-readable descending report. `names` comes from
    /// `BytecodeOp::CENSUS_NAMES` (cfml-common cannot depend on cfml-codegen,
    /// so the embedder passes it in).
    pub fn report(names: &[&str]) -> String {
        let mut rows = snapshot();
        let total: u64 = rows.iter().map(|(_, n)| *n).sum();
        rows.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
        let mut out = format!(
            "--- dynamic op census: {} ops executed across {} distinct opcodes ---",
            total,
            rows.len()
        );
        let mut cum = 0u64;
        for (i, n) in &rows {
            cum += n;
            out.push_str(&format!(
                "\n{:>14} {:>6.2}% {:>6.2}%cum  {}",
                n,
                *n as f64 / total.max(1) as f64 * 100.0,
                cum as f64 / total.max(1) as f64 * 100.0,
                names.get(*i).copied().unwrap_or("<unknown>"),
            ));
        }
        out
    }
}

/// Phase attribution for the UDF call prologue (`call-phases` builds only).
///
/// The 2026-08-11 op census established that a UDF call costs ~700 ns of frame
/// machinery against ~60 ns for a builtin dispatch, i.e. ~640 ns of pure setup,
/// and that this — not interpreter dispatch — is the warm-request lever. Sizing
/// the rework needs to know WHICH of the prologue's phases owns that budget, so
/// this accumulates nanoseconds per phase across every call.
///
/// The clock itself is not free (`Instant::now()` is tens of ns on macOS), so
/// [`CLOCK_CAL_NS`] accumulates the cost of a back-to-back pair of reads on the
/// same path. Subtract `CLOCK_CAL_NS / CALLS` per phase boundary before reading
/// any phase total as a real cost.
#[cfg(feature = "call-phases")]
pub mod call_phases {
    use std::sync::atomic::{AtomicU64, Ordering::Relaxed};

    pub const N: usize = 32;

    #[allow(clippy::declare_interior_mutable_const)]
    const ZERO: AtomicU64 = AtomicU64::new(0);
    /// Cumulative nanoseconds per phase.
    pub static NS: [AtomicU64; N] = [ZERO; N];
    /// Frames measured (every `execute_function_body` entry).
    pub static CALLS: AtomicU64 = AtomicU64::new(0);
    /// Cumulative cost of one extra clock read per call — the calibration term.
    pub static CLOCK_CAL_NS: AtomicU64 = AtomicU64::new(0);

    #[inline]
    pub fn bump_calls() {
        CALLS.fetch_add(1, Relaxed);
    }

    /// Calls whose callee carried a captured scope, so the caller pre-call
    /// (phase 8) had to CLONE the whole captured env map before merging the
    /// caller's locals into it. Paired with the key count, this says whether
    /// that clone is worth removing with a layered scope view.
    pub static ENV_CLONE_CALLS: AtomicU64 = AtomicU64::new(0);
    /// Keys copied by those clones (env size + the caller locals merged in).
    pub static ENV_CLONE_KEYS: AtomicU64 = AtomicU64::new(0);
    /// Calls that passed the caller's locals straight through (no clone).
    pub static ENV_PASSTHROUGH_CALLS: AtomicU64 = AtomicU64::new(0);

    #[inline]
    pub fn bump_env_clone(keys: u64) {
        ENV_CLONE_CALLS.fetch_add(1, Relaxed);
        ENV_CLONE_KEYS.fetch_add(keys, Relaxed);
    }

    #[inline]
    pub fn bump_env_passthrough() {
        ENV_PASSTHROUGH_CALLS.fetch_add(1, Relaxed);
    }

    /// Frames that built the `arguments` scope EAGERLY vs took the lazy path
    /// (Lever A). Phase 4 is 338 ns/frame on live Preside, so which side of this
    /// branch the real workload sits on decides whether the fix is "make eager
    /// cheaper" or "make more calls lazy".
    pub static ARGS_EAGER: AtomicU64 = AtomicU64::new(0);
    pub static ARGS_LAZY: AtomicU64 = AtomicU64::new(0);
    /// Which disjunct of `build_arguments_eager` fired (first-wins order:
    /// template frame, overflow args, body actually references `arguments`).
    /// Decides whether widening laziness is even possible.
    pub static ARGS_EAGER_TEMPLATE: AtomicU64 = AtomicU64::new(0);
    pub static ARGS_EAGER_OVERFLOW: AtomicU64 = AtomicU64::new(0);
    pub static ARGS_EAGER_REFERENCED: AtomicU64 = AtomicU64::new(0);
    /// Frames whose classic-localMode parent write-back diff ran, and of those,
    /// how many actually produced any write-back (phase-7 split item 16 is
    /// 44% of the Return arm — if almost nothing is written back, the whole diff
    /// is avoidable work).
    pub static CLOSURE_WB_SCANNED: AtomicU64 = AtomicU64::new(0);
    pub static CLOSURE_WB_NONEMPTY: AtomicU64 = AtomicU64::new(0);
    pub static CLOSURE_WB_KEYS_SCANNED: AtomicU64 = AtomicU64::new(0);
    pub static CLOSURE_WB_KEYS_WRITTEN: AtomicU64 = AtomicU64::new(0);
    /// Frames whose `Return` arm did the component-method `this`/`variables`
    /// write-back (phase 7's expensive branch) vs those that skipped it.
    pub static RET_THIS_WRITEBACK: AtomicU64 = AtomicU64::new(0);
    pub static RET_PLAIN: AtomicU64 = AtomicU64::new(0);
    /// Inside the write-back diff: how many keys the `argument_scope_key_set`
    /// HashSet lowercases and allocates per frame (a FIXED per-frame cost paid
    /// before the key loop), and how many scanned locals actually reach that
    /// set's `contains` probe — i.e. survive the cheap `__`/param/declared
    /// filters. If almost none reach it, building the set eagerly is pure waste.
    pub static ARGSET_BUILT: AtomicU64 = AtomicU64::new(0);
    pub static ARGSET_KEYS: AtomicU64 = AtomicU64::new(0);
    pub static ARGSET_PROBED: AtomicU64 = AtomicU64::new(0);
    /// Worst case seen on any SINGLE frame. The allocation-free probe scans the
    /// `arguments` keys per probing local, so its worst-case work is
    /// probes x argKeys on one frame — where a build-once HashSet would have won.
    /// These maxima say whether that tail is real or hypothetical.
    pub static WB_MAX_ARGKEYS: AtomicU64 = AtomicU64::new(0);
    pub static WB_MAX_PROBES: AtomicU64 = AtomicU64::new(0);
    pub static WB_MAX_PRODUCT: AtomicU64 = AtomicU64::new(0);

    #[inline]
    pub fn bump_calls_args(eager: bool) {
        if eager { ARGS_EAGER.fetch_add(1, Relaxed) } else { ARGS_LAZY.fetch_add(1, Relaxed) };
    }

    #[inline]
    pub fn bump_ret(has_this: bool) {
        if has_this { RET_THIS_WRITEBACK.fetch_add(1, Relaxed) } else { RET_PLAIN.fetch_add(1, Relaxed) };
    }

    #[inline]
    pub fn bump_eager_reason(template: bool, overflow: bool) {
        if template {
            ARGS_EAGER_TEMPLATE.fetch_add(1, Relaxed);
        } else if overflow {
            ARGS_EAGER_OVERFLOW.fetch_add(1, Relaxed);
        } else {
            ARGS_EAGER_REFERENCED.fetch_add(1, Relaxed);
        }
    }

    #[inline]
    pub fn record_closure_wb(keys_scanned: u64, keys_written: u64) {
        CLOSURE_WB_SCANNED.fetch_add(1, Relaxed);
        CLOSURE_WB_KEYS_SCANNED.fetch_add(keys_scanned, Relaxed);
        CLOSURE_WB_KEYS_WRITTEN.fetch_add(keys_written, Relaxed);
        if keys_written > 0 {
            CLOSURE_WB_NONEMPTY.fetch_add(1, Relaxed);
        }
    }

    #[inline]
    pub fn record_wb_argset(keys: u64) {
        ARGSET_BUILT.fetch_add(1, Relaxed);
        ARGSET_KEYS.fetch_add(keys, Relaxed);
    }

    #[inline]
    pub fn bump_wb_argset_probe() {
        ARGSET_PROBED.fetch_add(1, Relaxed);
    }

    /// Per-frame maxima for the tail analysis above.
    pub fn record_wb_frame(argkeys: u64, probes: u64) {
        WB_MAX_ARGKEYS.fetch_max(argkeys, Relaxed);
        WB_MAX_PROBES.fetch_max(probes, Relaxed);
        WB_MAX_PRODUCT.fetch_max(argkeys * probes, Relaxed);
    }

    pub fn branch_report() -> String {
        let g = |c: &AtomicU64| c.load(Relaxed);
        let (e, l) = (g(&ARGS_EAGER), g(&ARGS_LAZY));
        let (t, pl) = (g(&RET_THIS_WRITEBACK), g(&RET_PLAIN));
        format!(
            "--- caller pre-call scope handling (phase 8) ---\n\
             env CLONED (closure callee):  {:>12}\n\
               .. keys copied:             {:>12}\n\
             locals passed through:        {:>12}\n\
             --- call-path branch split ---\n\
             arguments scope eager:  {:>12}  ({:.1}%)\n\
             arguments scope lazy:   {:>12}  ({:.1}%)\n\
             Return with this-wb:    {:>12}  ({:.1}%)\n\
             Return plain:           {:>12}  ({:.1}%)",
            g(&ENV_CLONE_CALLS), g(&ENV_CLONE_KEYS), g(&ENV_PASSTHROUGH_CALLS),
            e, e as f64 / (e + l).max(1) as f64 * 100.0,
            l, l as f64 / (e + l).max(1) as f64 * 100.0,
            t, t as f64 / (t + pl).max(1) as f64 * 100.0,
            pl, pl as f64 / (t + pl).max(1) as f64 * 100.0,
        ) + &format!(
            "\n  eager because template frame:  {:>12}\n\
               eager because overflow args:   {:>12}\n\
               eager because body uses it:    {:>12}\n\
             classic-localMode wb diffs run:  {:>12}\n\
               .. that wrote ANY key:         {:>12}  ({:.1}%)\n\
               .. keys scanned / written:     {:>12} / {}\n\
               .. argset builds / keys lc'd:  {:>12} / {}\n\
               .. locals reaching argset:     {:>12}\n\
               .. WORST single frame: argKeys {:>4}, probes {:>4}, product {:>6}",
            g(&ARGS_EAGER_TEMPLATE),
            g(&ARGS_EAGER_OVERFLOW),
            g(&ARGS_EAGER_REFERENCED),
            g(&CLOSURE_WB_SCANNED),
            g(&CLOSURE_WB_NONEMPTY),
            g(&CLOSURE_WB_NONEMPTY) as f64 / g(&CLOSURE_WB_SCANNED).max(1) as f64 * 100.0,
            g(&CLOSURE_WB_KEYS_SCANNED),
            g(&CLOSURE_WB_KEYS_WRITTEN),
            g(&ARGSET_BUILT),
            g(&ARGSET_KEYS),
            g(&ARGSET_PROBED),
            g(&WB_MAX_ARGKEYS),
            g(&WB_MAX_PROBES),
            g(&WB_MAX_PRODUCT),
        )
    }

    /// Phase-4 sub-split census (arguments scope + param binding + required
    /// check). Same design as the phase-8 split that found `arg_sources`: the
    /// EXACT frequencies decide, the three sub-timers (phases 29-31) only
    /// corroborate — at a ~17 ns clock floor a fine-grained split of a ~230 ns
    /// phase measures its own instrument.
    ///
    /// The suspects, in code order:
    ///  - two `global_id`-keyed HashMap memo probes (`arguments_scope_needed`,
    ///    `arguments_scope_can_skip_tracking`) — SipHash per probe, per call,
    ///    for bits that are pure functions of the bytecode (codegen could
    ///    precompute both);
    ///  - the eager `ValueMap::with_capacity` allocation;
    ///  - the param-binding loop (+ a SECOND full loop for the required check);
    ///  - the `arguments_params_cache` probe + markers on the eager tail;
    ///  - the `CfmlStruct` alloc, tracked (cycle-GC log) vs untracked.
    /// Calls that consulted the `arguments_scope_needed` memo (i.e. were not
    /// short-circuited by template-frame / overflow-args).
    pub static P4_NEEDED_PROBES: AtomicU64 = AtomicU64::new(0);
    /// Calls that additionally consulted the Lever-C untracking memo.
    pub static P4_UNTRACK_PROBES: AtomicU64 = AtomicU64::new(0);
    /// Declared params iterated (NOTE: the binding loop and the required-check
    /// loop each iterate all of them — the per-call work is 2x this rate).
    pub static P4_PARAMS_ITER: AtomicU64 = AtomicU64::new(0);
    /// Of those, actually supplied by the caller (bound into locals).
    pub static P4_PARAMS_SUPPLIED: AtomicU64 = AtomicU64::new(0);
    /// Supplied params that carried a declared type and ran the type check.
    pub static P4_TYPECHECKS: AtomicU64 = AtomicU64::new(0);
    /// Eager frames that probed `arguments_params_cache` (declared params > 0).
    pub static P4_MARKER_PROBES: AtomicU64 = AtomicU64::new(0);
    /// Eager arguments structs allocated tracked (cycle-GC logged) vs
    /// untracked (Lever C fired).
    pub static P4_STRUKT_TRACKED: AtomicU64 = AtomicU64::new(0);
    pub static P4_STRUKT_UNTRACKED: AtomicU64 = AtomicU64::new(0);
    /// `__main__` frames that seeded arguments from the parent (include bridge).
    pub static P4_MAIN_SEEDS: AtomicU64 = AtomicU64::new(0);

    #[inline]
    pub fn bump_p4_needed_probe() {
        P4_NEEDED_PROBES.fetch_add(1, Relaxed);
    }
    #[inline]
    pub fn bump_p4_untrack_probe() {
        P4_UNTRACK_PROBES.fetch_add(1, Relaxed);
    }
    /// One call per frame, after the binding loop — aggregates locally first so
    /// the loop body stays free of atomics.
    #[inline]
    pub fn record_p4_binding(params: u64, supplied: u64, typechecks: u64) {
        P4_PARAMS_ITER.fetch_add(params, Relaxed);
        P4_PARAMS_SUPPLIED.fetch_add(supplied, Relaxed);
        P4_TYPECHECKS.fetch_add(typechecks, Relaxed);
    }
    #[inline]
    pub fn record_p4_eager_tail(marker_probe: bool, untracked: bool, main_seed: bool) {
        if marker_probe {
            P4_MARKER_PROBES.fetch_add(1, Relaxed);
        }
        if untracked {
            P4_STRUKT_UNTRACKED.fetch_add(1, Relaxed);
        } else {
            P4_STRUKT_TRACKED.fetch_add(1, Relaxed);
        }
        if main_seed {
            P4_MAIN_SEEDS.fetch_add(1, Relaxed);
        }
    }

    pub fn p4_report() -> String {
        let g = |c: &AtomicU64| c.load(Relaxed);
        let calls = CALLS.load(Relaxed).max(1);
        let pct = |v: u64| v as f64 / calls as f64 * 100.0;
        let eager = (g(&P4_STRUKT_TRACKED) + g(&P4_STRUKT_UNTRACKED)).max(1);
        format!(
            "--- phase 4 sub-split census ({} frames) ---\n\
             needed-memo probes (HashMap):  {:>12}  ({:.1}% of frames)\n\
             untrack-memo probes (HashMap): {:>12}  ({:.1}%)\n\
             params iterated / frame:       {:>12}  ({:.2} — x2: bind + required loops)\n\
               .. supplied (bound):         {:>12}  ({:.2}/frame)\n\
               .. type-checked:             {:>12}\n\
             eager tails:                   {:>12}  ({:.1}% of frames)\n\
               .. marker-cache probes:      {:>12}  ({:.1}% of eager)\n\
               .. strukt tracked (GC log):  {:>12}  ({:.1}% of eager)\n\
               .. strukt untracked:         {:>12}\n\
               .. __main__ parent seeds:    {:>12}",
            calls,
            g(&P4_NEEDED_PROBES), pct(g(&P4_NEEDED_PROBES)),
            g(&P4_UNTRACK_PROBES), pct(g(&P4_UNTRACK_PROBES)),
            g(&P4_PARAMS_ITER), g(&P4_PARAMS_ITER) as f64 / calls as f64,
            g(&P4_PARAMS_SUPPLIED), g(&P4_PARAMS_SUPPLIED) as f64 / calls as f64,
            g(&P4_TYPECHECKS),
            eager, pct(eager),
            g(&P4_MARKER_PROBES), g(&P4_MARKER_PROBES) as f64 / eager as f64 * 100.0,
            g(&P4_STRUKT_TRACKED), g(&P4_STRUKT_TRACKED) as f64 / eager as f64 * 100.0,
            g(&P4_STRUKT_UNTRACKED),
            g(&P4_MAIN_SEEDS),
        )
    }

    /// Phase-8 sub-split census (the caller's pre-call window in the `Call`
    /// arm). The env-clone hypothesis is already dead — 1 call in 588 — so the
    /// remaining 139.9 ns/frame is spread across `arg_sources`, the argument
    /// `Vec`, the slot-spill probe and the try-stack isolation. These are exact
    /// frequencies rather than timings: at a ~17 ns clock floor a five-way
    /// sub-split of a 140 ns phase is measuring its own instrument, so the
    /// counters decide and the sub-timers only corroborate.
    pub static P8_CALLS: AtomicU64 = AtomicU64::new(0);
    /// Calls whose `arg_sources_cached` memo probe hit vs had to scan.
    pub static P8_ARGSRC_HIT: AtomicU64 = AtomicU64::new(0);
    pub static P8_ARGSRC_MISS: AtomicU64 = AtomicU64::new(0);
    /// Of the hits, how many returned a vector with ANY `Some` source. Every
    /// call pays a `HashMap` probe + `Arc` clone to obtain this; if it is
    /// almost always all-`None` the whole lookup can collapse to a cached bool.
    pub static P8_ARGSRC_USEFUL: AtomicU64 = AtomicU64::new(0);
    /// Calls where the callee actually reported a by-ref mutation, i.e. the
    /// only calls that still need `arg_sources` now the lookup is deferred.
    /// This is the RESIDUAL of the phase-24 lever.
    pub static P8_ARGREF_PRESENT: AtomicU64 = AtomicU64::new(0);
    /// Arguments popped, and calls that allocated a heap `Vec` to hold them
    /// (`arg_count > 0`). One malloc/free pair per call is the single largest
    /// allocation on the call path if the count is near 100%.
    pub static P8_ARGS_TOTAL: AtomicU64 = AtomicU64::new(0);
    pub static P8_ARGS_VEC_ALLOC: AtomicU64 = AtomicU64::new(0);
    /// Calls that reached the slot-spill probe (`!slots.is_empty()`), i.e. paid
    /// `callee_reflects_on_caller_scope`'s seven `eq_ignore_ascii_case`
    /// comparisons, vs those that actually spilled.
    pub static P8_SLOT_PROBED: AtomicU64 = AtomicU64::new(0);
    pub static P8_SLOT_SPILLED: AtomicU64 = AtomicU64::new(0);
    /// Calls that had to `mem::take` a non-empty try-stack and restore it.
    pub static P8_TRY_SAVED: AtomicU64 = AtomicU64::new(0);

    /// Bumped inside `arg_sources_cached`, which already knows which side of
    /// the memo it took — re-probing at the call site would double the hash
    /// lookup being measured.
    #[inline]
    pub fn bump_p8_argsrc_memo(hit: bool) {
        if hit { P8_ARGSRC_HIT.fetch_add(1, Relaxed) } else { P8_ARGSRC_MISS.fetch_add(1, Relaxed) };
    }

    #[inline]
    pub fn bump_p8_argref_present() {
        P8_ARGREF_PRESENT.fetch_add(1, Relaxed);
    }

    #[inline]
    pub fn record_p8_args(n: u64) {
        P8_CALLS.fetch_add(1, Relaxed);
        P8_ARGS_TOTAL.fetch_add(n, Relaxed);
        if n > 0 {
            P8_ARGS_VEC_ALLOC.fetch_add(1, Relaxed);
        }
    }

    #[inline]
    pub fn record_p8_slot(spilled: bool) {
        P8_SLOT_PROBED.fetch_add(1, Relaxed);
        if spilled {
            P8_SLOT_SPILLED.fetch_add(1, Relaxed);
        }
    }

    #[inline]
    pub fn bump_p8_try_saved() {
        P8_TRY_SAVED.fetch_add(1, Relaxed);
    }

    pub fn p8_report() -> String {
        let g = |c: &AtomicU64| c.load(Relaxed);
        let calls = g(&P8_CALLS).max(1);
        let pct = |v: u64| v as f64 / calls as f64 * 100.0;
        format!(
            "--- phase 8 sub-split census ({} Call-op executions) ---\n\
             arg_sources RESOLVED:        {:>12}  ({:.1}%)\n\
               .. memo miss (scan):       {:>12}\n\
               .. by-ref writeback fired: {:>12}  ({:.1}%)\n\
             args popped (total / per call): {:>9} / {:.2}\n\
               .. calls allocating a Vec: {:>12}  ({:.1}%)\n\
             slot-spill probe reached:    {:>12}  ({:.1}%)\n\
               .. actually spilled:       {:>12}  ({:.1}%)\n\
             try-stack saved+restored:    {:>12}  ({:.1}%)",
            calls,
            g(&P8_ARGSRC_HIT) + g(&P8_ARGSRC_MISS),
            pct(g(&P8_ARGSRC_HIT) + g(&P8_ARGSRC_MISS)),
            g(&P8_ARGSRC_MISS),
            g(&P8_ARGREF_PRESENT), pct(g(&P8_ARGREF_PRESENT)),
            g(&P8_ARGS_TOTAL), g(&P8_ARGS_TOTAL) as f64 / calls as f64,
            g(&P8_ARGS_VEC_ALLOC), pct(g(&P8_ARGS_VEC_ALLOC)),
            g(&P8_SLOT_PROBED), pct(g(&P8_SLOT_PROBED)),
            g(&P8_SLOT_SPILLED), pct(g(&P8_SLOT_SPILLED)),
            g(&P8_TRY_SAVED), pct(g(&P8_TRY_SAVED)),
        )
    }

    #[inline]
    pub fn add(phase: usize, ns: u64) {
        if let Some(c) = NS.get(phase) {
            c.fetch_add(ns, Relaxed);
        }
    }

    pub fn report(labels: &[&str]) -> String {
        let calls = CALLS.load(Relaxed).max(1);
        let cal = CLOCK_CAL_NS.load(Relaxed) as f64 / calls as f64;
        let tot: u64 = NS.iter().map(|c| c.load(Relaxed)).sum();
        let mut out = format!(
            "--- UDF call-prologue phases: {} frames, {:.1} ns/frame measured \
             (clock read ≈ {:.1} ns, already subtracted below) ---",
            calls,
            tot as f64 / calls as f64,
            cal
        );
        for (i, label) in labels.iter().enumerate() {
            let raw = NS[i].load(Relaxed) as f64 / calls as f64;
            let net = (raw - cal).max(0.0);
            out.push_str(&format!("\n{:>9.1} ns/frame  {}", net, label));
        }
        out
    }
}

/// Call-site census for keyed lookups that fold-hash a `&str` at the probe
/// site (`probe-sites` builds only).
///
/// The [`PROBE_PRECOMPUTED`]/[`PROBE_HASHED`] pair says how much of the
/// interned-key lever is captured; this says WHICH call sites still owe. A
/// site listed here is one `&str` probe away from being free — convert it to
/// take a `Name`/`Key` and its whole count moves to the precomputed column.
#[cfg(feature = "probe-sites")]
pub mod probe_sites {
    /// Totals for the two probe classes, reported alongside the site list.
    pub fn totals() -> String {
        use super::{Relaxed, PROBE_HASHED, PROBE_PRECOMPUTED};
        use std::sync::atomic::Ordering;
        let _ = Relaxed;
        let (p, h) = (
            PROBE_PRECOMPUTED.load(Ordering::Relaxed),
            PROBE_HASHED.load(Ordering::Relaxed),
        );
        let tot = (p + h).max(1);
        format!(
            "--- key probes: {p} precomputed ({:.1}%) / {h} hashed at site ({:.1}%) ---",
            p as f64 / tot as f64 * 100.0,
            h as f64 / tot as f64 * 100.0
        )
    }

    use parking_lot::Mutex;
    use std::collections::HashMap;
    use std::panic::Location;

    static SITES: Mutex<Option<HashMap<&'static Location<'static>, u64>>> = Mutex::new(None);

    #[inline]
    #[track_caller]
    pub fn record() {
        let loc = Location::caller();
        let mut g = SITES.lock();
        *g.get_or_insert_with(HashMap::new).entry(loc).or_default() += 1;
    }

    /// Cumulative top-`n` hashing probe sites, most frequent first.
    pub fn report(n: usize) -> String {
        let g = SITES.lock();
        let Some(m) = g.as_ref() else {
            return String::from("--- hashed probe sites: none recorded ---");
        };
        let mut rows: Vec<_> = m.iter().map(|(l, c)| (*l, *c)).collect();
        rows.sort_by_key(|(_, c)| std::cmp::Reverse(*c));
        let total: u64 = rows.iter().map(|(_, c)| c).sum();
        let mut out = format!(
            "--- hashed probe sites (cumulative; {total} total) ---"
        );
        for (loc, c) in rows.into_iter().take(n) {
            out.push_str(&format!("\n{:>12}  {:>5.1}%   {}:{}", c,
                c as f64 / total as f64 * 100.0, loc.file(), loc.line()));
        }
        out
    }
}

/// Call-site attribution for `CfmlStruct` construction (`alloc-sizing` builds
/// only). Every constructor in the chain (`new`/`new_untracked` and their thin
/// wrappers) carries `#[track_caller]`, so `Location::caller()` here resolves
/// to the real VM/stdlib call site, not the wrapper.
#[cfg(feature = "alloc-sizing")]
pub mod alloc_sites {
    use parking_lot::Mutex;
    use std::collections::HashMap;
    use std::panic::Location;

    /// site → [tracked, untracked] construction counts (cumulative).
    static SITES: Mutex<Option<HashMap<&'static Location<'static>, [u64; 2]>>> =
        Mutex::new(None);

    #[inline]
    #[track_caller]
    pub fn record(tracked: bool) {
        let loc = Location::caller();
        let mut g = SITES.lock();
        let m = g.get_or_insert_with(HashMap::new);
        m.entry(loc).or_default()[if tracked { 0 } else { 1 }] += 1;
    }

    /// Cumulative top-`n` sites, sorted by total count descending. Diff two
    /// consecutive reports to isolate a single request's shape.
    pub fn report(n: usize) -> String {
        let g = SITES.lock();
        let Some(m) = g.as_ref() else {
            return String::from("--- struct-alloc sites: none recorded ---");
        };
        let mut rows: Vec<_> = m.iter().map(|(l, c)| (*l, *c)).collect();
        rows.sort_by_key(|(_, c)| std::cmp::Reverse(c[0] + c[1]));
        let mut out = String::from("--- struct-alloc sites (cumulative; tracked/untracked) ---");
        for (loc, [t, u]) in rows.into_iter().take(n) {
            out.push_str(&format!(
                "\n{:>12} {:>12}   {}:{}",
                t,
                u,
                loc.file(),
                loc.line()
            ));
        }
        out
    }
}

/// Existence-probe outcome census (`exists-census` builds only) — the sizing
/// instrument for the application-lifetime existence caching closed in v0.598.0.
///
/// The shipped [`EXISTS_MEMO_HITS`] / [`EXISTS_FS_PROBES`] pair says the memo
/// takes more real syscalls than it serves hits, but it cannot say *what to
/// build*, because `request_exists_cache` holds POSITIVES ONLY: a negative
/// probe can never be a hit, so it is invisible to a hit-rate. The question
/// that decides the design is how many probes are REPEATS of a path already
/// probed — recoverable by a cache — versus genuine first looks, which no cache
/// can remove. This splits every probe six ways to answer exactly that:
///
/// | class | what would recover it |
/// |---|---|
/// | `pos_first` / `neg_first` | nothing — irreducible filesystem truth |
/// | `pos_repeat_req` | **only reachable after a wholesale memo clear** — measures the cost of the blanket `.clear()` |
/// | `pos_repeat_xreq` | a cross-request positive layer (already measured dead: −1% homepage, 0% admin) |
/// | `neg_repeat_req` | a request-scoped negative memo |
/// | `neg_repeat_xreq` | an application-lifetime negative memo |
///
/// Retains every probed path in a `Mutex`'d map (that is also how it reports
/// the distinct-path count, i.e. what a negative cache would have to hold), so
/// this is a sizing build only.
#[cfg(feature = "exists-census")]
pub mod exists_census {
    use super::{AtomicU64, Relaxed};
    use parking_lot::Mutex;
    use std::collections::HashMap;

    /// What we have already seen for one (path, probe-kind) pair.
    #[derive(Default)]
    struct Seen {
        /// Epoch of the most recent probe that found the path.
        last_pos_epoch: Option<u64>,
        /// Epoch of the most recent probe that did NOT find the path.
        last_neg_epoch: Option<u64>,
        /// Total probes that did not find it — ranks the worst offenders.
        neg_probes: u64,
    }

    #[derive(Default)]
    struct Tally {
        memo_hits: u64,
        pos_first: u64,
        pos_repeat_req: u64,
        pos_repeat_xreq: u64,
        neg_first: u64,
        neg_repeat_req: u64,
        neg_repeat_xreq: u64,
    }

    static STATE: Mutex<Option<(HashMap<(String, u8), Seen>, Tally)>> = Mutex::new(None);

    /// Monotonic request epoch. The VM takes one per `Vm` construction, which in
    /// serve mode is one per request, and passes it back into [`record`] — so
    /// "same request" is decided by the caller rather than by a thread-local,
    /// and stays correct under concurrent requests.
    static EPOCH: AtomicU64 = AtomicU64::new(0);

    /// Claim the next request epoch.
    pub fn next_epoch() -> u64 {
        EPOCH.fetch_add(1, Relaxed)
    }

    /// Names of builtins that retired the cached negatives, with counts — the
    /// list that decides which creators are worth attributing to a specific path
    /// instead of firing the coarse global retirement.
    static CREATORS: Mutex<Option<HashMap<String, u64>>> = Mutex::new(None);

    /// One builtin call that retired the negatives.
    pub fn record_creator(name_lower: &str) {
        let mut g = CREATORS.lock();
        let m = g.get_or_insert_with(HashMap::new);
        *m.entry(name_lower.to_string()).or_insert(0) += 1;
    }

    /// A probe answered from the memo without touching the filesystem.
    pub fn record_memo_hit() {
        let mut g = STATE.lock();
        let (_, t) = g.get_or_insert_with(Default::default);
        t.memo_hits += 1;
    }

    /// Classify one real filesystem probe of `path` under `bit`.
    pub fn record(path: &str, bit: u8, found: bool, epoch: u64) {
        let mut g = STATE.lock();
        let (m, t) = g.get_or_insert_with(Default::default);
        // `raw_entry` is unstable, so the miss path pays one allocation for the
        // key. This is a sizing build; correctness of the count is what matters.
        let seen = m.entry((path.to_string(), bit)).or_default();
        if found {
            match seen.last_pos_epoch {
                // Only reachable if something wiped the memo mid-request: the
                // memo would otherwise have served this as a hit.
                Some(e) if e == epoch => t.pos_repeat_req += 1,
                Some(_) => t.pos_repeat_xreq += 1,
                None => t.pos_first += 1,
            }
            seen.last_pos_epoch = Some(epoch);
        } else {
            match seen.last_neg_epoch {
                Some(e) if e == epoch => t.neg_repeat_req += 1,
                Some(_) => t.neg_repeat_xreq += 1,
                None => t.neg_first += 1,
            }
            seen.last_neg_epoch = Some(epoch);
            seen.neg_probes += 1;
        }
    }

    /// The census block, plus the top-`n` most-re-probed absent paths.
    pub fn report(n: usize) -> String {
        let g = STATE.lock();
        let Some((m, t)) = g.as_ref() else {
            return String::from("--- exists census: nothing recorded ---");
        };
        let epochs = EPOCH.load(Relaxed).max(1);
        let probes = t.pos_first
            + t.pos_repeat_req
            + t.pos_repeat_xreq
            + t.neg_first
            + t.neg_repeat_req
            + t.neg_repeat_xreq;
        let neg = t.neg_first + t.neg_repeat_req + t.neg_repeat_xreq;
        // What each candidate cache would actually have removed.
        let recoverable = t.neg_repeat_req + t.neg_repeat_xreq + t.pos_repeat_req + t.pos_repeat_xreq;
        let pct = |v: u64| {
            if probes == 0 {
                0.0
            } else {
                v as f64 * 100.0 / probes as f64
            }
        };
        let distinct_absent = m.values().filter(|s| s.last_neg_epoch.is_some()).count();
        let mut out = format!(
            "--- exists census ({} request epochs; {} probes, {} memo hits) ---\n\
             {:>12}  positive, first ever          {:>5.1}%\n\
             {:>12}  positive, repeat SAME request {:>5.1}%  <- cost of the wholesale memo clear\n\
             {:>12}  positive, repeat later req    {:>5.1}%  <- cross-request positive layer (measured dead)\n\
             {:>12}  negative, first ever          {:>5.1}%  <- irreducible\n\
             {:>12}  negative, repeat SAME request {:>5.1}%  <- request-scoped negative memo\n\
             {:>12}  negative, repeat later req    {:>5.1}%  <- application-lifetime negative memo\n\
             {:>12}  negative probes total         {:>5.1}%\n\
             {:>12}  RECOVERABLE by some cache     {:>5.1}%\n\
             {:>12}  distinct absent paths (a negative cache must hold these)\n\
             {:>12.1}  probes per request epoch",
            epochs,
            probes,
            t.memo_hits,
            t.pos_first,
            pct(t.pos_first),
            t.pos_repeat_req,
            pct(t.pos_repeat_req),
            t.pos_repeat_xreq,
            pct(t.pos_repeat_xreq),
            t.neg_first,
            pct(t.neg_first),
            t.neg_repeat_req,
            pct(t.neg_repeat_req),
            t.neg_repeat_xreq,
            pct(t.neg_repeat_xreq),
            neg,
            pct(neg),
            recoverable,
            pct(recoverable),
            distinct_absent,
            probes as f64 / epochs as f64,
        );
        let mut rows: Vec<_> = m
            .iter()
            .filter(|(_, s)| s.neg_probes > 1)
            .map(|((p, b), s)| (s.neg_probes, *b, p.as_str()))
            .collect();
        rows.sort_by_key(|(c, _, _)| std::cmp::Reverse(*c));
        {
            let g = CREATORS.lock();
            if let Some(m) = g.as_ref() {
                let mut rows: Vec<_> = m.iter().map(|(k, v)| (*v, k.as_str())).collect();
                rows.sort_by_key(|(c, _)| std::cmp::Reverse(*c));
                out.push_str("\n--- builtins that RETIRED the cached negatives (count, name) ---");
                for (c, n) in rows.into_iter().take(20) {
                    out.push_str(&format!("\n{:>12}  {}", c, n));
                }
            }
        }
        out.push_str("\n--- most-re-probed ABSENT paths (probes, kind bit, path) ---");
        for (c, b, p) in rows.into_iter().take(n) {
            out.push_str(&format!("\n{:>12}  {}  {}", c, b, p));
        }
        out
    }
}

/// True when `RUSTCFML_COUNTERS=1` — memoized once per process.
pub fn enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("RUSTCFML_COUNTERS").map(|v| v == "1").unwrap_or(false))
}

/// Per-BIF call census WITH ARGUMENT SHAPES (`bif-census` builds only).
///
/// Part 1 Step 0 of the performance plan: a cross-engine per-BIF benchmark is
/// only honest if it feeds each BIF the arguments the real workload feeds it.
/// Benching `len()` with a 3-char literal when Preside calls it with 4 KB of
/// rendered HTML is how a microbench lies (the rule that cost a build: a
/// microbench said the intercept chain was 44% of a call; live Preside said
/// 6.8%). So this records, per builtin name: the call count, the arity
/// histogram, and — per argument position — a type histogram plus size
/// statistics (string lengths, array/struct element counts).
///
/// ⚠️ The instrument that preceded this one was WRONG because `record_name()`
/// took a Mutex and allocated a String INSIDE a timing window, tripling the
/// phase it was sizing. There is deliberately NO timing here: this census
/// answers "what is called, with what", never "how long did it take". Timing
/// belongs to `call-phases` / the flamegraph, whose windows this must never
/// enter. Call [`record`] outside any measured region.
#[cfg(feature = "bif-census")]
pub mod bif_census {
    use std::collections::HashMap;
    use std::sync::Mutex;

    /// Argument positions tracked in detail. Beyond this, only arity is kept.
    pub const POSITIONS: usize = 4;
    /// Value-shape buckets, indexed by [`kind_of`].
    pub const KINDS: usize = 11;
    pub const KIND_NAMES: [&str; KINDS] = [
        "null", "bool", "int", "double", "string", "array", "struct", "query", "fn",
        "binary", "other",
    ];

    /// Per-argument-position shape accumulator.
    #[derive(Default, Clone)]
    pub struct ArgShape {
        /// How many calls supplied an argument at this position.
        pub seen: u64,
        /// Type histogram, indexed by [`kind_of`].
        pub kinds: [u64; KINDS],
        /// Sum of the "size" measure (string byte length, array/struct element
        /// count) over the sized kinds only.
        pub size_sum: u64,
        /// Largest size seen, so a mean of 12 that hides a 40 KB outlier is
        /// visible rather than averaged away.
        pub size_max: u64,
        /// Calls whose argument had a size measure at all (the divisor for the
        /// mean — an `int` argument contributes to `seen` but not to this).
        pub size_n: u64,
    }

    #[derive(Default, Clone)]
    pub struct BifStat {
        pub calls: u64,
        /// Arity histogram, 0..=7 with 8 = "8 or more".
        pub arity: [u64; 9],
        pub args: [ArgShape; POSITIONS],
    }

    static TABLE: Mutex<Option<HashMap<String, BifStat>>> = Mutex::new(None);

    /// Classify one value into a [`KIND_NAMES`] bucket and, where the notion
    /// applies, its size. Returns `(kind, Some(size))`.
    ///
    /// The size probe takes the container's own short read lock — safe here
    /// because the census runs at a dispatch site that holds no struct/array
    /// guard, but it is the reason this must never be called from inside a
    /// closure that already borrowed the same container
    /// (`bug_parking_lot_iflet_read_guard_deadlock`).
    pub fn classify(v: &crate::dynamic::CfmlValue) -> (usize, Option<u64>) {
        use crate::dynamic::CfmlValue as V;
        match v {
            V::Null => (0, None),
            V::Bool(_) => (1, None),
            V::Int(_) => (2, None),
            V::Double(_) | V::TimeSpan(_) => (3, None),
            V::String(s) => (4, Some(s.len() as u64)),
            V::Array(a) => (5, Some(a.len() as u64)),
            V::QueryColumn(c, _) => (5, Some(c.len() as u64)),
            V::Struct(s) => (6, Some(s.len() as u64)),
            V::Query(q) => (7, Some(q.row_count() as u64)),
            V::Function(_) | V::Closure(_) => (8, None),
            V::Binary(b) => (9, Some(b.len() as u64)),
            _ => (10, None),
        }
    }

    /// Record one builtin invocation. `name` is the resolved registry spelling;
    /// the table folds case so `Len` and `len` land in one row.
    pub fn record(name: &str, args: &[crate::dynamic::CfmlValue]) {
        let mut g = match TABLE.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        let map = g.get_or_insert_with(HashMap::new);
        // Fold case so `Len` and `len` are one row. Two probes rather than one
        // `entry()`: the hit path must not allocate a String per call, and this
        // is a probe build where a second hash is free.
        let lowered;
        let key: &str = if name.bytes().any(|b| b.is_ascii_uppercase()) {
            lowered = name.to_ascii_lowercase();
            &lowered
        } else {
            name
        };
        if !map.contains_key(key) {
            map.insert(key.to_string(), BifStat::default());
        }
        let e = map.get_mut(key).expect("just inserted");
        e.calls += 1;
        e.arity[args.len().min(8)] += 1;
        for (i, a) in args.iter().take(POSITIONS).enumerate() {
            let (kind, size) = classify(a);
            let s = &mut e.args[i];
            s.seen += 1;
            s.kinds[kind] += 1;
            if let Some(n) = size {
                s.size_sum += n;
                s.size_n += 1;
                s.size_max = s.size_max.max(n);
            }
        }
    }

    /// Machine-readable dump of EVERY row's raw accumulators, one line per
    /// builtin, for exact diffing of two cumulative snapshots.
    ///
    /// The human [`report`] cannot be diffed honestly: it truncates to the top
    /// `n` and prints shapes as percentages of the CUMULATIVE totals, which
    /// boot dominates. A warm render's real mix only appears when two dumps are
    /// subtracted — and the boot-vs-warm mixes genuinely differ (`compareNoCase`
    /// is 19% of boot and absent from the warm top 12), so the subtraction is
    /// not a nicety. Consumed by `scripts/perf/bif_census_diff.py`.
    ///
    /// Format (tab-separated, one line per builtin):
    /// `BIFRAW <name> <calls> <arity0..8 csv> <arg1 fields csv> .. <arg4 ..>`
    /// where each arg's fields are `seen,k0..k10,size_sum,size_n,size_max`.
    pub fn report_raw() -> String {
        let g = match TABLE.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        let map = match g.as_ref() {
            Some(m) => m,
            None => return String::new(),
        };
        let mut rows: Vec<(&String, &BifStat)> = map.iter().collect();
        rows.sort_by_key(|(_, s)| std::cmp::Reverse(s.calls));
        let mut out = String::from("--- BIFRAW BEGIN ---");
        for (name, s) in rows {
            out.push_str(&format!("\nBIFRAW\t{}\t{}\t", name, s.calls));
            out.push_str(
                &s.arity.iter().map(|c| c.to_string()).collect::<Vec<_>>().join(","),
            );
            for a in &s.args {
                let mut f = vec![a.seen.to_string()];
                f.extend(a.kinds.iter().map(|c| c.to_string()));
                f.push(a.size_sum.to_string());
                f.push(a.size_n.to_string());
                f.push(a.size_max.to_string());
                out.push('\t');
                out.push_str(&f.join(","));
            }
        }
        out.push_str("\n--- BIFRAW END ---");
        out
    }

    /// Descending report of the top `n` builtins by call count, each followed
    /// by the argument shapes it was really called with — the input to the
    /// per-BIF cross-engine bench.
    pub fn report(n: usize) -> String {
        let g = match TABLE.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        let map = match g.as_ref() {
            Some(m) => m,
            None => return "--- BIF census: no calls recorded ---".to_string(),
        };
        let mut rows: Vec<(&String, &BifStat)> = map.iter().collect();
        let total: u64 = rows.iter().map(|(_, s)| s.calls).sum();
        rows.sort_by_key(|(_, s)| std::cmp::Reverse(s.calls));
        let mut out = format!(
            "--- BIF census: {} calls across {} distinct builtins (top {}) ---",
            total,
            rows.len(),
            n.min(rows.len())
        );
        let mut cum = 0u64;
        for (name, s) in rows.iter().take(n) {
            cum += s.calls;
            let arity: Vec<String> = s
                .arity
                .iter()
                .enumerate()
                .filter(|(_, c)| **c > 0)
                .map(|(a, c)| format!("{}:{}", a, c))
                .collect();
            out.push_str(&format!(
                "\n{:>10} {:>6.2}% {:>6.2}%cum  {:<24} arity[{}]",
                s.calls,
                s.calls as f64 / total.max(1) as f64 * 100.0,
                cum as f64 / total.max(1) as f64 * 100.0,
                name,
                arity.join(" "),
            ));
            for (i, a) in s.args.iter().enumerate() {
                if a.seen == 0 {
                    continue;
                }
                let kinds: Vec<String> = a
                    .kinds
                    .iter()
                    .enumerate()
                    .filter(|(_, c)| **c > 0)
                    .map(|(k, c)| {
                        format!("{}={:.0}%", KIND_NAMES[k], *c as f64 / a.seen as f64 * 100.0)
                    })
                    .collect();
                let size = if a.size_n > 0 {
                    format!(
                        "  size mean {:.1} max {}",
                        a.size_sum as f64 / a.size_n as f64,
                        a.size_max
                    )
                } else {
                    String::new()
                };
                out.push_str(&format!(
                    "\n{:>28}arg{}: {}{}",
                    "",
                    i + 1,
                    kinds.join(" "),
                    size
                ));
            }
        }
        out
    }
}
