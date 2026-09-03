//! CFML Code Generator - AST to bytecode

use cfml_compiler::ast::*;
pub use cfml_common::name::Name;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

/// Per-`loop file=` id, so the synthesised handle/line temporaries are unique
/// across nested and sibling file loops.
static NEXT_FILE_LOOP_ID: AtomicU32 = AtomicU32::new(0);

/// Process-global monotonic counter assigning every compiled `BytecodeFunction`
/// a unique, stable `global_id`. The id is stable for the lifetime of a cached
/// program (the VM's bytecode cache reuses the same `Arc`s), so a stored
/// function reference resolves through the VM's function registry identically on
/// every request and under any program swap — identity never depends on a
/// per-request program-table layout, which is what makes the stale-index bug
/// class (cross-request dispatch and the issue #70 intra-request swap) impossible
/// by construction.
static NEXT_GLOBAL_FN_ID: AtomicU32 = AtomicU32::new(0);

/// Allocate the next process-global function id.
pub fn next_global_fn_id() -> u32 {
    NEXT_GLOBAL_FN_ID.fetch_add(1, Ordering::Relaxed)
}

/// CFML built-in scope names that resolve through the VM's scope chain
/// rather than the locals map. `<name>.foo` for any of these should NOT
/// route through the LoadLocalProperty peephole, because the VM would
/// miss the fallback lookups (globals, __variables, etc.).
fn is_reserved_scope_name(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "variables"
            | "local"
            | "arguments"
            | "this"
            | "super"
            | "request"
            | "application"
            | "server"
            | "session"
            | "cgi"
            | "url"
            | "form"
            | "cookie"
            | "client"
            | "thread"
            | "cfthread"
            | "attributes"
            | "caller"
            | "flash"
            | "thistag"
            | "static"
    )
}

fn int_lit(e: &Expression) -> Option<i64> {
    if let Expression::Literal(lit) = e {
        if let LiteralValue::Int(n) = lit.value {
            return Some(n);
        }
    }
    None
}

/// Helper function to capitalize the first letter of a string
fn capitalize_first(s: &str) -> String {
    let mut chars = s.chars();
    chars.next().map(|c| c.to_uppercase().collect::<String>())
        .unwrap_or_else(String::new)
        + &s[1..]
}

/// A struct-literal key in the nested-build tree: either a static literal
/// segment (from an identifier/quoted/dotted key) or a key that must be
/// evaluated at runtime (a computed `{ "#k#" = v }` key). Computed keys only
/// ever appear at the top level (they can't be a dotted nesting prefix).
enum StructKey {
    Static(String),
    Computed(Expression),
}

/// Node in the ordered tree used to compile a struct literal that contains
/// dotted-path keys (`{ a.b = 1, a.c = 2 }` → `{ a: { b: 1, c: 2 } }`). A leaf
/// carries the value expression; a branch carries ordered child segments.
enum StructKeyNode {
    Leaf(Expression),
    Branch(Vec<(StructKey, StructKeyNode)>),
}

pub struct CfmlCompiler {
    pub program: BytecodeProgram,
    /// Stack of (break_placeholder_indices, continue_placeholder_indices,
    /// is_loop, open_tag_pairs_at_entry) for loops and `switch` blocks.
    /// `is_loop` is true for real loops and false for `switch`: `break` targets
    /// the nearest frame (loop OR switch, C-style), but `continue` must skip
    /// `switch` frames and target the enclosing loop.
    ///
    /// `open_tag_pairs_at_entry` is `tag_pair_stack.len()` when the frame was
    /// entered. A `break`/`continue` must abandon exactly the custom-tag pairs
    /// opened INSIDE the frame and still open at the jump site — the difference
    /// between that depth and the current one.
    /// The 5th element is `finally_stack.len()` when the frame was entered:
    /// a `break`/`continue` runs the finallys opened INSIDE the frame (a
    /// `transaction { }` / `lock { }` / `try/finally` the jump escapes) before
    /// jumping, exactly as a `return` does. Skipping them left a transaction
    /// neither committed nor rolled back (GH #308).
    loop_stack: Vec<(Vec<usize>, Vec<usize>, bool, usize, usize)>,
    /// Custom-tag pairs currently open in the statement stream, innermost last;
    /// each entry is the instruction index of that pair's body. Pushed by a
    /// lowered `__cfcustomtag_start(...)` statement and popped by its matching
    /// `__cfcustomtag_end()`. Saved/restored across nested function bodies so a
    /// UDF declared inside a tag body never sees the enclosing pair.
    tag_pair_stack: Vec<usize>,
    /// Stack of enclosing `finally` bodies (one entry per enclosing
    /// try-with-finally / `lock {}`, innermost last). A `return` must run ALL of
    /// them (innermost first) before the Return op exits the function, since the
    /// runtime Return op does not run finallys; a `rethrow` in a catch runs the
    /// innermost (its own try's) finally before propagating.
    finally_stack: Vec<Vec<Statement>>,
    /// Names of the catch variables for the enclosing catch clauses currently
    /// being compiled (innermost last). GH #244: `rethrow` re-raises the
    /// exception caught by its enclosing catch clause; the runtime
    /// `last_exception` register can have been clobbered by a nested try/catch
    /// in the same catch body, so before emitting `Rethrow` we reset
    /// `last_exception` from this variable (which holds the full cfcatch struct).
    catch_var_stack: Vec<String>,
    /// Nesting depth of function-body compilation. 0 means page-scope; inside any
    /// UDF or CFC method this is > 0. Used to gate the `variables.x` peephole:
    /// at page scope `variables.x` is a read of globals (LoadGlobal semantics),
    /// but inside a function body `variables` refers to the local-scope merge or
    /// a CFC's `__variables` struct — different semantics entirely.
    function_depth: usize,
    /// Nesting depth of code that owns a function `local` scope (GH #351).
    ///
    /// Bumped by declared functions AND by closure / arrow bodies, which
    /// `function_depth` deliberately does not track. `local` is a reserved SCOPE
    /// name only where this is > 0; at page level and in a CFC
    /// pseudo-constructor it is an ordinary variable, exactly as on Lucee.
    local_scope_depth: usize,
    /// Declared `localMode` of the function currently being compiled. Used so
    /// that closures defined inside that function inherit its declared mode
    /// when the closure itself doesn't carry an explicit attribute. `None` =
    /// at page scope or inside a function that didn't declare its mode (the
    /// closure also inherits `None` and falls back to the application
    /// default at runtime).
    current_fn_local_mode: Option<bool>,
    /// Set while compiling the bodies of a component's methods (the `for func
    /// in &component.functions` loop in [`compile_component`]). Stamped onto
    /// the resulting `BytecodeFunction.is_component_method` so the VM's
    /// DefineFunction op skips the builtin-collision guard for methods. Lucee
    /// allows `obj.canonicalize()` etc.
    in_component_method: bool,
    /// True while compiling an assignment that appears in VALUE position — i.e.
    /// the RHS of an enclosing assignment (`a = b = c`), so the assignment must
    /// leave its assigned value on the stack for the outer store to consume. A
    /// statement-level assignment leaves it false: the consuming store ops emit
    /// NO extra `Dup`, keeping the exact bytecode the JIT admission analyzer
    /// accepts (a stray `Dup` in a hot setter disqualified the function).
    need_assign_value: bool,
    /// Source file path this program is being compiled from, stamped onto every
    /// `BytecodeFunction` so app-scope functions carry a stable, serializable
    /// identity. `None` for in-memory/CLI direct compiles.
    source_file: Option<String>,
}

impl BytecodeFunction {
    /// The parameter names as interned [`Key`](cfml_common::key::Key)s, built
    /// on first call and cached for the life of the function. Use this — not
    /// `params` — anywhere a parameter name is used to probe or seed a scope.
    #[inline]
    pub fn param_keys(&self) -> &[cfml_common::key::Key] {
        self.param_keys
            .get_or_init(|| self.params.iter().map(cfml_common::key::Key::new).collect())
    }
}

#[derive(Debug, Clone)]
pub struct BytecodeProgram {
    pub functions: Vec<Arc<BytecodeFunction>>,
}

#[derive(Debug, Clone)]
pub struct BytecodeFunction {
    pub name: String,
    pub params: Vec<String>,
    /// v0.599 — the parameter names as interned map keys, built once on first
    /// use and shared for the life of the function. Parameter binding and the
    /// return-time write-back scan both probe `locals` once per parameter per
    /// call; going through these means neither hashes, and binding inserts by
    /// cloning a key instead of allocating a `String`.
    pub param_keys: std::sync::OnceLock<Vec<cfml_common::key::Key>>,
    /// Whether the body can observe the `arguments` scope (bare load, string
    /// form, include, custom tag) — decides the eager-vs-lazy arguments build.
    /// Computed ONCE per process from the bytecode (the analysis lives in the
    /// VM, which is why this is a lazy cell rather than a `finalize()` field);
    /// previously a per-VM `HashMap<global_id, bool>` re-scanned and re-probed
    /// (SipHash) every request — ~0.9 probes per frame on a warm Preside
    /// render, plus a full instruction re-scan per function per request.
    pub args_needed: std::sync::OnceLock<bool>,
    /// Lever C: whether the eager `arguments` struct provably cannot escape
    /// the frame (⇒ allocated untracked, skipping cycle-GC logging). Same
    /// once-per-process pattern as `args_needed`, for the same reason.
    pub args_never_escapes: std::sync::OnceLock<bool>,
    /// The `__arguments_params` positional-marker array (declared param names
    /// as a CfmlArray), built once per process on first eager call. Previously
    /// a per-VM `HashMap<global_id, CfmlValue>` — one more SipHash probe per
    /// eager call, rebuilt every request. Shared exactly as widely as the
    /// per-request cache shared it within a request: the marker is filtered
    /// from user-visible introspection, so nothing can mutate it.
    pub params_marker: std::sync::OnceLock<cfml_common::dynamic::CfmlValue>,
    /// The `__cfc_body__` variant of this function: a CFC's `__main__` renamed
    /// and flagged as a template frame, which is what the pseudo-constructor
    /// actually executes. Built ONCE per process, same rationale as the
    /// `OnceLock`s above. Previously every single component construction did
    /// `(*cfc_func).clone()` — a full deep copy of the instruction `Vec` (and
    /// every other field) purely to change a name and set one `bool`, then
    /// dropped it again on return. `new Expectation()` in TestBox's `expect()`
    /// paid that per assertion.
    pub cfc_body: std::sync::OnceLock<std::sync::Arc<BytecodeFunction>>,
    /// Which params are required (parallel to `params`; true = required)
    pub required_params: Vec<bool>,
    /// Which params declare a default value (parallel to `params`; true = has
    /// default). An omitted param with no default must stay absent from the
    /// `arguments` scope; one with a default is materialized by the bytecode
    /// preamble, so the VM only needs to pre-seed it as Null.
    pub has_default: Vec<bool>,
    pub instructions: Vec<BytecodeOp>,
    pub source_file: Option<String>,
    /// Process-global, stable identity (see [`next_global_fn_id`]). Stored
    /// function references and `DefineFunction` ops carry this id; the VM
    /// resolves it through a dense per-request function registry, so dispatch
    /// never depends on the volatile per-request `program.functions` layout.
    pub global_id: u32,
    /// Lucee `localMode` for this function. `Some(true)` = modern (unscoped
    /// writes stay in `local`), `Some(false)` = classic (unscoped writes go
    /// to `variables`/`__variables`). `None` = inherit at runtime from the
    /// application default (`this.localMode` in Application.cfc), falling
    /// back to classic if no app default is set.
    pub declared_local_mode: Option<bool>,
    /// Declared parameter types (parallel to `params`; `None` when untyped).
    /// Surfaced in getMetadata()/getComponentMetadata().
    pub param_types: Vec<Option<String>>,
    /// Declared return type (`function string foo()` → `Some("string")`,
    /// `None` when undeclared/`any`). Surfaced as `returnType` in
    /// getMetadata() on a function reference.
    pub return_type: Option<String>,
    /// Javadoc/inline annotations per parameter (parallel to `params`), e.g.
    /// WireBox `@arg.inject coldbox:setting:features`. Surfaced as `param.inject`
    /// etc. in getMetadata()/getComponentMetadata() for DI frameworks.
    pub param_annotations: Vec<Vec<(String, String)>>,
    /// True when this function is a component method (declared inside a CFC
    /// body). Lucee/ACF allow component methods to shadow built-in function
    /// names — `obj.canonicalize()` dispatches to the method, not the BIF —
    /// so the VM's DefineFunction guard against builtin-name collisions must
    /// skip these. Top-level UDFs keep the guard.
    pub is_component_method: bool,
    /// Declared access modifier (`public`/`private`/`package`/`remote`).
    /// Surfaced so for-in over a component instance can yield only PUBLIC
    /// methods (matching Lucee's `this`-scope iteration, which WireBox virtual
    /// inheritance relies on). Defaults to `Public`.
    pub access: cfml_common::dynamic::CfmlAccess,
    /// Function-level doc-comment / inline annotations (`@expectedException`,
    /// `@skip`, `@labels`, `@hint`, ...). These are also emitted onto the owning
    /// component as `__funcmeta_<name>` for getComponentMetadata(), but a bare
    /// function/method *reference* (`getMetadata(o.foo)`) has no path back to
    /// the component scope, so we carry them on the function itself too. Surfaced
    /// as flat top-level keys in getMetadata() on a function reference, matching
    /// Lucee/ACF.
    pub metadata: Vec<(String, String)>,
    /// True for an engine-SYNTHESIZED property accessor (`accessors="true"` /
    /// `property name="x" type="numeric"` → `getX`/`setX`).
    ///
    /// Such a function carries a declared return type for metadata purposes but
    /// must NOT have it enforced (the v0.557.0 type rules): Lucee reports
    /// `numeric` on a generated `getNum()` and still returns `""` from it
    /// happily, and reports `void` on a generated `setX()` that in fact returns
    /// `this` for chaining. Enforcing either would break CFCs that are
    /// perfectly legal on the reference engine.
    pub is_generated_accessor: bool,
    /// True when this function is declared `output="false"`/`"no"`/`"0"`,
    /// meaning its body must produce NO page output (Lucee/ACF `<cfsilent>`
    /// semantics). Derived from `metadata` once at compile finalize
    /// ([`Self::finalize`]) so the VM's per-call prologue doesn't re-scan and
    /// re-lowercase the metadata on every dispatch.
    pub output_suppressed: bool,
    /// True for the synthetic template-shaped frames — `__main__`,
    /// `__cfc_body__`, `__cfc_static_init__` — whose locals ARE the page /
    /// component `variables` scope and must never leak out as a closure
    /// parent-scope writeback. Derived from `name` at compile finalize
    /// ([`Self::finalize`]).
    pub is_template_frame: bool,
    /// Chained-parent eligibility (perf plan 3.2 stage 2), derived from a
    /// one-pass opcode scan at compile finalize ([`Self::finalize`]):
    ///
    /// - `2` (tier A, strict): the body provably never observes the parent
    ///   scope except through bare-name reads/writes — no scope-as-value
    ///   loads, no dynamic names, no closures defined, no includes. The
    ///   frame can skip the eager parent-key copy entirely and fall back
    ///   through a (env, caller, filter) chain on lookup miss.
    /// - `1` (tier B): additionally uses shapes that resolve through a
    ///   handful of chokepoints (`is_variable_defined`, `scope_aware_load`/
    ///   `_store`, the property-op locals probes, `apply_numeric_delta`) —
    ///   chainable once those chokepoints take a chain fallback parameter.
    /// - `0`: ineligible — the body materializes or mutates the scope map
    ///   wholesale (bare `variables`/`static`/`thread`/`attributes` as a
    ///   value, `DefineFunction` closure capture, includes, dynamic
    ///   set/unset, `ArrayAppendLocal`, explicit `variables.x`).
    ///
    /// Template frames are always `0`: their locals ARE the page/component
    /// variables scope (`captured_locals`), so they must keep eager seeding.
    pub chain_tier: u8,
    /// Slot-resolved locals (perf plan T3.1 stage 1): the frame-private
    /// `var`-declared names this function's `*Slot*` ops index into, in slot
    /// order. Empty = no slots assigned (ineligible function, or the pass is
    /// disabled) — the frame allocates no slot vector. Derived at compile
    /// finalize by [`Self::assign_local_slots`]; construction sites leave it
    /// empty.
    pub slot_names: Vec<Name>,
}

impl BytecodeFunction {
    /// Stamp the name/metadata-derived dispatch flags and trim allocation
    /// slack. Every function reaches this exactly once, at registration
    /// ([`BytecodeCompiler::push_function`]) or, for the `__main__` template
    /// body, at the end of [`BytecodeCompiler::compile`] — construction sites
    /// leave the derived fields `false` and rely on this stamp.
    pub fn finalize(&mut self) {
        self.output_suppressed = self.metadata.iter().any(|(k, v)| {
            k.eq_ignore_ascii_case("output") && {
                let v = v.trim();
                v.eq_ignore_ascii_case("false") || v.eq_ignore_ascii_case("no") || v == "0"
            }
        });
        self.is_template_frame = self.name == "__main__"
            || self.name == "__cfc_body__"
            || self.name == "__cfc_static_init__";
        self.chain_tier = if self.is_template_frame {
            0
        } else {
            Self::scan_chain_tier(&self.instructions)
        };
        if !self.is_template_frame && slot_locals_enabled() {
            self.assign_local_slots();
        }
        self.shrink_to_fit();
    }

    /// One-pass opcode scan classifying chained-parent eligibility (see
    /// [`Self::chain_tier`]). Starts at tier A (2) and only ever downgrades.
    fn scan_chain_tier(ops: &[BytecodeOp]) -> u8 {
        let mut tier = 2u8;
        for op in ops {
            match op {
                // Hard disqualifiers: the body materializes/mutates the scope
                // map wholesale, or captures it into a closure env.
                BytecodeOp::Include(_)
                | BytecodeOp::IncludeDynamic
                | BytecodeOp::DefineFunction(_)
                | BytecodeOp::SetDynamicVar
                | BytecodeOp::UnsetPath(_)
                | BytecodeOp::DeleteScopeKey(_)
                | BytecodeOp::ArrayAppendLocal(_)
                | BytecodeOp::LoadVariablesKey(_) => return 0,
                BytecodeOp::LoadLocal(n) | BytecodeOp::TryLoadLocal(n) => {
                    if matches!(
                        n.lower(),
                        "variables" | "static" | "thread" | "attributes" | "caller"
                    ) {
                        return 0;
                    }
                }
                // Tier-B shapes: resolvable through chain-aware chokepoints.
                BytecodeOp::IsDefined(_)
                | BytecodeOp::LoadLocalProperty(..)
                | BytecodeOp::TryLoadLocalProperty(..)
                | BytecodeOp::StoreLocalProperty(..)
                | BytecodeOp::LoadLocalKey(_)
                | BytecodeOp::TryLoadLocalKey(_)
                | BytecodeOp::Increment(_)
                | BytecodeOp::Decrement(_)
                | BytecodeOp::AddLocalConst(..)
                | BytecodeOp::MulLocalConst(..) => tier = tier.min(1),
                BytecodeOp::CallMethod(_, _, wb) | BytecodeOp::CallMethodNamed(_, _, _, wb) => {
                    if wb.is_some() {
                        tier = tier.min(1);
                    }
                }
                _ => {}
            }
        }
        tier
    }

    /// Slot-resolved locals pass (perf plan T3.1 stage 1). Runs once at
    /// finalize for non-template functions. Assigns a `u16` slot to every
    /// `var`-declared name that is provably frame-private and only touched by
    /// ops that have slot twins, then rewrites those ops in place. The
    /// conservative rules (each is a correctness requirement, see
    /// SLOT_LOCALS_PLAN.md):
    ///
    /// * Whole-function disqualifiers — bodies where the frame's locals map
    ///   escapes or is reflected on dynamically: `DefineFunction` (closure env
    ///   captures the whole frame), `Include`/`IncludeDynamic` (function-scoped
    ///   include shares genuine `local` keys by name), `SetDynamicVar`,
    ///   `DeleteScopeKey("local")` (dynamic-key delete against the live local
    ///   scope), and call-position loads of the reflective builtins
    ///   (`evaluate` family) that read/write the calling frame's locals by
    ///   runtime-computed name.
    /// * Per-name exclusions — names referenced by ops we do NOT rewrite in
    ///   stage 1, whose handlers resolve the name against the locals map and
    ///   would silently miss a slotted value: call-position `LoadGlobal`,
    ///   `StoreGlobal`, `IsDefined` (first path segment, and the second when
    ///   the first is `local`), `SetLastExceptionFromLocal`,
    ///   `JumpIfArgPresent`, method write-back paths (`CallMethod`/
    ///   `CallMethodNamed` `wb[0]`, and `wb[1]` when `wb[0]` is `local`), and
    ///   `LoadVariablesKey`. `UnsetPath` is NOT an exclusion — the VM handler
    ///   clears the slot by name before running the generic delete.
    /// * Reserved scope names and declared params are never slotted (GH#312
    ///   scope reservation; params belong to the `arguments` bidi-sync
    ///   machinery, untouched in stage 1).
    fn assign_local_slots(&mut self) {
        use std::collections::{HashMap, HashSet};

        const SCOPE_NAMES: &[&str] = &[
            "local", "arguments", "variables", "this", "super", "request",
            "session", "application", "server", "url", "form", "cgi", "cookie",
            "client", "static", "attributes", "caller", "thread",
        ];
        const REFLECTIVE_BUILTINS: &[&str] = &[
            "evaluate", "precisionevaluate", "getvariable", "setvariable",
            "structget", "iif", "de", "structdelete", "structclear",
        ];

        // Pass 1: collect candidates + exclusions in one scan.
        let mut declared: Vec<Name> = Vec::new(); // first-declaration order
        let mut declared_seen: HashSet<String> = HashSet::new();
        let mut excluded: HashSet<String> = HashSet::new();
        for (i, op) in self.instructions.iter().enumerate() {
            match op {
                // Closure-defining bodies are counted apart from the other
                // wholesale disqualifiers: P2 sizing needs to know how much of
                // the ineligible code is ONLY ineligible because of a closure,
                // and how much of such a body precedes its first closure (all a
                // spill-on-DefineFunction design could recover).
                BytecodeOp::DefineFunction(_) => {
                    self.count_slot_class(SlotClass::DisqClosure, Some(i));
                    return;
                }
                BytecodeOp::Include(_) | BytecodeOp::IncludeDynamic => {
                    self.count_slot_class(SlotClass::DisqOther(DisqReason::Include), Some(i));
                    return;
                }
                // A runtime-path store only ever touches the path's FIRST
                // segment in this frame (`store_runtime_path`: one
                // `scope_aware_load`/`scope_aware_store` on `parts[0]`, then an
                // in-place walk through reference-typed intermediates). So when
                // the path is a COMPILE-TIME LITERAL — which every
                // auto-vivifying nested-write site emits as
                // `String(path); Swap; SetDynamicVar` — the by-name channel is
                // known and a single per-name exclusion covers it; the whole
                // function need not lose its slots. A reserved scope root
                // (`variables.a.b`, `application.x.y`) touches that scope's own
                // container, never a slottable frame name, so it needs no
                // exclusion at all. This is the widest slot-coverage gap
                // measured: 32% of Wheels' and 5% of Preside's non-template op
                // weight sat behind this one disqualifier.
                //
                // `local.…` stays wholesale: `scope_aware_store("local", …)`
                // merges a materialized whole-scope view back into the frame,
                // which is exactly the by-name channel slots cannot survive.
                // A genuinely runtime-computed path (`"#scope#.#prop#" = v`)
                // also stays wholesale — the name isn't knowable here.
                BytecodeOp::SetDynamicVar => {
                    let literal_root = (i >= 2)
                        .then(|| (&self.instructions[i - 2], &self.instructions[i - 1]))
                        .and_then(|(two_back, one_back)| match (two_back, one_back) {
                            (BytecodeOp::String(p), BytecodeOp::Swap) => {
                                Some(p.split('.').next().unwrap_or("").to_lowercase())
                            }
                            _ => None,
                        });
                    match literal_root.filter(|_| slot_dynvar_narrowing_enabled()) {
                        Some(root) if root != "local" && !root.is_empty() => {
                            if !SCOPE_NAMES.contains(&root.as_str()) {
                                excluded.insert(root);
                            }
                        }
                        _ => {
                            self.count_slot_class(SlotClass::DisqOther(DisqReason::DynVar), Some(i));
                            return;
                        }
                    }
                }
                BytecodeOp::DeleteScopeKey(n) if n.lower() == "local" => {
                    self.count_slot_class(SlotClass::DisqOther(DisqReason::DelScope), Some(i));
                    return;
                }
                BytecodeOp::DeclareLocal(n) => {
                    if declared_seen.insert(n.lower().to_string()) {
                        declared.push(n.clone());
                    }
                }
                BytecodeOp::LoadGlobal(n) => {
                    if REFLECTIVE_BUILTINS.contains(&n.lower()) {
                        self.count_slot_class(SlotClass::DisqOther(DisqReason::Reflective), Some(i));
                        return;
                    }
                    excluded.insert(n.lower().to_string());
                }
                BytecodeOp::StoreGlobal(n)
                | BytecodeOp::SetLastExceptionFromLocal(n)
                | BytecodeOp::JumpIfArgPresent(n, _)
                | BytecodeOp::SeedArgumentKey(n)
                | BytecodeOp::LoadVariablesKey(n) => {
                    excluded.insert(n.lower().to_string());
                }
                BytecodeOp::IsDefined(n) => {
                    let mut segs = n.lower().split('.');
                    if let Some(first) = segs.next() {
                        excluded.insert(first.to_string());
                        if first == "local" {
                            if let Some(second) = segs.next() {
                                excluded.insert(second.to_string());
                            }
                        }
                    }
                }
                BytecodeOp::CallMethod(_, _, Some(wb))
                | BytecodeOp::CallMethodNamed(_, _, _, Some(wb)) => {
                    if let Some(first) = wb.first() {
                        let first_lower = first.to_lowercase();
                        if first_lower == "local" {
                            if let Some(second) = wb.get(1) {
                                excluded.insert(second.to_lowercase());
                            }
                        }
                        excluded.insert(first_lower);
                    }
                }
                _ => {}
            }
        }

        // Filter: drop scope names, params, excluded names; cap at u16.
        let mut slot_of: HashMap<String, u16> = HashMap::new();
        let mut slot_names: Vec<Name> = Vec::new();
        for n in declared {
            let lower = n.lower();
            if SCOPE_NAMES.contains(&lower)
                || excluded.contains(lower)
                || self.params.iter().any(|p| p.eq_ignore_ascii_case(lower))
            {
                continue;
            }
            // Hard cap 64, not `u16::MAX`: the VM's per-frame "this slot
            // refused activation" flags are a single `u64` bitmask rather than a
            // second heap-allocated `Vec<bool>` per frame. Bodies with >64
            // `var`s exist but are not hot loops; the surplus names simply keep
            // the by-name path.
            if slot_names.len() >= 64 {
                break;
            }
            slot_of.insert(lower.to_string(), slot_names.len() as u16);
            slot_names.push(n);
        }
        if slot_names.is_empty() {
            self.count_slot_class(
                SlotClass::NoCandidates { any_declared: !declared_seen.is_empty() },
                None,
            );
            return;
        }
        self.count_slot_class(SlotClass::Slotted, None);

        // Pass 2: rewrite ops whose name resolves to a slot.
        let slot = |n: &Name| slot_of.get(n.lower()).copied();
        for op in &mut self.instructions {
            let new = match &*op {
                BytecodeOp::DeclareLocal(n) => {
                    slot(n).map(|i| BytecodeOp::DeclareSlot(i, n.clone()))
                }
                BytecodeOp::LoadLocal(n) => {
                    slot(n).map(|i| BytecodeOp::LoadSlot(i, n.clone()))
                }
                BytecodeOp::TryLoadLocal(n) => {
                    slot(n).map(|i| BytecodeOp::TryLoadSlot(i, n.clone()))
                }
                BytecodeOp::StoreLocal(n) => {
                    slot(n).map(|i| BytecodeOp::StoreSlot(i, n.clone()))
                }
                BytecodeOp::Increment(n) => {
                    slot(n).map(|i| BytecodeOp::IncrementSlot(i, n.clone()))
                }
                BytecodeOp::Decrement(n) => {
                    slot(n).map(|i| BytecodeOp::DecrementSlot(i, n.clone()))
                }
                BytecodeOp::AddLocalConst(n, k) => {
                    slot(n).map(|i| BytecodeOp::AddSlotConst(i, n.clone(), *k))
                }
                BytecodeOp::MulLocalConst(n, k) => {
                    slot(n).map(|i| BytecodeOp::MulSlotConst(i, n.clone(), *k))
                }
                BytecodeOp::JumpIfLocalCmpConstFalse(n, k, c, t) => slot(n)
                    .map(|i| BytecodeOp::JumpIfSlotCmpConstFalse(i, n.clone(), *k, *c, *t)),
                BytecodeOp::ForLoopStep(n, step, c, k, t) => slot(n)
                    .map(|i| BytecodeOp::ForSlotStep(i, n.clone(), *step, *c, *k, *t)),
                BytecodeOp::LoadLocalKey(n) => {
                    slot(n).map(|i| BytecodeOp::LoadSlotKey(i, n.clone()))
                }
                BytecodeOp::TryLoadLocalKey(n) => {
                    slot(n).map(|i| BytecodeOp::TryLoadSlotKey(i, n.clone()))
                }
                BytecodeOp::LoadLocalProperty(n, p) => {
                    slot(n).map(|i| BytecodeOp::LoadSlotProperty(i, n.clone(), p.clone()))
                }
                BytecodeOp::TryLoadLocalProperty(n, p) => {
                    slot(n).map(|i| BytecodeOp::TryLoadSlotProperty(i, n.clone(), p.clone()))
                }
                BytecodeOp::StoreLocalProperty(n, p) => {
                    slot(n).map(|i| BytecodeOp::StoreSlotProperty(i, n.clone(), p.clone()))
                }
                BytecodeOp::ArrayAppendLocal(n) => {
                    slot(n).map(|i| BytecodeOp::ArrayAppendSlot(i, n.clone()))
                }
                _ => None,
            };
            if let Some(new) = new {
                *op = new;
            }
        }
        self.slot_names = slot_names;
    }

    /// Record one function's slot-locals classification into the process
    /// counters (`RUSTCFML_COUNTERS=1`; diagnostics only, no behavior). Skipped
    /// entirely when counters are off, so the common path pays one bool load.
    /// `first_closure_at` is the instruction index of the body's first
    /// `DefineFunction`, used to attribute the recoverable prefix.
    fn count_slot_class(&self, class: SlotClass, first_closure_at: Option<usize>) {
        use cfml_common::perf_counters as pc;
        if !pc::enabled() {
            return;
        }
        let ops = self.instructions.len() as u64;
        match class {
            SlotClass::Slotted => {
                pc::bump(&pc::SLOT_FN_SLOTTED);
                pc::add(&pc::SLOT_OPS_SLOTTED, ops);
            }
            SlotClass::DisqClosure => {
                pc::bump(&pc::SLOT_FN_DISQ_CLOSURE);
                pc::add(&pc::SLOT_OPS_DISQ_CLOSURE, ops);
                pc::add(
                    &pc::SLOT_OPS_CLOSURE_PREFIX,
                    first_closure_at.unwrap_or(0) as u64,
                );
            }
            SlotClass::DisqOther(reason) => {
                pc::bump(&pc::SLOT_FN_DISQ_OTHER);
                pc::add(&pc::SLOT_OPS_DISQ_OTHER, ops);
                match reason {
                    DisqReason::Include => {
                        pc::bump(&pc::SLOT_FN_DISQ_INCLUDE);
                        pc::add(&pc::SLOT_OPS_DISQ_INCLUDE, ops);
                        pc::add(
                            &pc::SLOT_OPS_INCLUDE_PREFIX,
                            first_closure_at.unwrap_or(0) as u64,
                        );
                    }
                    DisqReason::DynVar => {
                        pc::bump(&pc::SLOT_FN_DISQ_DYNVAR);
                        pc::add(&pc::SLOT_OPS_DISQ_DYNVAR, ops);
                    }
                    DisqReason::Reflective => {
                        pc::bump(&pc::SLOT_FN_DISQ_REFLECTIVE);
                        pc::add(&pc::SLOT_OPS_DISQ_REFLECTIVE, ops);
                    }
                    DisqReason::DelScope => pc::bump(&pc::SLOT_FN_DISQ_DELSCOPE),
                }
            }
            SlotClass::NoCandidates { any_declared } => {
                pc::bump(&pc::SLOT_FN_NO_CANDIDATES);
                pc::bump(if any_declared {
                    &pc::SLOT_FN_ALL_EXCLUDED
                } else {
                    &pc::SLOT_FN_NO_DECLARES
                });
            }
        }
        if !matches!(class, SlotClass::Slotted) {
            pc::add(&pc::SLOT_PARAMS_UNSLOTTED_FNS, self.params.len() as u64);
        }
    }

    /// Release the spare capacity every `Vec` here accumulated while being built.
    ///
    /// Bytecode is produced by repeated `push`, so each vector grows by amortized
    /// doubling and ends up holding up to 2x the capacity it needs — and, unlike a
    /// transient buffer, a compiled function is then retained in the bytecode cache
    /// for the life of the process, so that slack is permanent. On a live Preside
    /// install this was measurable: ~126 MiB of the live heap had arrived via
    /// `RawVec::grow_amortized`, most of it under `compile_file_cached`.
    ///
    /// Safe to do unconditionally because a compiled function is immutable once
    /// built: nothing pushes to these vectors after the compiler hands them over.
    pub fn shrink_to_fit(&mut self) {
        self.instructions.shrink_to_fit();
        self.params.shrink_to_fit();
        self.required_params.shrink_to_fit();
        self.has_default.shrink_to_fit();
        self.param_types.shrink_to_fit();
        self.param_annotations.shrink_to_fit();
        for a in &mut self.param_annotations {
            a.shrink_to_fit();
        }
        self.metadata.shrink_to_fit();
        self.slot_names.shrink_to_fit();
    }
}

/// Is a declared parameter type worth emitting a runtime check for? Only the
/// trivially-unconstrained forms are skipped (undeclared and `any`); every
/// other name — including ones Lucee has no cast target for and therefore
/// always rejects — reaches the VM, which owns the actual rules
/// (`cfml-vm/src/type_check.rs`; enforcement added v0.557.0).
pub fn declared_type_is_checkable(param_type: Option<&str>) -> bool {
    match param_type {
        None => false,
        Some(t) => {
            let t = t.trim();
            !t.is_empty() && !t.eq_ignore_ascii_case("any")
        }
    }
}

/// Inspect a function/closure metadata attribute list for `localMode`.
/// Returns `Some(true)` for modern aliases (`modern`/`always`/`true`),
/// `Some(false)` for classic aliases (`classic`/`update`/`false`),
/// `None` if no `localMode` attribute is present, or its value is not a
/// recognised alias. `None` means "inherit at runtime" — the VM resolves it
/// against the application default (`this.localMode` in Application.cfc),
/// falling back to classic. Case-insensitive. The VM extractor in
/// `extract_app_config` uses the same alias set and `None`-on-unknown rule.
pub fn metadata_declared_local_mode(metadata: &[(String, String)]) -> Option<bool> {
    for (k, v) in metadata {
        if k.eq_ignore_ascii_case("localmode") {
            return match v.trim().to_ascii_lowercase().as_str() {
                "modern" | "always" | "true" => Some(true),
                "classic" | "update" | "false" => Some(false),
                _ => None,
            };
        }
    }
    None
}

/// Runtime kill switch for the slot-locals pass (perf plan T3.1 stage 1):
/// `RUSTCFML_SLOT_LOCALS=0|false|off|no` disables it, anything else (including
/// unset) leaves it on. Read once per process — flipping mid-process would
/// only affect not-yet-compiled files anyway (cached bytecode keeps whatever
/// shape it was compiled with; both shapes execute correctly side by side).
fn slot_locals_enabled() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| {
        !matches!(
            std::env::var("RUSTCFML_SLOT_LOCALS")
                .unwrap_or_default()
                .trim()
                .to_ascii_lowercase()
                .as_str(),
            "0" | "false" | "off" | "no"
        )
    })
}

/// Runtime kill switch for the literal-path `SetDynamicVar` narrowing (T3.1
/// P2): `RUSTCFML_SLOT_DYNVAR=0` restores the wholesale disqualification — also
/// the exact A/B arm the win was measured against. Read-once, like
/// [`slot_locals_enabled`].
fn slot_dynvar_narrowing_enabled() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| {
        !matches!(
            std::env::var("RUSTCFML_SLOT_DYNVAR")
                .unwrap_or_default()
                .trim()
                .to_ascii_lowercase()
                .as_str(),
            "0" | "false" | "off" | "no"
        )
    })
}

/// How `assign_local_slots` classified one function (diagnostics only, see
/// [`BytecodeFunction::count_slot_class`]).
#[derive(Debug, Clone, Copy)]
enum SlotClass {
    Slotted,
    DisqClosure,
    /// Wholesale disqualification other than a closure, with the reason so P2
    /// can be sized per reason rather than as one opaque bucket.
    DisqOther(DisqReason),
    NoCandidates { any_declared: bool },
}

/// Which wholesale disqualifier a function tripped (diagnostics only).
#[derive(Debug, Clone, Copy)]
enum DisqReason {
    /// Function-scoped `<cfinclude>`/`include` — the included template shares
    /// genuine `local` keys BY NAME, so slots must at least spill there.
    Include,
    /// `SetDynamicVar` — a runtime-computed assignment target.
    DynVar,
    /// A call-position load of the reflective `evaluate` family.
    Reflective,
    /// `structDelete(local, …)` against the live local scope.
    DelScope,
}

/// Comparison operator tag for fused-compare super-instructions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CmpOp {
    Lt,
    Lte,
    Gt,
    Gte,
    Eq,
    Neq,
}

#[derive(Debug, Clone)]
pub enum BytecodeOp {
    // Literals
    Null,
    True,
    False,
    Integer(i64),
    Double(f64),
    String(String),

    // Variables
    LoadLocal(Name),
    StoreLocal(Name),
    /// Fused arrayAppend(<ident>, value): pops the value off the stack and
    /// appends it to the array held in the named variable, in place. Avoids the
    /// LoadLocal/clone + builtin call + StoreLocal round-trip whose Arc aliasing
    /// makes a loop of appends O(n²) (every `make_mut` deep-clones the backing
    /// Vec). Emitted only for a 2-arg call with a simple, non-scope identifier.
    ArrayAppendLocal(Name),
    LoadGlobal(Name),
    /// Page-scope `variables.foo` read peephole. Same locals-then-globals
    /// resolution chain as LoadGlobal, but READ position: a plain data value
    /// is always returned as-is. LoadGlobal is otherwise emitted only in
    /// call position, where data inherited from an ancestor frame (and data
    /// under a builtin name) must stay invisible to function-name
    /// resolution (PR #97) — semantics that would corrupt reads of
    /// variables named like builtins (`variables.log`, `variables.len`).
    LoadVariablesKey(Name),
    StoreGlobal(Name),

    // Stack
    Pop,
    Dup,
    Swap,

    // Arithmetic
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Pow,
    IntDiv,
    Negate,

    // String
    Concat,

    // Comparison
    Eq,
    Neq,
    /// `===` / `!==` strict (same-type) equality — no cross-type coercion.
    StrictEq,
    StrictNeq,
    Lt,
    Lte,
    Gt,
    Gte,
    Contains,
    DoesNotContain,

    // Logical
    And,
    Or,
    Not,
    Xor,
    Eqv,
    Imp,

    // Control flow
    Jump(usize),
    JumpIfFalse(usize),
    JumpIfTrue(usize),
    /// Loop-condition super-instruction: `if !(locals[name] CMP const) { jump offset }`.
    /// Fuses LoadLocal + Integer + Cmp + JumpIfFalse into one dispatch.
    /// Emitted by compile_for for conditions of the shape `<identifier> <cmp> <int-const>`.
    JumpIfLocalCmpConstFalse(Name, i64, CmpOp, usize),
    /// For-loop step super-instruction: `locals[name] += step; if (locals[name] CMP const) jump target`.
    /// Fuses Increment + LoadLocal + Integer + Cmp + JumpIfFalse-style test into one
    /// dispatch. `step` is +1 (for `i++`) or -1 (for `i--`). The jump fires on the
    /// TRUE arm (back to body); falling through means the loop has finished.
    ForLoopStep(Name, i64, CmpOp, i64, usize),
    Call(usize),
    Return,

    // Collections
    BuildArray(usize),   // Build array from top N stack items
    BuildStruct(usize),  // Build struct from top N key-value pairs
    GetIndex,            // Get array[index] or struct[key]
    SetIndex,            // Set array[index] = value or struct[key] = value
    GetProperty(Name), // Get object.property — THROWS "Variable '<name>' is undefined" on a genuine miss (Lucee/ACF parity)
    /// Null-tolerant twin of GetProperty: a missing struct/component member reads
    /// as Null instead of throwing. Emitted only in contexts that must tolerate a
    /// missing member — the `?:` (Elvis) left operand, `isNull()`, null-safe `?.`,
    /// and nested-write auto-vivification bases (`q["a"]["b"] = v` with `q` unset).
    TryGetProperty(Name),
    /// Push the `super` receiver for the CURRENTLY EXECUTING method, resolved
    /// relative to that method's defining class (not the leaf instance). Reads
    /// `this.__super_map[<defining source>]` keyed by the active `source_file`,
    /// falling back to `this.__super`. Fixes multi-level `super.method()` so an
    /// intermediate class's method reaches ITS parent rather than recursing.
    LoadSuper,
    /// Push the static "holder" (a cached, lazily-built template instance whose
    /// `__variables.__static` is the shared static scope) for a named component.
    /// Used by the `::` operator to reach static members without instantiating.
    LoadStaticHolder(Name),
    /// Pop a static holder (or any component value) and push the named member of
    /// its static scope (`Component::member`). Pushes Null if absent.
    GetStaticProperty(Name),
    /// Fused LoadLocal(name) + GetProperty(member) — reads a struct field from a
    /// named local in one dispatch. Only emitted for non-null-safe accesses
    /// where the receiver is a plain identifier (the common `s.foo` pattern).
    LoadLocalProperty(Name, Name),
    /// Fused LoadLocal(name) + SetProperty(member) — stores a value into a struct
    /// field of a named local in one dispatch. Only emitted for non-null-safe
    /// accesses where the receiver is a plain identifier (the common `s.foo = x` pattern).
    StoreLocalProperty(Name, Name),
    /// Fused LoadLocal("local") + GetProperty(member) for an explicit `local.foo`
    /// read. The generic path materializes the ENTIRE per-call `local` scope view
    /// (cloning every visible key+value into a fresh struct) just to extract one
    /// key — profiling stock Wheels showed `build_local_scope_view` was ~35% of
    /// request allocations. This op reads the single member directly from the
    /// frame's `locals`, applying the same per-call visibility filter
    /// (`build_local_scope_view`): inherited/param keys, `this`/`super`, and
    /// `__`-prefixed bridge keys are invisible; a miss yields Null (matching
    /// GetProperty on the materialized view).
    LoadLocalKey(Name),
    /// Null-tolerant twins of the fused read ops (see TryGetProperty). A missing
    /// receiver variable OR a missing member reads as Null instead of throwing.
    TryLoadLocalProperty(Name, Name),
    TryLoadLocalKey(Name),
    /// `local.X = v` compiled at TEMPLATE level (GH #351).
    ///
    /// Whether the frame owns a function `local` scope cannot be decided at
    /// compile time: a template `include`d from INSIDE a function compiles as
    /// `__main__` yet shares the caller's `local` scope at run time, while a
    /// top-level page and a CFC pseudo-constructor have none at all. So this op
    /// carries the decision to the VM:
    ///
    /// - frame HAS a local scope → identical to `DeclareLocal` + `StoreLocal`,
    ///   the fast frame-key write a function body compiles directly.
    /// - frame has NONE → `local` is an ordinary variable, so auto-vivify it as a
    ///   struct and set the member (Lucee creates `variables.local`).
    ///
    /// Deliberately NOT lowered to the generic `LoadLocal("local")` /
    /// `SetProperty` / `StoreLocal("local")` trio, which was the first attempt:
    /// that round-trips the WHOLE scope view through a struct on every write,
    /// spilling the frame's slots and re-syncing the closure env — the pattern
    /// behind several historic Wheels stale-value bugs, and Wheels has hundreds
    /// of `local.` sites in `.cfm` templates. It cost 75 spec errors there.
    StoreLocalScopeKey(Name),
    SetProperty(Name), // Set object.property = value
    /// Mark a property name as accessor-private on the current frame's `this`
    /// component: its value was written by a generated `setX()` accessor, so
    /// Lucee keeps it in the private `variables` scope and it must be hidden from
    /// `structKeyList`/`structCount`/`structKeyExists`/for-in (but stays readable
    /// via `getX()`/`serializeJSON`). Records into the `ACCESSOR_PRIVATE_MARKER`
    /// set on `this`. No stack effect. The implicit accessor constructor does the
    /// equivalent from Rust (`mark_accessor_private`).
    MarkAccessorPrivate(Name),
    /// Dynamic/quoted-string LHS assignment: `"#scope#.#prop#" = v` or
    /// `"variables.x" = v`. Stack: [pathString, value]. The path is resolved at
    /// runtime and the value stored scope-aware into the current frame (so
    /// `variables.x` lands in a CFC's __variables, not the page scope). Leaves
    /// the assigned value on the stack. Lucee/ACF semantics; WireBox's
    /// MixerUtil.injectPropertyMixin relies on this.
    SetDynamicVar,
    /// Delete a variable / scope path (CFML null-assignment semantics). Assigning
    /// the result of a function that returns null/void (`x = voidFn()`) must NOT
    /// create the target key, and must DELETE a pre-existing one — the assigned
    /// name stays undefined (StructKeyExists / isDefined both false) in every
    /// scope. Emitted by `=` assignments, guarded by `JumpIfNotNull` so it only
    /// fires when the RHS evaluated to Null. The string is the dotted target path
    /// ("rv", "local.rv", "variables.x", "obj.member", "a.b.c"). Pops/pushes
    /// nothing — the guard's `Pop` already cleared the Null. Lucee semantics.
    UnsetPath(String),

    /// Delete a dynamically-named key from a named scope: pops a key value off
    /// the stack and removes `<scope>.<key>` from the real scope container.
    /// Emitted for `StructDelete(<scope>, keyExpr)` (e.g. `StructDelete(request,
    /// "$flag")`) — scopes are snapshotted when passed as a builtin argument, so
    /// the in-place struct mutation that handles `StructDelete(localStruct, k)`
    /// can't reach the live scope; this op deletes straight from it. Pushes
    /// nothing.
    DeleteScopeKey(Name),

    // Object
    NewObject(usize),  // arg_count for constructor
    // arg_count for constructor + call-site argument names (empty string = positional).
    // Used when `new X(...)` supplies named arguments so init() binds by name, not position.
    NewObjectNamed(Vec<String>, usize),

    // Function definition
    DefineFunction(usize), // BytecodeFunction.global_id (resolved via the VM's fn_registry)

    // Postfix ops
    Increment(Name),  // Increment variable (+1)
    Decrement(Name),  // Decrement variable (-1)
    AddLocalConst(Name, i64),  // Add constant to local: i += K or i = i + K
    MulLocalConst(Name, i64),  // Multiply local by constant: i *= K

    // Exception handling
    TryStart(usize),    // Jump target for catch
    TryEnd,
    Throw,
    Rethrow,            // Re-throw current exception
    // Save/restore the engine's "last exception" register onto an internal
    // stack. A `finally` body emitted inline before a `rethrow`/`return` may
    // itself contain a `try {} catch {}` that throws-and-swallows — which would
    // clobber `last_exception` and make the following `rethrow` re-raise the
    // WRONG (inner, already-handled) exception. Wrapping the inline finally in
    // SaveException/RestoreException preserves the exception the enclosing catch
    // actually caught.
    SaveException,
    RestoreException,
    // GH #244: reset the engine's "last exception" register from a local
    // variable (the enclosing catch clause's caught-exception variable). Emitted
    // just before a `rethrow` inside a catch body so it re-raises the exception
    // THAT clause caught, even when a nested try/catch in the same body has since
    // overwritten `last_exception` with an already-handled inner error. A no-op
    // if the named local is undefined.
    SetLastExceptionFromLocal(Name),
    // Peek the exception value on top of the stack (does NOT consume it) and
    // push a boolean: does its `type` match this catch clause's declared type?
    // "any"/empty matches everything; otherwise case-insensitive exact match or
    // dotted-hierarchy prefix (catch "Foo" also catches "Foo.Bar").
    CatchMatch(Name),

    // Method call: object is on stack, then args, method name + arg count
    // Optional write-back: (object_var, Option<property_name>)
    //   - Some(vec!["dog"]) for dog.method() — write modified this back to dog
    //   - Some(vec!["this", "items"]) for this.items.method() — write result back to this.items
    //   - Some(vec!["local", "_taffy", "factory"]) for local._taffy.factory.method()
    //   - None — no write-back needed
    CallMethod(Name, usize, Option<Vec<String>>),
    // Method call with named arguments: like CallMethod but carries the
    // call-site argument names (empty string for positional args), so the VM
    // can rebind them to the resolved method's parameters by name. Mirrors
    // CallNamed for free-function calls. The names are boxed so this variant
    // does not grow BytecodeOp past its size ceiling (it is the rare path).
    CallMethodNamed(Name, Box<Vec<String>>, usize, Option<Vec<String>>),

    // Computed-name method call: `obj[ nameExpr ]( args )`. Stack layout (bottom
    // to top): object, method-name value, then `arg_count` positional args. The
    // VM stringifies the method name at runtime and dispatches via the same
    // member-call path as CallMethod, so the receiver's component scope
    // (`this`/`__variables`/`super`) is bound correctly. Without this, the
    // dynamic call collapsed to indexing out a bare Function and invoking it
    // with the *caller's* scope (Preside DelayedInjector.onMissingMethod ->
    // `instance[ missingMethodName ]( argumentCollection=... )` ran the target's
    // method against the proxy's variables).
    CallComputedMethod(usize),
    // Named-argument variant of CallComputedMethod. Names box matches CallNamed.
    CallComputedMethodNamed(Box<Vec<String>>, usize),

    // For-in support
    GetKeys,  // Pop value: if struct, push array of keys; if array, leave as-is

    // Include
    Include(String),  // Include and execute a file (static path)
    IncludeDynamic,   // Include: pop path from stack (dynamic expression)

    /// Compile-time-bound builtin call: args are already on the stack, the
    /// (already-lowercased) builtin name is baked in. Replaces the
    /// `LoadGlobal(name)` + `Call(n)` pair for pure, non-VM-intercepted
    /// builtins.
    ///
    /// This is the shape Lucee emits: it resolves a BIF's implementing class and
    /// arg types at COMPILE time and emits a typed direct call — "no name
    /// lookup, no Object[], no map probe at runtime"
    /// (`VariableImpl._writeOutFirstBIF`). Ours still probes one lowercase index,
    /// but skips the `LoadGlobal` op, the locals/__variables/globals chain walk,
    /// the per-call `to_lowercase` HEAP ALLOCATION, and the ~825-line intercept
    /// chain in `call_function`.
    ///
    /// Only names that `cfml_common::builtins_meta::is_pure_builtin` accepts are
    /// lowered — registered builtins the VM never intercepts. Named/spread args never lower.
    CallBuiltin(Name, u8),

    // Null handling
    IsNull,                // Pop value, push bool (true if Null)
    JumpIfNotNull(usize),  // Pop value, jump if not null (pushes value back)

    // Default-argument preamble: jump to `target` (skip the default) when the
    // named param WAS supplied by the caller — i.e. the current frame's
    // `arguments` scope already contains that key. Unlike `LoadLocal + IsNull`,
    // this never consults the enclosing scope, so an omitted param whose default
    // expression reads a same-named outer variable (`function f(x = x)`) is not
    // shadowed by its own not-yet-initialized slot (GitHub #240). No stack traffic.
    JumpIfArgPresent(Name, usize),
    /// Seed the frame's own `arguments` scope with an applied default parameter
    /// value (popped from the stack). Replaces the four-op round-trip
    /// `LoadLocal("arguments"); Swap; SetProperty(n); StoreLocal("arguments")`
    /// that the default-parameter preamble used to emit.
    ///
    /// The round-trip was not just slower — that `LoadLocal("arguments")` is
    /// what `function_needs_arguments_scope` keys on, so ONE defaulted parameter
    /// forced every call of the function onto the eager `arguments` path,
    /// opting it out of Lever A's lazy `arguments` whether the default ever
    /// fired or not. With the load gone the function stays lazy, and this op
    /// becomes a no-op on frames that never build an arguments struct (nothing
    /// can observe it there — any reference to `arguments` puts the function
    /// back on the eager path by construction).
    SeedArgumentKey(Name),

    /// Enforce the declared type of param `N` (index into the function's
    /// `params`/`param_types`) against its CURRENT local value — emitted only
    /// inside the default-argument preamble, where the value came from the
    /// declared default rather than from the caller (a caller-supplied argument
    /// is checked by the VM at bind time; enforcement added v0.557.0). No stack
    /// traffic.
    ValidateParamType(usize),

    // Output
    Print,
    Halt,

    // Variable existence check
    IsDefined(Name),

    // Spread operator support
    ConcatArrays,
    MergeStructs,
    CallSpread,

    // Source location tracking
    LineInfo(usize, usize),  // (line, column) — emitted before statements for stack traces

    /// Emitted in place of the trailing `Pop` of a lowered `__cfcustomtag_end()`
    /// statement. Pops the end call's result and, when the end phase asked for
    /// another iteration (`<cfexit method="loop">`), rewinds `ip` to the operand
    /// — the index of the tag body's first instruction.
    ///
    /// The target is resolved LEXICALLY at codegen time, where the
    /// `__cfcustomtag_start(...); <body> __cfcustomtag_end();` triple emitted by
    /// the tag preprocessor is still visibly a pair. It is deliberately not
    /// inferred at runtime from the `Call; Pop` instruction shape: every
    /// statement is already prefixed with a `LineInfo`, and several peepholes in
    /// `compile_statement` reshape that arm, so a layout assumption baked into
    /// the VM dispatch loop would be silently invalidated by unrelated codegen
    /// changes.
    ///
    /// Note that the body is NOT registered on `loop_stack`. It is not a loop:
    /// a `<cfbreak>` written in a custom tag body binds to the CALLER's
    /// enclosing loop (Lucee-verified), and keeping the body as inline bytecode
    /// with a rewind preserves that exactly.
    TagLoopBack(usize),

    /// Abandon the innermost `n` open custom-tag pairs: discard each captured
    /// body buffer (restoring the enclosing one) and drop its `CustomTagState`.
    ///
    /// Emitted before a `break`/`continue` that jumps out of a custom tag body,
    /// where the matching `__cfcustomtag_end()` will never execute. Without it
    /// the buffer `__cfcustomtag_start` pushed is never popped, so every
    /// subsequent write on the page lands in the orphaned buffer and is silently
    /// discarded — the page is truncated at the tag with no error. Lucee
    /// discards the body content, skips the end phase, and carries on.
    AbandonTagPairs(usize),

    // Safe variable load: returns Null for undefined vars (used by Elvis, null-safe, isNull)
    TryLoadLocal(Name),

    // Declare a variable as function-local (var keyword) — prevents writeback to parent scope
    DeclareLocal(Name),

    // ── Slot-resolved locals (perf plan T3.1 stage 1) ────────────────────────
    // Each is the slot twin of the correspondingly-named `*Local*` op, produced
    // by the `assign_local_slots` finalize pass for `var`-declared names in
    // eligible functions. `u16` indexes the frame's slot vector
    // (`BytecodeFunction::slot_names` names each slot); the carried `Name` is
    // BOTH the diagnostic spelling AND the runtime fallback identity: a slot
    // that is `None` (the `var` statement hasn't executed yet on this control
    // path, or the name was `UnsetPath`-deleted) makes the op behave exactly
    // like its named twin, preserving CFML's order-sensitive `var` semantics
    // with no dominance analysis. Semantics notes:
    // * `LoadSlotKey`/`TryLoadSlotKey` are the `local.x` fused reads — their
    //   `None` fallback is the LoadLocalKey path (local-scope-only, Null on
    //   miss), NOT the scope chain.
    // * A declared slot always wins the scope chain (a `var` name shadows
    //   everything except reserved scope names, which are never slotted).
    DeclareSlot(u16, Name),
    LoadSlot(u16, Name),
    TryLoadSlot(u16, Name),
    StoreSlot(u16, Name),
    IncrementSlot(u16, Name),
    DecrementSlot(u16, Name),
    AddSlotConst(u16, Name, i64),
    MulSlotConst(u16, Name, i64),
    JumpIfSlotCmpConstFalse(u16, Name, i64, CmpOp, usize),
    ForSlotStep(u16, Name, i64, CmpOp, i64, usize),
    LoadSlotKey(u16, Name),
    TryLoadSlotKey(u16, Name),
    LoadSlotProperty(u16, Name, Name),
    TryLoadSlotProperty(u16, Name, Name),
    StoreSlotProperty(u16, Name, Name),
    ArrayAppendSlot(u16, Name),

    // Named function call: like Call but carries argument names for name-to-param mapping
    // (names, arg_count) — names[i] corresponds to the i-th arg on the stack
    CallNamed(Vec<String>, usize),

    // Explicit super(args) constructor call for a CFC whose parent is a Rust class.
    // Pops arg_count values, looks up the constructor registered under
    // this.__rust_extends, calls it, and stores the new NativeObject on
    // this.__super (replacing any default-constructed one). Pushes Null.
    CallRustSuperCtor(usize),
}

impl BytecodeOp {
    /// Dense opcode index for the dynamic op census (`op-census` builds).
    /// Generated from the variant order of this enum; kept in lockstep with
    /// [`Self::CENSUS_NAMES`].
    #[inline]
    pub fn census_index(&self) -> usize {
        match self {
            Self::Null => 0,
            Self::True => 1,
            Self::False => 2,
            Self::Integer(..) => 3,
            Self::Double(..) => 4,
            Self::String(..) => 5,
            Self::LoadLocal(..) => 6,
            Self::StoreLocal(..) => 7,
            Self::ArrayAppendLocal(..) => 8,
            Self::LoadGlobal(..) => 9,
            Self::LoadVariablesKey(..) => 10,
            Self::StoreGlobal(..) => 11,
            Self::Pop => 12,
            Self::Dup => 13,
            Self::Swap => 14,
            Self::Add => 15,
            Self::Sub => 16,
            Self::Mul => 17,
            Self::Div => 18,
            Self::Mod => 19,
            Self::Pow => 20,
            Self::IntDiv => 21,
            Self::Negate => 22,
            Self::Concat => 23,
            Self::Eq => 24,
            Self::Neq => 25,
            Self::StrictEq => 26,
            Self::StrictNeq => 27,
            Self::Lt => 28,
            Self::Lte => 29,
            Self::Gt => 30,
            Self::Gte => 31,
            Self::Contains => 32,
            Self::DoesNotContain => 33,
            Self::And => 34,
            Self::Or => 35,
            Self::Not => 36,
            Self::Xor => 37,
            Self::Eqv => 38,
            Self::Imp => 39,
            Self::Jump(..) => 40,
            Self::JumpIfFalse(..) => 41,
            Self::JumpIfTrue(..) => 42,
            Self::JumpIfLocalCmpConstFalse(..) => 43,
            Self::ForLoopStep(..) => 44,
            Self::Call(..) => 45,
            Self::Return => 46,
            Self::BuildArray(..) => 47,
            Self::BuildStruct(..) => 48,
            Self::GetIndex => 49,
            Self::SetIndex => 50,
            Self::GetProperty(..) => 51,
            Self::TryGetProperty(..) => 52,
            Self::LoadSuper => 53,
            Self::LoadStaticHolder(..) => 54,
            Self::GetStaticProperty(..) => 55,
            Self::LoadLocalProperty(..) => 56,
            Self::StoreLocalProperty(..) => 57,
            Self::LoadLocalKey(..) => 58,
            Self::TryLoadLocalProperty(..) => 59,
            Self::TryLoadLocalKey(..) => 60,
            Self::SetProperty(..) => 61,
            Self::MarkAccessorPrivate(..) => 62,
            Self::SetDynamicVar => 63,
            Self::UnsetPath(..) => 64,
            Self::DeleteScopeKey(..) => 65,
            Self::NewObject(..) => 66,
            Self::NewObjectNamed(..) => 67,
            Self::DefineFunction(..) => 68,
            Self::Increment(..) => 69,
            Self::Decrement(..) => 70,
            Self::AddLocalConst(..) => 71,
            Self::MulLocalConst(..) => 72,
            Self::TryStart(..) => 73,
            Self::TryEnd => 74,
            Self::Throw => 75,
            Self::Rethrow => 76,
            Self::SaveException => 77,
            Self::RestoreException => 78,
            Self::SetLastExceptionFromLocal(..) => 79,
            Self::CatchMatch(..) => 80,
            Self::CallMethod(..) => 81,
            Self::CallMethodNamed(..) => 82,
            Self::CallComputedMethod(..) => 83,
            Self::CallComputedMethodNamed(..) => 84,
            Self::GetKeys => 85,
            Self::Include(..) => 86,
            Self::IncludeDynamic => 87,
            Self::IsNull => 88,
            Self::JumpIfNotNull(..) => 89,
            Self::JumpIfArgPresent(..) => 90,
            Self::ValidateParamType(..) => 91,
            Self::Print => 92,
            Self::Halt => 93,
            Self::IsDefined(..) => 94,
            Self::ConcatArrays => 95,
            Self::MergeStructs => 96,
            Self::CallSpread => 97,
            Self::LineInfo(..) => 98,
            Self::TagLoopBack(..) => 99,
            Self::AbandonTagPairs(..) => 100,
            Self::TryLoadLocal(..) => 101,
            Self::DeclareLocal(..) => 102,
            Self::DeclareSlot(..) => 103,
            Self::LoadSlot(..) => 104,
            Self::TryLoadSlot(..) => 105,
            Self::StoreSlot(..) => 106,
            Self::IncrementSlot(..) => 107,
            Self::DecrementSlot(..) => 108,
            Self::AddSlotConst(..) => 109,
            Self::MulSlotConst(..) => 110,
            Self::JumpIfSlotCmpConstFalse(..) => 111,
            Self::ForSlotStep(..) => 112,
            Self::LoadSlotKey(..) => 113,
            Self::TryLoadSlotKey(..) => 114,
            Self::LoadSlotProperty(..) => 115,
            Self::TryLoadSlotProperty(..) => 116,
            Self::StoreSlotProperty(..) => 117,
            Self::ArrayAppendSlot(..) => 118,
            Self::CallNamed(..) => 119,
            Self::CallRustSuperCtor(..) => 120,
            Self::CallBuiltin(..) => 121,
            Self::SeedArgumentKey(..) => 122,
            Self::StoreLocalScopeKey(..) => 123,
        }
    }

    /// Variant names, indexed by [`Self::census_index`].
    pub const CENSUS_NAMES: [&'static str; 124] = [
        "Null",
        "True",
        "False",
        "Integer",
        "Double",
        "String",
        "LoadLocal",
        "StoreLocal",
        "ArrayAppendLocal",
        "LoadGlobal",
        "LoadVariablesKey",
        "StoreGlobal",
        "Pop",
        "Dup",
        "Swap",
        "Add",
        "Sub",
        "Mul",
        "Div",
        "Mod",
        "Pow",
        "IntDiv",
        "Negate",
        "Concat",
        "Eq",
        "Neq",
        "StrictEq",
        "StrictNeq",
        "Lt",
        "Lte",
        "Gt",
        "Gte",
        "Contains",
        "DoesNotContain",
        "And",
        "Or",
        "Not",
        "Xor",
        "Eqv",
        "Imp",
        "Jump",
        "JumpIfFalse",
        "JumpIfTrue",
        "JumpIfLocalCmpConstFalse",
        "ForLoopStep",
        "Call",
        "Return",
        "BuildArray",
        "BuildStruct",
        "GetIndex",
        "SetIndex",
        "GetProperty",
        "TryGetProperty",
        "LoadSuper",
        "LoadStaticHolder",
        "GetStaticProperty",
        "LoadLocalProperty",
        "StoreLocalProperty",
        "LoadLocalKey",
        "TryLoadLocalProperty",
        "TryLoadLocalKey",
        "SetProperty",
        "MarkAccessorPrivate",
        "SetDynamicVar",
        "UnsetPath",
        "DeleteScopeKey",
        "NewObject",
        "NewObjectNamed",
        "DefineFunction",
        "Increment",
        "Decrement",
        "AddLocalConst",
        "MulLocalConst",
        "TryStart",
        "TryEnd",
        "Throw",
        "Rethrow",
        "SaveException",
        "RestoreException",
        "SetLastExceptionFromLocal",
        "CatchMatch",
        "CallMethod",
        "CallMethodNamed",
        "CallComputedMethod",
        "CallComputedMethodNamed",
        "GetKeys",
        "Include",
        "IncludeDynamic",
        "IsNull",
        "JumpIfNotNull",
        "JumpIfArgPresent",
        "ValidateParamType",
        "Print",
        "Halt",
        "IsDefined",
        "ConcatArrays",
        "MergeStructs",
        "CallSpread",
        "LineInfo",
        "TagLoopBack",
        "AbandonTagPairs",
        "TryLoadLocal",
        "DeclareLocal",
        "DeclareSlot",
        "LoadSlot",
        "TryLoadSlot",
        "StoreSlot",
        "IncrementSlot",
        "DecrementSlot",
        "AddSlotConst",
        "MulSlotConst",
        "JumpIfSlotCmpConstFalse",
        "ForSlotStep",
        "LoadSlotKey",
        "TryLoadSlotKey",
        "LoadSlotProperty",
        "TryLoadSlotProperty",
        "StoreSlotProperty",
        "ArrayAppendSlot",
        "CallNamed",
        "CallRustSuperCtor",
        "CallBuiltin",
        "SeedArgumentKey",
        "StoreLocalScopeKey",
    ];
}

/// True if `name` is a registered builtin the VM does not intercept, i.e. safe to bind at
/// compile time via [`BytecodeOp::CallBuiltin`].
///
/// This replaced a hand-curated 19-name allowlist. The curated list could only ever cover
/// names someone had personally verified against `call_function`'s 7,496-line intercept
/// chain — and that chain contains traps: `arrayFindNoCase` looks exactly like a pure list
/// helper and IS intercepted. Deriving the answer from
/// [`cfml_common::builtins_meta`] covers all ~541 non-intercepted builtins instead of 19,
/// and both of its lists are guarded by tests against the real registration table and the
/// real chain, so they cannot silently rot.
#[inline]
fn is_direct_builtin(name: &str) -> bool {
    // `name` arrives in source casing; the declared lists are lowercase.
    let lower = name.to_ascii_lowercase();
    // An extension's BIF is bound at compile time on the same terms as a
    // compiled-in one. It cannot be VM-intercepted by construction (the
    // intercept chain knows nothing about it), and extensions load before
    // anything is compiled and are never unloaded — so the answer cannot change
    // under a compiled template. Worth ~195 ns per call: the ABI crossing is a
    // small part of an extension call, and the generic dispatch path is most
    // of it.
    cfml_common::builtins_meta::is_pure_builtin(&lower)
        || cfml_common::builtins_meta::is_foreign_builtin(&lower)
}


impl CfmlCompiler {
    /// GH #351 — is `local` a SCOPE in the code currently being compiled?
    ///
    /// Only inside a function body. At page level, and in a CFC
    /// pseudo-constructor, Lucee has no `local` scope at all: `local` is an
    /// ordinary variable name, so `local.foo = 1` creates a `variables.local`
    /// struct and reading `local` before that throws "variable [local] doesn't
    /// exist". The `local.X` fast paths below all compile the member to a
    /// *frame* key (`StoreLocal`/`LoadLocalKey`), which at page level wrote
    /// straight into page `variables` — code written that way worked here and
    /// broke on the reference engine.
    ///
    /// When this is false the callers fall through to the generic
    /// `LoadLocal("local")` path, which the VM resolves against the frame's
    /// real local-scope status (`current_frame_has_local_scope`). That runtime
    /// check is what keeps a template `include`d from INSIDE a function working:
    /// it compiles as `__main__` (depth 0) but does own a shared `local` scope
    /// at run time.
    fn local_is_scope(&self) -> bool {
        self.local_scope_depth > 0
    }
}

impl CfmlCompiler {
    pub fn new() -> Self {
        Self {
            program: BytecodeProgram {
                functions: vec![Arc::new(BytecodeFunction {
                    name: "__main__".to_string(),
                    params: Vec::new(),
                    param_keys: Default::default(),
                    args_needed: Default::default(),
                    args_never_escapes: Default::default(),
                    params_marker: Default::default(),
                    cfc_body: Default::default(),
                    required_params: Vec::new(),
                    has_default: Vec::new(),
                    instructions: Vec::new(),
                    source_file: None,
                    global_id: next_global_fn_id(),
                    declared_local_mode: None,
                    param_types: Vec::new(),
                    return_type: None,
                    param_annotations: Vec::new(),
                    is_component_method: false,
                    access: cfml_common::dynamic::CfmlAccess::Public,
                    metadata: Vec::new(),
                    is_generated_accessor: false,
                    output_suppressed: false,
                    is_template_frame: false,
            chain_tier: 0,
            slot_names: Vec::new(),
                })],
            },
            loop_stack: Vec::new(),
            tag_pair_stack: Vec::new(),
            finally_stack: Vec::new(),
            catch_var_stack: Vec::new(),
            function_depth: 0,
            local_scope_depth: 0,
            current_fn_local_mode: None,
            in_component_method: false,
            need_assign_value: false,
            source_file: None,
        }
    }

    /// Builder: stamp the source file path onto this program's functions so
    /// they carry a stable `(source_file, name, ordinal)` identity. Used by
    /// `compile_file_cached`; the CLI direct-compile path leaves it `None`.
    pub fn with_source_file(mut self, source_file: Option<String>) -> Self {
        self.source_file = source_file;
        self
    }

    /// Flatten a member-access chain like a.b.c into "a.b.c" for dotted new expressions.
    fn flatten_member_access(expr: &Expression) -> Option<String> {
        match expr {
            Expression::Identifier(ident) => Some(ident.name.clone()),
            Expression::MemberAccess(access) => {
                if let Some(base) = Self::flatten_member_access(&access.object) {
                    Some(format!("{}.{}", base, access.member))
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    /// Determine write-back target for a method call from the AST.
    /// Returns Some((var_name, Some(prop_name))) for obj.prop.method()
    /// or Some((var_name, None)) for var.method()
    fn method_call_write_back(object: &Expression) -> Option<Vec<String>> {
        // Recursively collect the member access chain: a.b.c.method()
        // returns vec!["a", "b", "c"]
        fn collect_path(expr: &Expression, path: &mut Vec<String>) -> bool {
            match expr {
                Expression::Identifier(ident) => {
                    path.push(ident.name.clone());
                    true
                }
                Expression::This(_) => {
                    path.push("this".to_string());
                    true
                }
                Expression::Super(_) => {
                    path.push("this".to_string());
                    true
                }
                Expression::MemberAccess(access) => {
                    if collect_path(&access.object, path) {
                        path.push(access.member.clone());
                        true
                    } else {
                        false
                    }
                }
                Expression::MethodCall(call) => {
                    // For chained calls like a.b().c(), extract the root path
                    // so all calls in the chain write back to the same variable.
                    // BUT: if the inner method returns a new value distinct from
                    // its receiver (filter/map/slice/etc.), the outer call is
                    // operating on that new value, not on `a` — so propagating
                    // the path would cause the outer call's result to clobber
                    // `a`. Break the chain for known transformative methods.
                    let inner_lower = call.method.to_lowercase();
                    let is_transformative = matches!(
                        inner_lower.as_str(),
                        "filter" | "map" | "slice" | "reduce" | "tolist"
                        | "toarray" | "tojson" | "serializejson" | "merge"
                        | "splice" | "indexexists" | "keyarray" | "keylist"
                        | "valuearray" | "copy"
                        // Element/key lookups return a looked-up value (struct
                        // entry, array index), NOT the receiver — so the outer
                        // call in `states.find( k ).process()` operates on that
                        // element and must NOT inherit the receiver's write-back
                        // path. Without this break, a chained non-mutating method
                        // (e.g. `.process()`) propagates its `this` snapshot back
                        // onto `states.find`'s receiver, clobbering it (ColdBox
                        // InterceptorService.processState: `interceptionStates`
                        // got replaced by an InterceptorState on the 2nd call).
                        | "find" | "findnocase"
                    );
                    if is_transformative {
                        return false;
                    }
                    collect_path(&call.object, path)
                }
                _ => false,
            }
        }

        let mut path = Vec::new();
        if collect_path(object, &mut path) {
            Some(path)
        } else {
            None
        }
    }

    /// Register a finished function on the program, trimming its allocation
    /// slack first. Every site that adds a compiled function goes through here
    /// so a new one can't silently reintroduce the untrimmed path.
    fn push_function(&mut self, mut func: BytecodeFunction) -> usize {
        func.finalize();
        self.program.functions.push(Arc::new(func));
        self.program.functions.len() - 1
    }

    pub fn compile(mut self, ast: Program) -> BytecodeProgram {
        let mut instructions = Vec::new();

        // Hoist top-level function declarations: standard CFML behaviour is
        // that <cffunction> / `function f(){}` declarations at template scope
        // are callable from anywhere in the template, regardless of textual
        // order. Compile each top-level FunctionDecl up front so its name is
        // bound before the body runs, then re-emit DefineFunction+StoreLocal
        // at the original textual position. The re-emit re-runs the
        // closure-env sync (DefineFunction folds current locals into the
        // shared closure env), preserving the snapshot semantics that
        // existing scope-capture tests rely on.
        let mut hoisted_indices: Vec<usize> = Vec::new();
        for node in &ast.statements {
            if let CfmlNode::Statement(Statement::FunctionDecl(fd)) = node {
                self.compile_function_decl(&fd.func, &mut instructions);
                // compile_function_decl ends with DefineFunction(idx) +
                // StoreLocal(name); the function's idx is the one in that
                // penultimate op (it is NOT len()-1 of program.functions
                // because nested anon-fn decls inside the body push their
                // own entries first).
                let idx = match instructions.get(instructions.len().saturating_sub(2)) {
                    Some(BytecodeOp::DefineFunction(i)) => *i,
                    _ => panic!("compile_function_decl did not end with DefineFunction"),
                };
                hoisted_indices.push(idx);
            }
        }
        let mut hoist_iter = hoisted_indices.into_iter();
        for node in &ast.statements {
            if let CfmlNode::Statement(Statement::FunctionDecl(fd)) = node {
                let idx = hoist_iter.next().expect("hoist index");
                instructions.push(BytecodeOp::DefineFunction(idx));
                instructions.push(BytecodeOp::StoreLocal(Name::from(&fd.func.name)));
            } else {
                self.compile_node(node, &mut instructions);
            }
        }

        instructions.push(BytecodeOp::Halt);

        // functions[0] is the template body itself; its instruction vector is
        // assigned here rather than via `push_function`, so trim it explicitly.
        instructions.shrink_to_fit();
        let main = Arc::get_mut(&mut self.program.functions[0]).unwrap();
        main.instructions = instructions;
        // A component's PSEUDO-CONSTRUCTOR is this `__main__` body (the VM clones
        // it as `__cfc_body__`), so `<cfcomponent output="false">` has to reach it
        // the same way `<cffunction output="false">` reaches a method: as an
        // `output` entry in the frame's metadata, which `finalize()` turns into
        // `output_suppressed`. Without this the component attribute was parsed,
        // stored in `__metadata`, and then ignored at execution — every
        // instantiation of a TAG-BASED CFC emitted its own inter-tag whitespace
        // into the response. Lucee emits nothing for `output="false"` and leaks
        // the whitespace for `output="true"`/no attribute (verified against
        // 7.1.0+204), which is exactly what this reproduces. Component metadata
        // keys are lower-cased by the tag preprocessor and the script parser, but
        // compare loosely anyway — the same shape a method's `finalize()` accepts.
        if let Some(CfmlNode::Statement(Statement::ComponentDecl(cd))) = ast
            .statements
            .iter()
            .find(|n| matches!(n, CfmlNode::Statement(Statement::ComponentDecl(_))))
        {
            if let Some((k, v)) = cd
                .component
                .metadata
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case("output"))
            {
                main.metadata.push((k.clone(), v.clone()));
            }
        }
        main.finalize();
        self.program.functions.shrink_to_fit();

        self.program
    }

    fn compile_node(&mut self, node: &CfmlNode, instructions: &mut Vec<BytecodeOp>) {
        match node {
            CfmlNode::Statement(stmt) => self.compile_statement(stmt, instructions),
            CfmlNode::Expression(expr) => {
                self.compile_expression(expr, instructions);
                instructions.push(BytecodeOp::Pop);
            }
            _ => {}
        }
    }

    /// Check if an expression is a call to a known mutating function with a simple
    /// variable as the first argument. Returns the variable name for write-back.
    /// e.g. structAppend(myStruct, other) → Some("myStruct")
    /// Emit an `AbandonTagPairs` for every custom-tag pair opened since
    /// `entry_depth` and still open here. No-op when none are (the overwhelmingly
    /// common case), so ordinary `break`/`continue` codegen is unchanged.
    fn emit_abandon_tag_pairs(&self, entry_depth: usize, instructions: &mut Vec<BytecodeOp>) {
        let open = self.tag_pair_stack.len().saturating_sub(entry_depth);
        if open > 0 {
            instructions.push(BytecodeOp::AbandonTagPairs(open));
        }
    }

    /// `Some(true)` for a lowered `__cfcustomtag_start(...)` statement,
    /// `Some(false)` for its matching `__cfcustomtag_end()`, `None` otherwise.
    /// These names are emitted by the tag preprocessor for a custom tag written
    /// with a body; they are not spellable as ordinary CFML identifiers.
    fn custom_tag_pair_call(expr: &Expression) -> Option<bool> {
        if let Expression::FunctionCall(call) = expr {
            if let Expression::Identifier(ident) = &*call.name {
                if ident.name.eq_ignore_ascii_case("__cfcustomtag_start") {
                    return Some(true);
                }
                if ident.name.eq_ignore_ascii_case("__cfcustomtag_end") {
                    return Some(false);
                }
            }
        }
        None
    }

    fn is_mutating_standalone_call(expr: &Expression) -> bool {
        if let Expression::FunctionCall(call) = expr {
            if let Expression::Identifier(ident) = &*call.name {
                let name_lower = ident.name.to_lowercase();
                // NB: structDelete is intentionally absent — it mutates the
                // shared struct handle in place AND returns a BOOLEAN (Lucee/ACF
                // semantics), so storing its return value back over the first
                // arg would clobber the struct variable with `true`/`false`.
                // querySort is absent for exactly the same reason (GH #345): it
                // sorts the shared query handle in place and returns a boolean.
                return matches!(name_lower.as_str(),
                    "structappend" | "structinsert" | "structupdate" |
                    "structclear" | "arrayclear" | "arrayappend" | "arrayprepend" |
                    "arrayinsert" | "arrayinsertat" | "arraydeleteat" | "arraysort" |
                    "arrayresize" | "arrayswap" | "arrayreverse" | "arrayset" |
                    "queryaddcolumn" |
                    "querydeleterow" | "querydeletecolumn"
                ) && !call.arguments.is_empty();
            }
        }
        false
    }

    /// When `expr` is `StructDelete(<reservedScope>, keyExpr [, …])`, returns the
    /// scope name and the key expression. Used to delete straight from the live
    /// scope (scopes don't share their backing when passed as a builtin arg).
    fn structdelete_scope_target(expr: &Expression) -> Option<(String, &Expression)> {
        if let Expression::FunctionCall(call) = expr {
            if let Expression::Identifier(ident) = &*call.name {
                if ident.name.eq_ignore_ascii_case("structdelete") && call.arguments.len() >= 2 {
                    if let Expression::Identifier(scope) = &call.arguments[0] {
                        if Self::is_reserved_scope_name(&scope.name)
                            && !matches!(&call.arguments[1], Expression::NamedArgument(_))
                        {
                            return Some((scope.name.to_lowercase(), &call.arguments[1]));
                        }
                    }
                }
            }
        }
        None
    }

    /// True when `expr` is exactly `arrayAppend(<ident>, value)` — a two-arg
    /// append whose first argument is the given simple identifier and which is
    /// not a reserved scope name. These compile to the fused `ArrayAppendLocal`
    /// op for an O(1) in-place append. The merge form (`arrayAppend(a, b, true)`)
    /// and member-access targets keep the generic clone+store path.
    fn is_inplace_array_append(expr: &Expression, ident: &Identifier) -> bool {
        if let Expression::FunctionCall(call) = expr {
            if let Expression::Identifier(name) = &*call.name {
                if name.name.eq_ignore_ascii_case("arrayappend")
                    && call.arguments.len() == 2
                    && !call.arguments.iter().any(|a| matches!(a, Expression::NamedArgument(_)))
                    && !Self::is_reserved_scope_name(&ident.name)
                {
                    return true;
                }
            }
        }
        false
    }

    /// Extract a static component name from the left side of a `::` operator.
    /// Handles a bare identifier (`A`) and a dotted identifier chain (`pkg.A`,
    /// parsed as nested MemberAccess). Returns None for anything else (the
    /// caller then evaluates the expression and uses its value as the holder).
    fn static_class_name(expr: &Expression) -> Option<String> {
        match expr {
            Expression::Identifier(id) => Some(id.name.clone()),
            Expression::MemberAccess(ma) if !ma.null_safe => {
                Self::static_class_name(&ma.object).map(|base| format!("{}.{}", base, ma.member))
            }
            _ => None,
        }
    }

    /// Scope keywords that must never be treated as plain mutable variables.
    ///
    /// ⚠️ This is the NARROWER of two lists with the same name. The free
    /// [`is_reserved_scope_name`] at the top of this file additionally covers
    /// `attributes`, `caller`, `cfthread`, `flash` and `thistag`. They are NOT
    /// interchangeable: routing an `attributes.x` read through this one lowers it
    /// to `TryLoadLocalProperty("attributes", "x")`, which reads a *local named
    /// `attributes`* — inside a cfthread body nested in a custom tag that silently
    /// picks up the custom tag's attributes instead of the thread's. Check which
    /// one a call site uses before touching it.
    fn is_reserved_scope_name(name: &str) -> bool {
        matches!(name.to_lowercase().as_str(),
            "local" | "variables" | "arguments" | "this" | "super" | "request" |
            "application" | "session" | "server" | "cgi" | "url" | "form" |
            "cookie" | "client" | "thread" | "static"
        )
    }

    /// Flatten a struct-literal KEY expression into its dotted path segments.
    /// A bare identifier (`id`) or a quoted string literal (`"a.b"`) is a single
    /// literal segment; a plain (non null-safe) member-access chain of
    /// identifiers (`obj_a.meta`) becomes the multi-segment path
    /// `["obj_a", "meta"]`. Returns `None` for anything that must be evaluated at
    /// runtime (bracketed/parenthesized/computed keys, calls, indices) — those
    /// keep the existing flat `BuildStruct` path.
    fn flatten_struct_key_path(expr: &Expression) -> Option<Vec<String>> {
        match expr {
            Expression::Identifier(id) => Some(vec![id.name.clone()]),
            Expression::MemberAccess(access) if !access.null_safe => {
                let mut base = Self::flatten_struct_key_path(&access.object)?;
                base.push(access.member.clone());
                Some(base)
            }
            Expression::Literal(Literal { value: LiteralValue::String(s), .. }) => {
                // Quoted keys are LITERAL single keys — `{ "a.b" = 1 }` makes a
                // key named "a.b", it does NOT nest. So never split on dots here.
                Some(vec![s.clone()])
            }
            _ => None,
        }
    }

    /// Insert a dotted-path value into the ordered struct-literal tree, creating
    /// branch nodes for intermediate segments and merging into existing branches
    /// that share a prefix (so `{ a.b = 1, a.c = 2 }` collapses to a single
    /// `a` branch holding both). Key matching is case-insensitive (CFML struct
    /// keys), first-occurrence original case wins. A later leaf at an existing
    /// path overwrites; a deeper path under an existing leaf promotes it to a
    /// branch (last write wins, matching Lucee's left-to-right vivification).
    fn insert_struct_path(
        children: &mut Vec<(StructKey, StructKeyNode)>,
        segs: &[String],
        value: Expression,
    ) {
        let (head, rest) = segs.split_first().expect("non-empty path");
        let existing = children.iter().position(|(k, _)| {
            matches!(k, StructKey::Static(s) if s.eq_ignore_ascii_case(head))
        });
        if rest.is_empty() {
            match existing {
                Some(pos) => children[pos].1 = StructKeyNode::Leaf(value),
                None => children.push((StructKey::Static(head.clone()), StructKeyNode::Leaf(value))),
            }
            return;
        }
        match existing {
            Some(pos) => {
                if let StructKeyNode::Branch(c) = &mut children[pos].1 {
                    Self::insert_struct_path(c, rest, value);
                } else {
                    let mut c = Vec::new();
                    Self::insert_struct_path(&mut c, rest, value);
                    children[pos].1 = StructKeyNode::Branch(c);
                }
            }
            None => {
                let mut c = Vec::new();
                Self::insert_struct_path(&mut c, rest, value);
                children.push((StructKey::Static(head.clone()), StructKeyNode::Branch(c)));
            }
        }
    }

    /// Emit bytecode that builds a (possibly nested) struct from a struct-literal
    /// tree: for each child push its key string then its value (a leaf compiles
    /// the value expression; a branch recurses to build the nested struct), then
    /// a single `BuildStruct` over all the children.
    fn emit_struct_tree(
        &mut self,
        children: &[(StructKey, StructKeyNode)],
        instructions: &mut Vec<BytecodeOp>,
    ) {
        for (key, node) in children {
            match key {
                StructKey::Static(s) => instructions.push(BytecodeOp::String(s.clone())),
                StructKey::Computed(expr) => self.compile_expression(expr, instructions),
            }
            match node {
                StructKeyNode::Leaf(value) => self.compile_expression(value, instructions),
                StructKeyNode::Branch(c) => self.emit_struct_tree(c, instructions),
            }
        }
        instructions.push(BytecodeOp::BuildStruct(children.len()));
    }

    /// Scope roots whose nested member writes are routed through the runtime
    /// scope-path store (`SetDynamicVar` → `store_runtime_path`), which
    /// auto-vivifies missing intermediate structs scope-aware. The `this`
    /// scope is handled separately in `flatten_scope_path` (it parses to
    /// `Expression::This`, not an `Identifier`). Excludes
    /// `super`/`arguments`/`thread`, whose member chains keep their
    /// established struct-receiver writeback semantics (cfthread's thread.x
    /// capture relies on this; the page-level `thread` soft-scope is committed
    /// in the StoreLocal `thread` writeback arm instead).
    fn is_autoviv_scope_root(name: &str) -> bool {
        matches!(name.to_lowercase().as_str(),
            "local" | "variables" | "request" | "application" | "session" |
            "server" | "cgi" | "url" | "form" | "cookie" | "client" | "static"
        )
    }

    /// Flatten a pure member-access chain rooted at an auto-viv scope name into
    /// a dotted path string (e.g. `variables.zzc.name` → `"variables.zzc.name"`).
    /// Returns `None` unless the root is an auto-viv scope identifier and every
    /// level is a plain (non null-safe) member access — array indices, calls or
    /// dynamic members fall back to the generic assignment path.
    fn flatten_scope_path(expr: &Expression) -> Option<String> {
        match expr {
            Expression::Identifier(id) if Self::is_autoviv_scope_root(&id.name) => {
                Some(id.name.clone())
            }
            // `this` is its own AST node (Expression::This), not an Identifier.
            // A nested write rooted at it (`this.paths.migrate = v`) auto-vivifies
            // the same way: store_runtime_path resolves "this" via locals["this"].
            Expression::This(_) => Some("this".to_string()),
            Expression::MemberAccess(access) if !access.null_safe => {
                let base = Self::flatten_scope_path(&access.object)?;
                Some(format!("{}.{}", base, access.member))
            }
            _ => None,
        }
    }

    /// For a `<scope>.a.b…leaf = v` assignment target, return the full dotted
    /// path string IF the base is a multi-level member chain rooted at an
    /// auto-viv scope (i.e. ≥2 levels below the scope). Single-level writes
    /// like `variables.x = v` return `None` so they keep their existing,
    /// well-exercised compilation. This is the case that otherwise throws
    /// "Variable 'X' is undefined" or silently drops the write because the
    /// intermediate container was never declared.
    fn scope_rooted_nested_path(obj: &Expression, member: &str) -> Option<String> {
        if matches!(obj, Expression::MemberAccess(_)) {
            let base = Self::flatten_scope_path(obj)?;
            return Some(format!("{}.{}", base, member));
        }
        None
    }

    /// Like [`scope_rooted_nested_path`] but rooted at *any* plain identifier,
    /// not just an auto-viv scope name — e.g. `copies.request.cgi`, where
    /// `copies` is an undeclared bare variable. Returns the dotted path only
    /// when the target is ≥2 levels below a plain-`Identifier` root and every
    /// level is a plain (non null-safe) member access (no array index, call,
    /// or dynamic member). Used as a fallback after `scope_rooted_nested_path`
    /// so an unscoped, undeclared nested container auto-vivifies through
    /// `store_runtime_path` instead of throwing "Variable 'X' is undefined"
    /// when the generic store path reads the missing base. Lucee silently
    /// creates the intermediate structs (verified vs Lucee 7).
    fn bare_rooted_nested_path(obj: &Expression, member: &str) -> Option<String> {
        fn flatten_any(expr: &Expression) -> Option<String> {
            match expr {
                Expression::Identifier(id) => Some(id.name.clone()),
                Expression::MemberAccess(access) if !access.null_safe => {
                    let base = flatten_any(&access.object)?;
                    Some(format!("{}.{}", base, access.member))
                }
                _ => None,
            }
        }
        // Only ≥2-level chains (obj itself is a member access). A single-level
        // `x.y = v` keeps its StoreLocalProperty fast path (which already
        // auto-vivifies the bare local as a struct).
        if matches!(obj, Expression::MemberAccess(_)) {
            let base = flatten_any(obj)?;
            return Some(format!("{}.{}", base, member));
        }
        None
    }

    /// For a plain `=` assignment, return the dotted path string that names the
    /// target, so a Null RHS can DELETE it (CFML null-assignment semantics —
    /// `x = voidFn()` must leave the name undefined, not materialize a null key).
    /// Mirrors the store-side target dispatch in `compile_statement`. Returns
    /// `None` for targets we don't guard (array-element writes, exotic bases) —
    /// those keep their plain store behaviour.
    fn assign_unset_path(target: &AssignTarget) -> Option<String> {
        match target {
            AssignTarget::Variable(name) => Some(name.clone()),
            AssignTarget::StructAccess(obj, member) => {
                if let Some(path) = Self::scope_rooted_nested_path(obj, member) {
                    Some(path)
                } else if let Expression::Identifier(ref ident) = **obj {
                    // Single-level `base.member = null`. A reserved SCOPE root
                    // (variables/local/request/this/…) keeps scope
                    // null-assignment semantics (delete the key). A plain STRUCT
                    // variable does NOT: a null value stays an enumerable null
                    // key, matching the bracket form `s["x"]=null`, struct
                    // literals, and Lucee (GH #268).
                    if Self::is_reserved_scope_name(&ident.name) {
                        Some(format!("{}.{}", ident.name, member))
                    } else {
                        None
                    }
                } else {
                    None
                }
            }
            AssignTarget::ArrayAccess(_, _) => None,
        }
    }

    /// Same as [`assign_unset_path`] but for a script `=` assignment, whose LHS
    /// is an `Expression` (assignment-as-expression: `x = voidFn()` parses to a
    /// `BinaryOp{Assign}`). Returns the dotted target path for the
    /// value-CONSUMING store paths (`StoreLocal` / `StoreLocalProperty` /
    /// `SetProperty`). Returns `None` for scope-rooted-nested targets — those
    /// store via `SetDynamicVar`, whose `store_runtime_path` already deletes on
    /// a Null value — and for exotic bases (array element, computed object).
    fn expr_assign_unset_path(left: &Expression) -> Option<String> {
        match left {
            Expression::Identifier(id) => Some(id.name.clone()),
            Expression::MemberAccess(access) => {
                if Self::scope_rooted_nested_path(&access.object, &access.member).is_some() {
                    None
                } else if let Expression::Identifier(ref ident) = *access.object {
                    // See assign_unset_path: only a reserved SCOPE root keeps
                    // null-delete; a plain struct member keeps the enumerable
                    // null key (GH #268).
                    if Self::is_reserved_scope_name(&ident.name) {
                        Some(format!("{}.{}", ident.name, access.member))
                    } else {
                        None
                    }
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    /// Whether an expression could evaluate to Null at runtime. Used to decide
    /// if a plain `=` assignment needs the null-delete guard (a non-null RHS —
    /// literals, struct/array/closure/`new`, arithmetic/logical ops, and bare
    /// identifier reads — can skip it, keeping the hot path of `x = 5` /
    /// `x = a + b` / `var t = s` exactly as before). Only a function/method
    /// CALL return (void/null) or an explicit `null` is guarded.
    ///
    /// A bare `Identifier` read is non-null by CFML semantics: a defined
    /// variable holds a non-null value, and reading an *undefined* one THROWS
    /// rather than yielding Null — so `x = someVar` never assigns Null. Keeping
    /// identifier RHS unguarded matters beyond the hot path: the extra
    /// `JumpIfNotNull`/`UnsetPath` ops are outside the JIT's admitted op-subset,
    /// so guarding `var t = s` would silently disqualify the whole function
    /// from native compilation (regressed in v0.137.0, restored here).
    fn expr_may_be_null(expr: &Expression) -> bool {
        match expr {
            Expression::Literal(lit) => matches!(lit.value, LiteralValue::Null),
            Expression::Array(_)
            | Expression::Struct(_)
            | Expression::Closure(_)
            | Expression::ArrowFunction(_)
            | Expression::New(_)
            | Expression::StringInterpolation(_)
            | Expression::UnaryOp(_)
            | Expression::BinaryOp(_)
            | Expression::PostfixOp(_)
            | Expression::Identifier(_) => false,
            _ => true,
        }
    }

    /// Compile the base collection of an index-assignment target (`base[idx] = v`).
    /// A bare, non-scope identifier is loaded with TryLoadLocal so an undefined
    /// variable yields Null — which SetIndex then auto-vivifies into a struct or
    /// array (Lucee/ACF/BoxLang) — instead of throwing "Variable is undefined".
    /// Member/index bases (`a.b[k]`, `a[i][k]`) already read missing links as
    /// Null via GetProperty/GetIndex, so they use the normal compile path.
    fn compile_index_assign_base(&mut self, base: &Expression, instructions: &mut Vec<BytecodeOp>) {
        match base {
            Expression::Identifier(ident) if !Self::is_reserved_scope_name(&ident.name) => {
                instructions.push(BytecodeOp::TryLoadLocal(Name::from(&ident.name)));
            }
            // A nested base (`q["lineData"][0] = v`, `q.lineData[0] = v`): recurse
            // so the *root* identifier is the Null-tolerant TryLoadLocal above,
            // then walk back down with GetIndex/GetProperty — both of which yield
            // Null for a Null receiver or a missing key, so the whole base reads
            // as Null when nothing exists yet. The leaf SetIndex then
            // auto-vivifies the entire chain (Lucee/ACF/BoxLang), instead of the
            // generic compile path throwing "Variable '<root>' is undefined".
            Expression::ArrayAccess(access) => {
                self.compile_index_assign_base(&access.array, instructions);
                self.compile_expression(&access.index, instructions);
                instructions.push(BytecodeOp::GetIndex);
            }
            Expression::MemberAccess(access) => {
                self.compile_index_assign_base(&access.object, instructions);
                // Auto-viv base: a not-yet-existing link must read as Null so the
                // leaf SetIndex can build the chain — Try* twin, not throwing.
                instructions.push(BytecodeOp::TryGetProperty(Name::from(&access.member)));
            }
            _ => self.compile_expression(base, instructions),
        }
    }

    /// Push the current value of a compound-assignment target onto the stack.
    /// Used by `+=`, `-=`, `*=`, `/=`, `%=`, `&=` so the existing value can be
    /// combined with the RHS regardless of whether the target is a plain
    /// variable, a struct member, or an array element.
    fn emit_load_current_target(&mut self, target: &AssignTarget, instructions: &mut Vec<BytecodeOp>) {
        match target {
            AssignTarget::Variable(name) => {
                instructions.push(BytecodeOp::LoadLocal(Name::from(&name)));
            }
            AssignTarget::StructAccess(obj, member) => {
                // `local.x += 1`: read the single frame key (same Null-on-miss
                // filter as the materialized-view read below, minus the clone —
                // and slot-resolvable, see `emit_load_for_writeback`).
                if let Expression::Identifier(ref ident) = **obj {
                    // GH #351: emitted at EVERY depth. The op itself resolves
                    // `local` against the frame's real local-scope status, so a
                    // template included from inside a function keeps the caller's
                    // scope while a true page reads the ordinary `local` variable.
                    if ident.name.eq_ignore_ascii_case("local") {
                        instructions.push(BytecodeOp::TryLoadLocalKey(Name::from(&member)));
                        return;
                    }
                }
                // Compound assign (`s.x += 1`) reads the current value; preserve the
                // pre-existing Null-on-miss behaviour (treated as 0/"") rather than
                // throwing when the target member doesn't exist yet.
                self.compile_expression(obj, instructions);
                instructions.push(BytecodeOp::TryGetProperty(Name::from(&member)));
            }
            AssignTarget::ArrayAccess(arr, idx) => {
                self.compile_expression(arr, instructions);
                self.compile_expression(idx, instructions);
                instructions.push(BytecodeOp::GetIndex);
            }
        }
    }

    /// True when an Elvis left operand cannot raise an exception once compiled
    /// through `compile_member_read_tolerant` — i.e. a pure variable/member/index
    /// chain over identifiers and literals, where every genuine miss already reads
    /// as Null via the Try* ops. Such operands need no try/catch guard, which keeps
    /// the overwhelmingly common `s.a.b ?: d` shape at its previous op count
    /// (Elvis is hot in framework code). Anything else — a function call, an
    /// arithmetic/comparison expression, a call nested in an index — can throw and
    /// must be guarded so `?:` absorbs it the way Lucee does (GH #329).
    fn elvis_left_is_infallible(expr: &Expression) -> bool {
        match expr {
            Expression::Identifier(_) | Expression::Literal(_) => true,
            Expression::MemberAccess(m) => Self::elvis_left_is_infallible(&m.object),
            Expression::ArrayAccess(a) => {
                Self::elvis_left_is_infallible(&a.array) && Self::elvis_left_is_infallible(&a.index)
            }
            _ => false,
        }
    }

    /// Compile a member/array-access chain in a NULL-TOLERANT way: a missing
    /// receiver variable, a missing struct member, or a missing key reads as Null
    /// instead of throwing. This mirrors the normal read path (`compile_expression`
    /// for MemberAccess) but swaps every throwing member op for its Try* twin, so
    /// it can be used for the `?:` left operand, `isNull(...)`, and null-safe reads
    /// without a genuine miss aborting the whole expression. Non-access leaves fall
    /// back to the normal compiler.
    fn compile_member_read_tolerant(&mut self, expr: &Expression, instructions: &mut Vec<BytecodeOp>) {
        match expr {
            Expression::Identifier(ident) if !Self::is_reserved_scope_name(&ident.name) => {
                // A bare variable root that may be undefined → Null, not a throw.
                instructions.push(BytecodeOp::TryLoadLocal(Name::from(&ident.name)));
            }
            Expression::MemberAccess(access) if !access.null_safe => {
                // Mirror the normal read path's fusion peepholes so the JIT sees
                // the same op shapes it already specialises — just the Null-
                // tolerant twins (`local.foo` → TryLoadLocalKey, `<ident>.foo` →
                // TryLoadLocalProperty). Reserved-scope and nested roots recurse
                // then TryGetProperty.
                if let Expression::Identifier(ref ident) = *access.object {
                    // GH #351: emitted at EVERY depth. The op itself resolves
                    // `local` against the frame's real local-scope status, so a
                    // template included from inside a function keeps the caller's
                    // scope while a true page reads the ordinary `local` variable.
                    if ident.name.eq_ignore_ascii_case("local") {
                        instructions.push(BytecodeOp::TryLoadLocalKey(Name::from(&access.member)));
                        return;
                    }
                    if !is_reserved_scope_name(&ident.name) {
                        instructions.push(BytecodeOp::TryLoadLocalProperty(Name::from(&ident.name),Name::from(&access.member),
                        ));
                        return;
                    }
                }
                self.compile_member_read_tolerant(&access.object, instructions);
                instructions.push(BytecodeOp::TryGetProperty(Name::from(&access.member)));
            }
            Expression::ArrayAccess(access) => {
                // GetIndex already reads a missing key / Null receiver as Null.
                self.compile_member_read_tolerant(&access.array, instructions);
                self.compile_expression(&access.index, instructions);
                instructions.push(BytecodeOp::GetIndex);
            }
            // Reserved scopes (variables/local/this/…) always resolve to a live
            // scope struct; null-safe accesses and everything else keep their
            // normal (already Null-tolerant) compilation.
            _ => self.compile_expression(expr, instructions),
        }
    }

    /// Get the first argument expression of a function call (for mutating write-back).
    fn mutating_call_first_arg(expr: &Expression) -> Option<&Expression> {
        if let Expression::FunctionCall(call) = expr {
            call.arguments.first()
        } else {
            None
        }
    }

    /// Emit write-back instructions for nested property assignment.
    /// After SetProperty("leaf"), the modified intermediate object is on the stack.
    /// This walks up the MemberAccess chain, writing back to each parent level.
    /// e.g. for `s.a.b = val`: after SetProperty("b"), stack has modified s.a
    ///   → Load s, Swap, SetProperty("a") → stack has modified s → StoreLocal("s")
    /// Emit bytecode to write back a modified nested value through the property chain.
    /// Stack state on entry: [modified_value]
    /// For `s.a.b = val`, after SetProperty("b"), stack has [modified_a_struct].
    /// We need to: load s, swap, SetProperty("a"), StoreLocal("s").
    /// For deeper chains like `s.a.b.c = val`, we recurse up the chain.
    fn emit_nested_writeback(&mut self, obj: &Expression, instructions: &mut Vec<BytecodeOp>) {
        match obj {
            Expression::Identifier(ident) => {
                instructions.push(BytecodeOp::StoreLocal(Name::from(&ident.name)));
            }
            Expression::This(_) => {
                instructions.push(BytecodeOp::StoreLocal(Name::intern("this")));
            }
            Expression::MemberAccess(access) => {
                // `local.X` is the function frame itself, not a struct living in
                // it: write the single key with the SAME ops a plain
                // `local.X = v` assignment emits (DeclareLocal + StoreLocal, see
                // the Statement::Assignment StructAccess arm). The generic path
                // below would instead materialize the ENTIRE per-call `local`
                // scope view (`LoadLocal("local")`), SetProperty into that copy,
                // and merge the whole copy back (`StoreLocal("local")`) — three
                // costs for a one-key write: the view clone, the whole-map merge,
                // and (since v0.584.0) permanent deactivation of the frame's
                // slots, so every `local.i++`-idiom function lost the slot win.
                // Only single-segment `local.X` is special-cased; `local.a.b`
                // recurses here with `access.object == local.a` and reaches this
                // arm one level up, which is exactly the right write for it.
                if let Expression::Identifier(ref ident) = *access.object {
                    if ident.name.eq_ignore_ascii_case("local") {
                        if self.local_is_scope() {
                            instructions.push(BytecodeOp::DeclareLocal(Name::from(&access.member)));
                            instructions.push(BytecodeOp::StoreLocal(Name::from(&access.member)));
                        } else {
                            // GH #351: template level — the VM decides.
                            instructions.push(BytecodeOp::StoreLocalScopeKey(Name::from(&access.member)));
                        }
                        return;
                    }
                }
                // Stack has modified child value. Load the parent, swap, set property.
                // Then recurse to write back the parent.
                self.emit_load_for_writeback(&access.object, instructions);
                instructions.push(BytecodeOp::Swap);
                instructions.push(BytecodeOp::SetProperty(Name::from(&access.member)));
                self.emit_nested_writeback(&access.object, instructions);
            }
            Expression::ArrayAccess(access) => {
                // Stack has [modified_value]. We need to write it back into the parent collection.
                // Load the parent collection, then the index, then SetIndex, then recurse.
                // e.g. for `a.b[0][1] = val`: after inner SetIndex, stack has modified inner array.
                // We need to: load a.b[0], swap, push 0-index, SetIndex → modified a.b, then write back a.b.
                // Index uses the full compiler (complex/interpolated keys), matching the read path.
                self.emit_load_for_writeback(&access.array, instructions);
                self.compile_expression(&access.index, instructions);
                // Stack: [modified_value, parent_collection, index]
                // We need: [value_to_set, collection, index] for SetIndex
                // Rearrange: rotate so modified_value goes under collection
                // Actually SetIndex wants [value, collection, index] bottom-to-top
                // Current: [modified_value, parent_collection, index]
                // That's already correct for SetIndex
                instructions.push(BytecodeOp::SetIndex);
                self.emit_nested_writeback(&access.array, instructions);
            }
            _ => {
                instructions.push(BytecodeOp::Pop);
            }
        }
    }

    /// Emit a load instruction for the given expression (used during write-back chain).
    fn emit_load_for_writeback(&mut self, expr: &Expression, instructions: &mut Vec<BytecodeOp>) {
        match expr {
            Expression::Identifier(ident) => {
                // A non-scope root may not exist yet when a nested index/member
                // write auto-vivifies it (`q["a"]["b"] = v` with q undefined);
                // TryLoadLocal yields Null instead of throwing, and the parent
                // SetIndex/SetProperty + StoreLocal then build & store the chain.
                if Self::is_reserved_scope_name(&ident.name) {
                    instructions.push(BytecodeOp::LoadLocal(Name::from(&ident.name)));
                } else {
                    instructions.push(BytecodeOp::TryLoadLocal(Name::from(&ident.name)));
                }
            }
            Expression::This(_) => {
                instructions.push(BytecodeOp::LoadLocal(Name::intern("this")));
            }
            Expression::MemberAccess(access) => {
                // `local.X` reads one key out of the frame, not a member of a
                // materialized scope copy: the fused TryLoadLocalKey is the same
                // read the normal expression path already emits for `local.X`
                // (`compile_member_read_tolerant`), minus the whole-view clone —
                // and it has a slot twin, so a `local.a.b = v` write-back keeps
                // the frame's slots alive.
                if let Expression::Identifier(ref ident) = *access.object {
                    // GH #351: emitted at EVERY depth. The op itself resolves
                    // `local` against the frame's real local-scope status, so a
                    // template included from inside a function keeps the caller's
                    // scope while a true page reads the ordinary `local` variable.
                    if ident.name.eq_ignore_ascii_case("local") {
                        instructions.push(BytecodeOp::TryLoadLocalKey(Name::from(&access.member)));
                        return;
                    }
                }
                // For nested access like loading "s.a", we load s then get property a.
                // Write-back reads a not-yet-existing link as Null (auto-viv), so the
                // Try* twin — a throwing GetProperty would abort `s.a.b = v` when
                // `s.a` doesn't exist yet.
                self.emit_load_for_writeback(&access.object, instructions);
                instructions.push(BytecodeOp::TryGetProperty(Name::from(&access.member)));
            }
            Expression::ArrayAccess(access) => {
                // For nested access like loading "s.a[0]", we load s.a then get index 0.
                // The index must use the FULL expression compiler — a complex index
                // (interpolation `"total#t#"`, concat, a call) would otherwise fall
                // back to a Null and read the wrong cell.
                self.emit_load_for_writeback(&access.array, instructions);
                self.compile_expression(&access.index, instructions);
                instructions.push(BytecodeOp::GetIndex);
            }
            _ => {
                // Can't load this expression for writeback
                instructions.push(BytecodeOp::Null);
            }
        }
    }

    fn stmt_line(stmt: &Statement) -> Option<usize> {
        match stmt {
            Statement::Expression(e) => Some(e.location.start.line),
            Statement::Var(v) => Some(v.location.start.line),
            Statement::Assignment(a) => Some(a.location.start.line),
            Statement::If(i) => Some(i.location.start.line),
            Statement::For(f) => Some(f.location.start.line),
            Statement::ForIn(f) => Some(f.location.start.line),
            Statement::While(w) => Some(w.location.start.line),
            Statement::Do(d) => Some(d.location.start.line),
            Statement::Switch(s) => Some(s.location.start.line),
            Statement::Return(r) => Some(r.location.start.line),
            Statement::FunctionDecl(f) => Some(f.func.location.start.line),
            Statement::Try(t) => Some(t.location.start.line),
            Statement::Throw(t) => Some(t.location.start.line),
            Statement::Rethrow(loc) => Some(loc.start.line),
            Statement::ComponentDecl(c) => Some(c.component.location.start.line),
            Statement::InterfaceDecl(i) => Some(i.interface.location.start.line),
            Statement::Include(i) => Some(i.location.start.line),
            Statement::Break(b) => Some(b.location.start.line),
            Statement::Continue(c) => Some(c.location.start.line),
            Statement::Import(i) => Some(i.location.start.line),
            Statement::Output(o) => Some(o.location.start.line),
            Statement::PropertyDecl(p) => Some(p.prop.location.start.line),
            Statement::Exit => None,
        }
    }

    /// Emit the code for a `break`/`continue` that has no enclosing loop in the
    /// current function/closure. Lucee treats this as ending the current
    /// invocation (an `.each`/`map`/`filter` callback keeps iterating with the
    /// next element), so it compiles to a null `return` — running any enclosing
    /// finallys first, exactly like a bare `return;`. Emitting an unpatched
    /// `Jump(0)` instead (the old behavior) looped back to the closure's first
    /// instruction and spun the CPU forever.
    /// Emit, innermost first, the `finally` bodies opened between the jump site
    /// and the loop/switch frame a `break`/`continue` targets. The runtime jump
    /// does not run them, so without this a `break` out of a
    /// `transaction { }` / `lock { }` / `try {} finally {}` skipped the cleanup
    /// entirely — the transaction was neither committed nor rolled back and its
    /// connection stayed open for the rest of the request (GH #308).
    fn emit_finallys_above(&mut self, frame_depth: usize, instructions: &mut Vec<BytecodeOp>) {
        if self.finally_stack.len() <= frame_depth {
            return;
        }
        // Detach the ones being emitted so a `return`/`rethrow` inside one of
        // them does not re-emit the very finally that contains it (the same
        // self-reference guard the `return` path uses).
        let saved = std::mem::take(&mut self.finally_stack);
        for fb in saved[frame_depth..].iter().rev() {
            for s in fb {
                self.compile_statement(s, instructions);
            }
        }
        self.finally_stack = saved;
    }

    fn emit_break_out_of_closure(&mut self, instructions: &mut Vec<BytecodeOp>) {
        if !self.finally_stack.is_empty() {
            let saved = std::mem::take(&mut self.finally_stack);
            for fb in saved.iter().rev() {
                for s in fb {
                    self.compile_statement(s, instructions);
                }
            }
            self.finally_stack = saved;
        }
        instructions.push(BytecodeOp::Null);
        instructions.push(BytecodeOp::Return);
    }

    fn compile_statement(&mut self, stmt: &Statement, instructions: &mut Vec<BytecodeOp>) {
        if let Some(line) = Self::stmt_line(stmt) {
            instructions.push(BytecodeOp::LineInfo(line, 0));
        }

        match stmt {
            Statement::Expression(expr_stmt) => {
                // A bare identifier used as a statement (`j;`) is dead code:
                // reading a variable has no side effects and the result is
                // discarded, so emit nothing. Lucee/ACF evaluate such a
                // statement leniently — notably they do NOT throw when the
                // variable is undefined (Preside's PresideObjectReader
                // ._setUseDrafts ships a stray `{j` typo that boots fine on
                // Lucee). A bare word is never an implicit call in CFML — that
                // needs `()` — so this can't drop a side-effecting call.
                // A lowered custom-tag pair. By the time codegen runs, the tag
                // preprocessor has flattened `<cf_foo>body</cf_foo>` into three
                // ordinary statements — but the pair is still lexically visible
                // HERE, which is the only place the body's start index can be
                // known reliably. Record it on the start call and hand it to the
                // end call's `TagLoopBack`, which is what `<cfexit method="loop">`
                // rewinds to. See the `TagLoopBack` doc comment for why this is
                // resolved here rather than inferred from instruction layout at
                // runtime.
                if let Some(is_start) = Self::custom_tag_pair_call(&expr_stmt.expr) {
                    self.compile_expression(&expr_stmt.expr, instructions);
                    if is_start {
                        instructions.push(BytecodeOp::Pop);
                        self.tag_pair_stack.push(instructions.len());
                    } else {
                        match self.tag_pair_stack.pop() {
                            Some(body_start) => {
                                instructions.push(BytecodeOp::TagLoopBack(body_start))
                            }
                            // Unbalanced (an `__cfcustomtag_end()` with no start
                            // in this statement stream) — emit the plain Pop and
                            // let the VM raise its "without matching start"
                            // error, as it does today.
                            None => instructions.push(BytecodeOp::Pop),
                        }
                    }
                } else if matches!(&expr_stmt.expr, Expression::Identifier(_)) {
                    // no-op
                }
                // Peephole: `i++;` / `i--;` / `++i;` / `--i;` as a bare statement.
                // The normal 5-op expand (Load/Dup/Int1/Add/Store) plus a trailing
                // Pop collapses to a single Increment/Decrement.
                else if self.try_emit_inc_dec_statement(&expr_stmt.expr, instructions) {
                    // emitted; no Pop needed — the op has no stack effect
                }
                // `StructDelete(<scope>, keyExpr)` — a scope (request/variables/
                // session/…) is snapshotted when passed as a builtin arg, so the
                // in-place struct mutation can't reach the live scope. Delete the
                // key straight from the scope container instead. (Plain-struct
                // StructDelete falls through to the generic path below, where the
                // shared-Arc in-place mutation handles it; the boolean return is
                // discarded by the trailing Pop.)
                else if let Some((scope, key_expr)) =
                    Self::structdelete_scope_target(&expr_stmt.expr)
                {
                    self.compile_expression(key_expr, instructions);
                    instructions.push(BytecodeOp::DeleteScopeKey(Name::from(scope)));
                }
                // Check for mutating function calls: structAppend(a, b), structInsert(a, k, v), etc.
                // These return the modified struct; store it back to the first arg's location.
                else if Self::is_mutating_standalone_call(&expr_stmt.expr) {
                    if let Some(first_arg) = Self::mutating_call_first_arg(&expr_stmt.expr) {
                        match first_arg {
                            Expression::Identifier(ident)
                                if Self::is_inplace_array_append(&expr_stmt.expr, ident) =>
                            {
                                // Hot path: arrayAppend(<ident>, value) with exactly two
                                // args. Push the value, then append in place via the fused
                                // op — no array clone, no StoreLocal round-trip. This turns
                                // a quadratic append loop linear.
                                if let Expression::FunctionCall(call) = &expr_stmt.expr {
                                    self.compile_expression(&call.arguments[1], instructions);
                                }
                                instructions.push(BytecodeOp::ArrayAppendLocal(Name::from(&ident.name)));
                            }
                            Expression::Identifier(ident) => {
                                // Simple: structAppend(a, b) → compile call → StoreLocal(a)
                                self.compile_expression(&expr_stmt.expr, instructions);
                                instructions.push(BytecodeOp::StoreLocal(Name::from(&ident.name)));
                            }
                            Expression::MemberAccess(_) => {
                                // Nested: structAppend(local._taffy.settings, defaultConfig)
                                // → compile call → emit_nested_writeback(local._taffy.settings)
                                self.compile_expression(&expr_stmt.expr, instructions);
                                self.emit_nested_writeback(first_arg, instructions);
                            }
                            _ => {
                                // Can't write back — just pop
                                self.compile_expression(&expr_stmt.expr, instructions);
                                instructions.push(BytecodeOp::Pop);
                            }
                        }
                    } else {
                        self.compile_expression(&expr_stmt.expr, instructions);
                        instructions.push(BytecodeOp::Pop);
                    }
                } else {
                    self.compile_expression(&expr_stmt.expr, instructions);
                    instructions.push(BytecodeOp::Pop);
                }
            }
            Statement::Var(var) => {
                // `var local.X` is identical to `var X` — X already lives in the
                // local scope, so the `local.` prefix is redundant. Strip it so the
                // declare/store target the local key `X`. Without this, the name
                // reached `StoreLocal("local.X")`, which matches no scope branch and
                // lands under a flat `"local.X"` key that reads of `local.X` never
                // see — the initializer was silently dropped and a
                // `for (var local.i = 1; …)` counter started empty, lagged, and
                // over-ran (Lucee runs it fine). Only a single-segment key is
                // normalized; deeper paths (`var local.a.b`, `var foo.bar`) are left
                // untouched.
                // GH #351: the `local.` strip is a FUNCTION-scope normalisation.
                // At template level `local` is an ordinary variable and `var` is
                // not legal there on Lucee anyway, so the strip only applies
                // inside a function body.
                let name = match var.name.to_lowercase().strip_prefix("local.") {
                    Some(rest) if !rest.contains('.') && self.local_is_scope() => {
                        var.name[6..].to_string()
                    }
                    _ => var.name.clone(),
                };
                instructions.push(BytecodeOp::DeclareLocal(Name::from(&name)));
                if let Some(value) = &var.value {
                    // `var x = y = expr` — the initialiser is itself an assignment
                    // (value position), so it must LEAVE its assigned value on the
                    // stack for the `StoreLocal(x)` below to consume. Without
                    // `need_assign_value`, the inner assignment stored `y` but left
                    // nothing, so `x` was never bound — "Variable 'x' is undefined"
                    // (Preside's ObjectPicker.cfc:
                    // `var labelRenderer = args.labelRenderer = args.labelRenderer ?: …`).
                    // Same fix as the `return x = expr` arm below.
                    if matches!(value, Expression::BinaryOp(b) if b.operator == BinaryOpType::Assign)
                    {
                        self.need_assign_value = true;
                    }
                    self.compile_expression(value, instructions);
                    self.need_assign_value = false;
                    // `var x = voidFn()` — a Null initialiser must NOT create the
                    // key (CFML null-assignment semantics), same as `local.x =`.
                    if Self::expr_may_be_null(value) {
                        instructions.push(BytecodeOp::JumpIfNotNull(0)); // -> store (patched)
                        let guard_idx = instructions.len() - 1;
                        instructions.push(BytecodeOp::Pop); // drop the Null
                        instructions.push(BytecodeOp::UnsetPath(name.clone()));
                        instructions.push(BytecodeOp::Jump(0)); // -> end (patched)
                        let end_idx = instructions.len() - 1;
                        instructions[guard_idx] = BytecodeOp::JumpIfNotNull(instructions.len());
                        instructions.push(BytecodeOp::StoreLocal(Name::from(&name)));
                        instructions[end_idx] = BytecodeOp::Jump(instructions.len());
                    } else {
                        instructions.push(BytecodeOp::StoreLocal(Name::from(&name)));
                    }
                } else {
                    instructions.push(BytecodeOp::Null);
                    instructions.push(BytecodeOp::StoreLocal(Name::from(&name)));
                }
            }
            Statement::Assignment(assign) => {
                // Hot-path: x += <int>, x -= <int>, x *= <int> compile to a single
                // load-compute-store op inside locals. No stack traffic, no trailing
                // StoreLocal.
                if let AssignTarget::Variable(name) = &assign.target {
                    if let Some(k) = int_lit(&assign.value) {
                        let op = match &assign.operator {
                            AssignOp::PlusEqual  => Some(BytecodeOp::AddLocalConst(Name::from(&name),  k)),
                            AssignOp::MinusEqual => Some(BytecodeOp::AddLocalConst(Name::from(&name), -k)),
                            AssignOp::StarEqual  => Some(BytecodeOp::MulLocalConst(Name::from(&name),  k)),
                            _ => None,
                        };
                        if let Some(op) = op {
                            instructions.push(op);
                            return;
                        }
                    }
                }

                self.compile_expression(&assign.value, instructions);

                // Stack on entry to each arithmetic/concat arm: [rhs]. We push the
                // target's current value, then a RUNTIME Swap to get [current, rhs]
                // (the correct order for the non-commutative ops), then the op.
                // A compile-time instruction swap would only be correct when the
                // RHS is a single push; for multi-op RHS like `x += arr[i]` or
                // `x -= obj.p` it corrupts the bytecode. The current-value load
                // must cover all three target kinds, not just plain variables.
                match &assign.operator {
                    AssignOp::PlusEqual => {
                        self.emit_load_current_target(&assign.target, instructions);
                        instructions.push(BytecodeOp::Swap);
                        instructions.push(BytecodeOp::Add);
                    }
                    AssignOp::MinusEqual => {
                        self.emit_load_current_target(&assign.target, instructions);
                        instructions.push(BytecodeOp::Swap);
                        instructions.push(BytecodeOp::Sub);
                    }
                    AssignOp::StarEqual => {
                        self.emit_load_current_target(&assign.target, instructions);
                        instructions.push(BytecodeOp::Swap);
                        instructions.push(BytecodeOp::Mul);
                    }
                    AssignOp::SlashEqual => {
                        self.emit_load_current_target(&assign.target, instructions);
                        instructions.push(BytecodeOp::Swap);
                        instructions.push(BytecodeOp::Div);
                    }
                    AssignOp::PercentEqual => {
                        self.emit_load_current_target(&assign.target, instructions);
                        instructions.push(BytecodeOp::Swap);
                        instructions.push(BytecodeOp::Mod);
                    }
                    AssignOp::ConcatEqual => {
                        self.emit_load_current_target(&assign.target, instructions);
                        instructions.push(BytecodeOp::Swap);
                        instructions.push(BytecodeOp::Concat);
                    }
                    AssignOp::Equal => {} // Value already on stack
                }

                // CFML null-assignment semantics: `x = voidFn()` (a function
                // returning null/void, or an explicit `x = null`) must NOT create
                // the target key and must DELETE a pre-existing one — the name
                // stays undefined in every scope. Guard the store with the
                // existing JumpIfNotNull (peeks, doesn't pop): on a non-null RHS
                // it jumps straight to the normal store; on Null it falls through
                // to Pop + UnsetPath. Only plain `=` with a derivable target path
                // AND a possibly-null RHS pays for the guard — literal/arithmetic
                // assignments keep their original single-store bytecode.
                let mut unset_end_jump = None;
                if matches!(assign.operator, AssignOp::Equal)
                    && Self::expr_may_be_null(&assign.value)
                {
                    if let Some(path) = Self::assign_unset_path(&assign.target) {
                        instructions.push(BytecodeOp::JumpIfNotNull(0)); // -> store (patched)
                        let guard_idx = instructions.len() - 1;
                        instructions.push(BytecodeOp::Pop); // drop the Null
                        instructions.push(BytecodeOp::UnsetPath(path));
                        instructions.push(BytecodeOp::Jump(0)); // -> end (patched)
                        unset_end_jump = Some(instructions.len() - 1);
                        // The store ops emitted next are the JumpIfNotNull target.
                        instructions[guard_idx] = BytecodeOp::JumpIfNotNull(instructions.len());
                    }
                }

                match &assign.target {
                    AssignTarget::Variable(name) => {
                        instructions.push(BytecodeOp::StoreLocal(Name::from(&name)));
                    }
                    AssignTarget::ArrayAccess(arr, idx) => {
                        self.compile_index_assign_base(arr, instructions);
                        self.compile_expression(idx, instructions);
                        instructions.push(BytecodeOp::SetIndex);
                        // SetIndex leaves modified collection on stack; write it back
                        self.emit_nested_writeback(arr, instructions);
                    }
                    AssignTarget::StructAccess(obj, member) => {
                        // Nested write to an undeclared scope-qualified container
                        // (`variables.zzc.name = v`): route through the runtime
                        // scope-path store, which auto-vivifies every missing
                        // intermediate struct scope-aware. Reading the base first
                        // (the generic path below) would throw "Variable 'zzc' is
                        // undefined" at page scope or silently drop the write
                        // elsewhere. Stack on entry is [value]; SetDynamicVar wants
                        // [path, value], so push the path and Swap.
                        if let Some(path) = Self::scope_rooted_nested_path(obj, member) {
                            instructions.push(BytecodeOp::String(path));
                            instructions.push(BytecodeOp::Swap);
                            instructions.push(BytecodeOp::SetDynamicVar);
                            // SetDynamicVar pushes the value back; this is a
                            // statement, so discard it.
                            instructions.push(BytecodeOp::Pop);
                        } else if let Some(path) = Self::bare_rooted_nested_path(obj, member) {
                            // Undeclared bare root ≥2 levels deep
                            // (`copies.request.cgi = v`): same auto-vivifying
                            // runtime store as the scope-rooted case, so the
                            // missing `copies` container is created instead of
                            // throwing "Variable 'copies' is undefined".
                            instructions.push(BytecodeOp::String(path));
                            instructions.push(BytecodeOp::Swap);
                            instructions.push(BytecodeOp::SetDynamicVar);
                            instructions.push(BytecodeOp::Pop);
                        } else if let Expression::Identifier(ref ident) = **obj {
                            if !is_reserved_scope_name(&ident.name) {
                                instructions.push(BytecodeOp::StoreLocalProperty(Name::from(&ident.name),Name::from(&member),
                                ));
                            } else if ident.name.eq_ignore_ascii_case("local") && self.local_is_scope() {
                                // GH #351: only inside a function body — see the
                                // depth-0 branch below for why.
                                // `local.X = v` is identical to `var X = v` in CFML —
                                // function-frame scope, must NOT propagate to caller at
                                // return. Compile to DeclareLocal + StoreLocal so the
                                // classic-localmode writeback loop skips it (same as `var`).
                                instructions.push(BytecodeOp::DeclareLocal(Name::from(&member)));
                                instructions.push(BytecodeOp::StoreLocal(Name::from(&member)));
                            } else if ident.name.eq_ignore_ascii_case("local") {
                                // GH #351: template level — whether this frame
                                // owns a `local` scope is a RUNTIME question, so
                                // hand the decision to StoreLocalScopeKey rather
                                // than guessing here.
                                instructions.push(BytecodeOp::StoreLocalScopeKey(Name::from(&member)));
                            } else {
                                self.compile_expression(obj, instructions);
                                instructions.push(BytecodeOp::Swap);
                                instructions.push(BytecodeOp::SetProperty(Name::from(&member)));
                                self.emit_nested_writeback(obj, instructions);
                            }
                        } else {
                            // obj is itself an index/member chain (`q["a"].foo = v`).
                            // Use the auto-vivifying base compile so an undefined
                            // root reads as Null (and SetProperty/the write-back
                            // build the chain) rather than throwing.
                            self.compile_index_assign_base(obj, instructions);
                            instructions.push(BytecodeOp::Swap);
                            instructions.push(BytecodeOp::SetProperty(Name::from(&member)));
                            self.emit_nested_writeback(obj, instructions);
                        }
                    }
                }

                // Close the null-delete guard: the store branch jumps here, past
                // the Pop+UnsetPath sequence emitted before it.
                if let Some(idx) = unset_end_jump {
                    instructions[idx] = BytecodeOp::Jump(instructions.len());
                }
            }
            Statement::Return(ret) => {
                if let Some(value) = &ret.value {
                    // A return value is a VALUE position, so `return x = expr`
                    // must yield the assigned value (Lucee/ACF/BoxLang), exactly
                    // like an assignment used as an RHS. Without setting
                    // `need_assign_value`, the assignment stored but left nothing
                    // on the stack, so the function returned null — e.g. Preside's
                    // `return alerts = obj.selectData(...)` returned null and the
                    // caller's `criticalAlerts.recordCount` threw "undefined"
                    // (surfaced on the admin sitetree once GH #259 was fixed).
                    if matches!(value, Expression::BinaryOp(b) if b.operator == BinaryOpType::Assign)
                    {
                        self.need_assign_value = true;
                    }
                    self.compile_expression(value, instructions);
                    self.need_assign_value = false;
                } else {
                    instructions.push(BytecodeOp::Null);
                }
                // Run every enclosing finally (innermost first) before exiting:
                // the runtime Return op does not run finallys, so a `return`
                // inside a `lock {}` / `try {} finally {}` would otherwise skip
                // the unlock/cleanup (e.g. leak the lock → next acquire deadlocks).
                // Stash the return value in a temp local first so the finally
                // bodies run on a clean operand stack (they are not guaranteed
                // net-zero relative to an extra value sitting beneath them); the
                // `__`-prefix keeps the temp out of the variables-scope writeback.
                if !self.finally_stack.is_empty() {
                    instructions.push(BytecodeOp::StoreLocal(Name::intern("__cf_finally_retval")));
                    // Take the whole stack while emitting the finallys inline, so a
                    // `return`/`rethrow` that appears INSIDE a finally body does not
                    // re-emit the very finallys being emitted (which contain it) —
                    // that self-reference recurses until the native stack overflows
                    // at compile time. The finallys currently unwinding are no longer
                    // "enclosing" for statements within them. Restored afterwards so
                    // sibling statements still see the correct enclosing finallys.
                    let saved = std::mem::take(&mut self.finally_stack);
                    for fb in saved.iter().rev() {
                        for s in fb {
                            self.compile_statement(s, instructions);
                        }
                    }
                    self.finally_stack = saved;
                    instructions.push(BytecodeOp::LoadLocal(Name::intern("__cf_finally_retval")));
                }
                instructions.push(BytecodeOp::Return);
            }
            Statement::If(if_stmt) => {
                self.compile_if(if_stmt, instructions);
            }
            Statement::For(for_stmt) => {
                self.compile_for(for_stmt, instructions);
            }
            Statement::ForIn(for_in) => {
                self.compile_for_in(for_in, instructions);
            }
            Statement::While(while_stmt) => {
                self.compile_while(while_stmt, instructions);
            }
            Statement::Do(do_stmt) => {
                self.compile_do(do_stmt, instructions);
            }
            Statement::Switch(switch_stmt) => {
                self.compile_switch(switch_stmt, instructions);
            }
            Statement::Break(_) => {
                if let Some(entry_depth) = self.loop_stack.last().map(|c| c.3) {
                    // Jumping out of any custom-tag bodies opened inside this
                    // loop: their `__cfcustomtag_end()` will never run, so
                    // abandon them here or their captured output buffers leak
                    // and swallow the rest of the page.
                    self.emit_abandon_tag_pairs(entry_depth, instructions);
                }
                if let Some(finally_depth) = self.loop_stack.last().map(|c| c.4) {
                    self.emit_finallys_above(finally_depth, instructions);
                }
                if let Some(loop_ctx) = self.loop_stack.last_mut() {
                    // Push a placeholder jump that will be patched to the loop exit.
                    let idx = instructions.len();
                    instructions.push(BytecodeOp::Jump(0)); // placeholder
                    loop_ctx.0.push(idx); // break indices
                } else {
                    // `break` with NO enclosing loop — e.g. inside an `.each()` /
                    // closure callback. Lucee ends the current closure invocation
                    // (iteration continues), so this is a null return, NOT an
                    // unpatched `Jump(0)` that loops back to the closure start and
                    // spins forever (Preside admin Layout.localePicker did exactly
                    // this: `locales.each(function(l){ if(...) { ...; break; } })`).
                    self.emit_break_out_of_closure(instructions);
                }
            }
            Statement::Continue(_) => {
                // `continue` targets the enclosing LOOP, not a `switch` it sits
                // inside (a switch has no loop semantics). Skip switch frames.
                if let Some(entry_depth) =
                    self.loop_stack.iter().rev().find(|c| c.2).map(|c| c.3)
                {
                    self.emit_abandon_tag_pairs(entry_depth, instructions);
                }
                if let Some(finally_depth) =
                    self.loop_stack.iter().rev().find(|c| c.2).map(|c| c.4)
                {
                    self.emit_finallys_above(finally_depth, instructions);
                }
                if let Some(loop_ctx) = self.loop_stack.iter_mut().rev().find(|c| c.2) {
                    let idx = instructions.len();
                    instructions.push(BytecodeOp::Jump(0)); // placeholder
                    loop_ctx.1.push(idx); // continue indices
                } else {
                    // `continue` with no enclosing loop behaves like `break` here:
                    // end the current closure invocation rather than jump-to-0.
                    self.emit_break_out_of_closure(instructions);
                }
            }
            Statement::Try(try_stmt) => {
                self.compile_try(try_stmt, instructions);
            }
            Statement::Throw(throw_stmt) => {
                if let Some(msg) = &throw_stmt.message {
                    self.compile_expression(msg, instructions);
                } else {
                    instructions.push(BytecodeOp::String("An error occurred".to_string()));
                }
                instructions.push(BytecodeOp::Throw);
            }
            Statement::Rethrow(_loc) => {
                // Emit the innermost enclosing finally before rethrow (a catch's
                // rethrow must run its own try's finally before the exception
                // propagates; outer finallys run when the exception reaches their
                // runtime handlers).
                if let Some(finally_body) = self.finally_stack.last().cloned() {
                    // Pop the finally being emitted inline so a `rethrow` (or
                    // `return`) INSIDE this finally body does not re-emit the same
                    // finally that contains it. A `try {} catch { rethrow }` nested
                    // in a finally block (Preside TaskManagerService) would otherwise
                    // recurse here until the native stack overflows at compile time:
                    // the inner try pushes no finally of its own, so the inner
                    // rethrow re-reads THIS finally off the stack and re-emits it.
                    let popped = self.finally_stack.pop();
                    // Preserve the caught exception across the finally body: a
                    // try/catch inside it that throws-and-swallows must not change
                    // which exception the following Rethrow re-raises.
                    instructions.push(BytecodeOp::SaveException);
                    for s in &finally_body {
                        self.compile_statement(s, instructions);
                    }
                    instructions.push(BytecodeOp::RestoreException);
                    if let Some(p) = popped {
                        self.finally_stack.push(p);
                    }
                }
                // GH #244: re-raise the exception caught by the enclosing catch
                // clause, not whatever `last_exception` currently holds — a nested
                // try/catch in the same catch body (or an inline finally above)
                // may have overwritten it. The catch variable still holds the full
                // cfcatch struct (type/message/detail/tagcontext). Emitted AFTER
                // any inline finally so it wins.
                if let Some(catch_var) = self.catch_var_stack.last() {
                    instructions.push(BytecodeOp::SetLastExceptionFromLocal(Name::from(&catch_var)));
                }
                instructions.push(BytecodeOp::Rethrow);
            }
            Statement::FunctionDecl(func_decl) => {
                self.compile_function_decl(&func_decl.func, instructions);
            }
            Statement::ComponentDecl(comp_decl) => {
                // Compile component as a struct with methods
                self.compile_component(&comp_decl.component, instructions);
            }
            Statement::InterfaceDecl(iface_decl) => {
                self.compile_interface(&iface_decl.interface, instructions);
            }
            Statement::Include(inc) => {
                // Static path: emit Include(path) directly
                if let Expression::Literal(lit) = &inc.path {
                    if let LiteralValue::String(path) = &lit.value {
                        instructions.push(BytecodeOp::Include(path.clone()));
                        return;
                    }
                }
                // Dynamic path: compile expression, pop from stack at runtime
                self.compile_expression(&inc.path, instructions);
                instructions.push(BytecodeOp::IncludeDynamic);
            }
            Statement::Import(_) => {
                // Import not yet supported at bytecode level
            }
            Statement::Exit => {
                instructions.push(BytecodeOp::Halt);
            }
            Statement::Output(output) => {
                // Compile each statement in the output block body
                for body_stmt in &output.body {
                    self.compile_statement(body_stmt, instructions);
                }
            }
            _ => {}
        }
    }

    fn compile_if(&mut self, if_stmt: &If, instructions: &mut Vec<BytecodeOp>) {
        let jump_false_idx = self.emit_cond_jump_false(&if_stmt.condition, instructions);

        // Then branch
        for s in &if_stmt.then_branch {
            self.compile_statement(s, instructions);
        }

        if !if_stmt.else_if.is_empty() || if_stmt.else_branch.is_some() {
            let jump_end_idx = instructions.len();
            instructions.push(BytecodeOp::Jump(0)); // placeholder

            // Patch the jump-to-else
            let end_of_then = instructions.len();
            Self::patch_cond_jump_target(instructions, jump_false_idx, end_of_then);

            // Else-if chains
            let mut end_jumps = vec![jump_end_idx];

            for (_i, else_if) in if_stmt.else_if.iter().enumerate() {
                let jf_idx = self.emit_cond_jump_false(&else_if.condition, instructions);

                for s in &else_if.body {
                    self.compile_statement(s, instructions);
                }

                let je_idx = instructions.len();
                instructions.push(BytecodeOp::Jump(0));
                end_jumps.push(je_idx);

                let after_arm = instructions.len();
                Self::patch_cond_jump_target(instructions, jf_idx, after_arm);
            }

            // Else branch
            if let Some(else_branch) = &if_stmt.else_branch {
                for s in else_branch {
                    self.compile_statement(s, instructions);
                }
            }

            // Patch all end jumps
            let end_pos = instructions.len();
            for idx in end_jumps {
                instructions[idx] = BytecodeOp::Jump(end_pos);
            }
        } else {
            let end_of_then = instructions.len();
            Self::patch_cond_jump_target(instructions, jump_false_idx, end_of_then);
        }
    }

    /// Peephole: if `expr` is a postfix/prefix inc/dec of a plain identifier and
    /// If `expr` is `<identifier> <cmp> <int-literal>` (either side), returns
    /// `(name, const, op)` with `op` oriented so that truthiness means
    /// "identifier CMP const" — i.e. the condition is true when the
    /// comparison evaluates that way. Used by `compile_for` to fuse the loop
    /// condition into `JumpIfLocalCmpConstFalse`.
    fn match_local_cmp_const(expr: &Expression) -> Option<(String, i64, CmpOp)> {
        let bin = match expr {
            Expression::BinaryOp(b) => b,
            _ => return None,
        };
        let cmp = match bin.operator {
            BinaryOpType::Less => CmpOp::Lt,
            BinaryOpType::LessEqual => CmpOp::Lte,
            BinaryOpType::Greater => CmpOp::Gt,
            BinaryOpType::GreaterEqual => CmpOp::Gte,
            BinaryOpType::Equal => CmpOp::Eq,
            BinaryOpType::NotEqual => CmpOp::Neq,
            _ => return None,
        };
        let int_lit = |e: &Expression| -> Option<i64> {
            if let Expression::Literal(lit) = e {
                if let LiteralValue::Int(n) = &lit.value {
                    return Some(*n);
                }
            }
            None
        };
        let ident_name = |e: &Expression| -> Option<String> {
            if let Expression::Identifier(id) = e {
                Some(id.name.clone())
            } else {
                None
            }
        };
        if let (Some(name), Some(c)) = (ident_name(&bin.left), int_lit(&bin.right)) {
            Some((name, c, cmp))
        } else if let (Some(c), Some(name)) = (int_lit(&bin.left), ident_name(&bin.right)) {
            // `CONST <cmp> ident` — flip the op so the semantics stay right.
            let flipped = match cmp {
                CmpOp::Lt => CmpOp::Gt,
                CmpOp::Lte => CmpOp::Gte,
                CmpOp::Gt => CmpOp::Lt,
                CmpOp::Gte => CmpOp::Lte,
                CmpOp::Eq => CmpOp::Eq,
                CmpOp::Neq => CmpOp::Neq,
            };
            Some((name, c, flipped))
        } else {
            None
        }
    }

    /// Emit a condition followed by a "jump-if-false" exit. If the condition
    /// matches `<ident> <cmp> <int-const>`, emits a single fused
    /// JumpIfLocalCmpConstFalse. Otherwise compile_expression + JumpIfFalse.
    /// Returns the index of the jump op (so the caller can patch the target).
    fn emit_cond_jump_false(
        &mut self,
        condition: &Expression,
        instructions: &mut Vec<BytecodeOp>,
    ) -> usize {
        if let Some((name, c, cmp)) = Self::match_local_cmp_const(condition) {
            let idx = instructions.len();
            instructions.push(BytecodeOp::JumpIfLocalCmpConstFalse(Name::from(name), c, cmp, 0));
            idx
        } else {
            self.compile_expression(condition, instructions);
            let idx = instructions.len();
            instructions.push(BytecodeOp::JumpIfFalse(0));
            idx
        }
    }

    /// Patch the jump target of either BytecodeOp::JumpIfFalse or the fused
    /// BytecodeOp::JumpIfLocalCmpConstFalse at `idx`.
    fn patch_cond_jump_target(instructions: &mut [BytecodeOp], idx: usize, target: usize) {
        match &mut instructions[idx] {
            BytecodeOp::JumpIfFalse(off) => *off = target,
            BytecodeOp::JumpIfLocalCmpConstFalse(_, _, _, off) => *off = target,
            _ => unreachable!("patch_cond_jump_target on unexpected op"),
        }
    }

    /// If `expr` advances a plain identifier by a constant integer step,
    /// returns `(name, step)`. Recognises all of:
    ///   - `i++` / `i--` / `++i` / `--i`       → step = ±1
    ///   - `i += K` / `i -= K` (int literal K)   → step = ±K
    ///   - `i = i + K` / `i = K + i` / `i = i - K` (int literal K)
    /// Used by compile_for to detect counted-loop shapes for ForLoopStep
    /// fusion; ForLoopStep encodes the step as an i64 so non-±1 strides
    /// like `i += 7` fuse too.
    fn match_inc_dec_identifier(expr: &Expression) -> Option<(String, i64)> {
        let int_lit = |e: &Expression| -> Option<i64> {
            if let Expression::Literal(lit) = e {
                if let LiteralValue::Int(n) = &lit.value {
                    return Some(*n);
                }
            }
            None
        };
        let ident_name = |e: &Expression| -> Option<String> {
            if let Expression::Identifier(id) = e {
                Some(id.name.clone())
            } else {
                None
            }
        };
        match expr {
            Expression::PostfixOp(postfix) => {
                if let Expression::Identifier(ident) = &*postfix.operand {
                    let step = match postfix.operator {
                        PostfixOpType::Increment => 1,
                        PostfixOpType::Decrement => -1,
                    };
                    return Some((ident.name.clone(), step));
                }
                None
            }
            Expression::UnaryOp(unary) => {
                if let Expression::Identifier(ident) = &*unary.operand {
                    let step = match unary.operator {
                        UnaryOpType::PrefixIncrement => 1,
                        UnaryOpType::PrefixDecrement => -1,
                        _ => return None,
                    };
                    return Some((ident.name.clone(), step));
                }
                None
            }
            // `i = i + K`, `i = K + i`, `i = i - K` — parser represents this
            // as a top-level BinaryOp with operator Assign, LHS the target
            // identifier and RHS the value expression.
            Expression::BinaryOp(outer) if matches!(outer.operator, BinaryOpType::Assign) => {
                let name = ident_name(&outer.left)?;
                let inner = match &*outer.right {
                    Expression::BinaryOp(b) => b,
                    _ => return None,
                };
                match inner.operator {
                    BinaryOpType::Add => {
                        if let (Some(l), Some(k)) =
                            (ident_name(&inner.left), int_lit(&inner.right))
                        {
                            if l == name {
                                return Some((name, k));
                            }
                        }
                        if let (Some(k), Some(r)) =
                            (int_lit(&inner.left), ident_name(&inner.right))
                        {
                            if r == name {
                                return Some((name, k));
                            }
                        }
                        None
                    }
                    BinaryOpType::Sub => {
                        if let (Some(l), Some(k)) =
                            (ident_name(&inner.left), int_lit(&inner.right))
                        {
                            if l == name {
                                return Some((name, -k));
                            }
                        }
                        None
                    }
                    _ => None,
                }
            }
            _ => None,
        }
    }

    /// Statement-level counterpart to `match_inc_dec_identifier`. In a for
    /// loop the stride lives in the `increment` slot as an Expression, but
    /// in a while/do-while body it's a Statement — either an expression
    /// statement (`i++;`) or an Assignment (`i = i + 1;`, `i += K;`).
    /// Returns (name, step) on a match.
    fn match_stride_statement(stmt: &Statement) -> Option<(String, i64)> {
        let int_lit = |e: &Expression| -> Option<i64> {
            if let Expression::Literal(lit) = e {
                if let LiteralValue::Int(n) = &lit.value {
                    return Some(*n);
                }
            }
            None
        };
        let ident_name = |e: &Expression| -> Option<String> {
            if let Expression::Identifier(id) = e {
                Some(id.name.clone())
            } else {
                None
            }
        };
        match stmt {
            Statement::Expression(es) => Self::match_inc_dec_identifier(&es.expr),
            Statement::Assignment(a) => {
                let name = match &a.target {
                    AssignTarget::Variable(n) => n.clone(),
                    _ => return None,
                };
                match a.operator {
                    AssignOp::PlusEqual => int_lit(&a.value).map(|k| (name, k)),
                    AssignOp::MinusEqual => int_lit(&a.value).map(|k| (name, -k)),
                    AssignOp::Equal => {
                        // `i = i + K` / `i = K + i` / `i = i - K`
                        let inner = match &a.value {
                            Expression::BinaryOp(b) => b,
                            _ => return None,
                        };
                        match inner.operator {
                            BinaryOpType::Add => {
                                if let (Some(l), Some(k)) =
                                    (ident_name(&inner.left), int_lit(&inner.right))
                                {
                                    if l.eq_ignore_ascii_case(&name) {
                                        return Some((name, k));
                                    }
                                }
                                if let (Some(k), Some(r)) =
                                    (int_lit(&inner.left), ident_name(&inner.right))
                                {
                                    if r.eq_ignore_ascii_case(&name) {
                                        return Some((name, k));
                                    }
                                }
                                None
                            }
                            BinaryOpType::Sub => {
                                if let (Some(l), Some(k)) =
                                    (ident_name(&inner.left), int_lit(&inner.right))
                                {
                                    if l.eq_ignore_ascii_case(&name) {
                                        return Some((name, -k));
                                    }
                                }
                                None
                            }
                            _ => None,
                        }
                    }
                    _ => None,
                }
            }
            _ => None,
        }
    }

    /// Returns true if the body contains a `continue` statement that would
    /// target THIS loop (i.e. not inside a nested loop or function body).
    /// Used to decide whether stride-hoisting is safe: hoisting the stride
    /// out changes what `continue` skips vs. runs, so we bail when a
    /// top-level continue is present.
    fn body_has_top_level_continue(body: &[Statement]) -> bool {
        for stmt in body {
            if Self::stmt_has_top_level_continue(stmt) {
                return true;
            }
        }
        false
    }

    fn stmt_has_top_level_continue(stmt: &Statement) -> bool {
        match stmt {
            Statement::Continue(_) => true,
            Statement::If(i) => {
                Self::body_has_top_level_continue(&i.then_branch)
                    || i.else_if
                        .iter()
                        .any(|ei| Self::body_has_top_level_continue(&ei.body))
                    || i.else_branch
                        .as_ref()
                        .map(|b| Self::body_has_top_level_continue(b))
                        .unwrap_or(false)
            }
            Statement::Switch(s) => {
                s.cases
                    .iter()
                    .any(|c| Self::body_has_top_level_continue(&c.body))
                    || s.default_case
                        .as_ref()
                        .map(|b| Self::body_has_top_level_continue(b))
                        .unwrap_or(false)
            }
            Statement::Try(t) => {
                Self::body_has_top_level_continue(&t.body)
                    || t.catches
                        .iter()
                        .any(|c| Self::body_has_top_level_continue(&c.body))
                    || t.finally_body
                        .as_ref()
                        .map(|b| Self::body_has_top_level_continue(b))
                        .unwrap_or(false)
            }
            Statement::Output(o) => Self::body_has_top_level_continue(&o.body),
            // Nested loops and function decls define their own continue target —
            // continues inside them don't target the outer loop.
            Statement::For(_)
            | Statement::ForIn(_)
            | Statement::While(_)
            | Statement::Do(_)
            | Statement::FunctionDecl(_)
            | Statement::ComponentDecl(_)
            | Statement::InterfaceDecl(_) => false,
            _ => false,
        }
    }

    /// its result is about to be discarded, emit a single `Increment` /
    /// `Decrement` op (pure side-effect, no stack push) and return true.
    /// Saves 5 ops → 1 op per iteration on tight `i++`-style loops, which is
    /// the dominant bytecode in `for (i=...;...;i++)` — the hottest loop shape
    /// in CFML.
    fn try_emit_inc_dec_statement(
        &mut self,
        expr: &Expression,
        instructions: &mut Vec<BytecodeOp>,
    ) -> bool {
        // Helper: emit `target = target + delta` as a statement (no stack
        // leftover) for any assignable member/index target. Reads via the normal
        // expression path and writes via emit_nested_writeback, both of which
        // already support MemberAccess (`obj.m`) AND ArrayAccess (`obj[k]`) —
        // including nested chains like `variables.lookup[id]["totalPass"]`.
        fn emit_assignable_step(
            this: &mut CfmlCompiler,
            operand: &Expression,
            delta: i64,
            instructions: &mut Vec<BytecodeOp>,
        ) {
            this.compile_expression(operand, instructions); // read current value
            instructions.push(BytecodeOp::Integer(delta));
            instructions.push(BytecodeOp::Add);
            this.emit_nested_writeback(operand, instructions); // write back
        }

        // True when an inc/dec target is an assignable member/index path that the
        // step helper + writeback can handle (vs a bare identifier, handled above).
        fn is_assignable_path(e: &Expression) -> bool {
            matches!(e, Expression::MemberAccess(_) | Expression::ArrayAccess(_))
        }

        match expr {
            Expression::PostfixOp(postfix) => {
                if let Expression::Identifier(ident) = &*postfix.operand {
                    match postfix.operator {
                        PostfixOpType::Increment => {
                            instructions.push(BytecodeOp::Increment(Name::from(&ident.name)));
                            return true;
                        }
                        PostfixOpType::Decrement => {
                            instructions.push(BytecodeOp::Decrement(Name::from(&ident.name)));
                            return true;
                        }
                    }
                }
                if is_assignable_path(&postfix.operand) {
                    let delta = match postfix.operator {
                        PostfixOpType::Increment => 1,
                        PostfixOpType::Decrement => -1,
                    };
                    emit_assignable_step(self, &postfix.operand, delta, instructions);
                    return true;
                }
            }
            Expression::UnaryOp(unary) => {
                if let Expression::Identifier(ident) = &*unary.operand {
                    match unary.operator {
                        UnaryOpType::PrefixIncrement => {
                            instructions.push(BytecodeOp::Increment(Name::from(&ident.name)));
                            return true;
                        }
                        UnaryOpType::PrefixDecrement => {
                            instructions.push(BytecodeOp::Decrement(Name::from(&ident.name)));
                            return true;
                        }
                        _ => {}
                    }
                }
                if is_assignable_path(&unary.operand) {
                    let delta = match unary.operator {
                        UnaryOpType::PrefixIncrement => 1,
                        UnaryOpType::PrefixDecrement => -1,
                        _ => return false,
                    };
                    emit_assignable_step(self, &unary.operand, delta, instructions);
                    return true;
                }
            }
            _ => {}
        }
        false
    }

    fn compile_for(&mut self, for_stmt: &For, instructions: &mut Vec<BytecodeOp>) {
        // Init
        if let Some(init) = &for_stmt.init {
            self.compile_statement(init, instructions);
        }

        // Counted-loop fusion: if both
        //   - condition is  <ident> <cmp> <int-const>
        //   - increment is  i++ / i-- / ++i / --i on the same identifier
        // then emit the specialized do-while-ish shape with ForLoopStep at
        // the bottom, dropping per-iter overhead from 3 ops (Increment,
        // JumpIfLocalCmpConstFalse, Jump) to 1 op (ForLoopStep).
        if let Some(condition) = &for_stmt.condition {
            if let (Some((cond_name, c, cmp)), Some(increment)) =
                (Self::match_local_cmp_const(condition), for_stmt.increment.as_deref())
            {
                if let Some((inc_name, step)) = Self::match_inc_dec_identifier(increment) {
                    if cond_name == inc_name {
                        self.compile_for_counted(
                            &cond_name, c, cmp, step, &for_stmt.body, instructions,
                        );
                        return;
                    }
                }
            }
        }

        // Fallback: the generic peephole'd shape.
        let loop_start = instructions.len();

        if let Some(condition) = &for_stmt.condition {
            let jump_false_idx = if let Some((name, c, cmp)) =
                Self::match_local_cmp_const(condition)
            {
                let idx = instructions.len();
                instructions.push(BytecodeOp::JumpIfLocalCmpConstFalse(Name::from(name), c, cmp, 0));
                idx
            } else {
                self.compile_expression(condition, instructions);
                let idx = instructions.len();
                instructions.push(BytecodeOp::JumpIfFalse(0));
                idx
            };

            self.loop_stack.push((
            Vec::new(),
            Vec::new(),
            true,
            self.tag_pair_stack.len(),
            self.finally_stack.len(),
        ));

            for s in &for_stmt.body {
                self.compile_statement(s, instructions);
            }

            let continue_target = instructions.len();

            if let Some(increment) = &for_stmt.increment {
                if !self.try_emit_inc_dec_statement(increment, instructions) {
                    self.compile_expression(increment, instructions);
                    instructions.push(BytecodeOp::Pop);
                }
            }

            instructions.push(BytecodeOp::Jump(loop_start));

            let loop_end = instructions.len();
            match &mut instructions[jump_false_idx] {
                BytecodeOp::JumpIfFalse(off) => *off = loop_end,
                BytecodeOp::JumpIfLocalCmpConstFalse(_, _, _, off) => *off = loop_end,
                _ => unreachable!("compile_for exit jump slot has unexpected op"),
            }

            let (break_indices, continue_indices, _, _, _) = self.loop_stack.pop().unwrap();
            for idx in break_indices {
                instructions[idx] = BytecodeOp::Jump(loop_end);
            }
            for idx in continue_indices {
                instructions[idx] = BytecodeOp::Jump(continue_target);
            }
        }
    }

    /// Emit the counted-for-loop shape using ForLoopStep.
    /// The variable `name` must match between condition and increment.
    fn compile_for_counted(
        &mut self,
        name: &str,
        limit: i64,
        cmp: CmpOp,
        step: i64,
        body: &[Statement],
        instructions: &mut Vec<BytecodeOp>,
    ) {
        // Initial check: if the condition is already false at entry, skip
        // the loop entirely. Emits one op; the target is patched to loop_end.
        let entry_check_idx = instructions.len();
        instructions.push(BytecodeOp::JumpIfLocalCmpConstFalse(Name::intern(name), limit, cmp, 0,
        ));

        let body_start = instructions.len();

        self.loop_stack.push((
            Vec::new(),
            Vec::new(),
            true,
            self.tag_pair_stack.len(),
            self.finally_stack.len(),
        ));

        for s in body {
            self.compile_statement(s, instructions);
        }

        // continue target = the step — continue runs the step, then re-tests.
        let continue_target = instructions.len();
        instructions.push(BytecodeOp::ForLoopStep(Name::intern(name), limit, cmp, step, body_start,
        ));

        let loop_end = instructions.len();

        // Patch the entry-check to exit to loop_end if condition initially false.
        if let BytecodeOp::JumpIfLocalCmpConstFalse(_, _, _, off) =
            &mut instructions[entry_check_idx]
        {
            *off = loop_end;
        }

        let (break_indices, continue_indices, _, _, _) = self.loop_stack.pop().unwrap();
        for idx in break_indices {
            instructions[idx] = BytecodeOp::Jump(loop_end);
        }
        for idx in continue_indices {
            instructions[idx] = BytecodeOp::Jump(continue_target);
        }
    }

    /// `loop file="…" item="line"` — pump a VFS line cursor instead of
    /// iterating a materialised array (GH #367).
    ///
    /// Synthesises this and compiles it through the ordinary statement path:
    ///
    /// ```text
    ///   __filehandle_N = __cfloop_file_open(path, startLine, endLine);
    ///   try {
    ///       while (true) {
    ///           __fileline_N = __cfloop_file_next(__filehandle_N);
    ///           if (isNull(__fileline_N)) { break; }
    ///           <loop variable> = __fileline_N;
    ///           <body>
    ///       }
    ///   } finally {
    ///       __cfloop_file_close(__filehandle_N);
    ///   }
    /// ```
    ///
    /// The `try`/`finally` is the whole reason this builds AST rather than
    /// emitting the loop directly: a cursor holds an OS file descriptor, and
    /// the body can leave through `return`, `break`, or a thrown exception as
    /// well as by running out. Hand-emitted bytecode covered the first two and
    /// leaked the descriptor on the third; `finally` covers all of them through
    /// the same machinery that already stops `break` skipping a
    /// `transaction {}` rollback (GH #308). The leak was not academic — a
    /// function that returns from inside the loop leaks one descriptor per
    /// call, so a few thousand calls in one request exhaust a default 1024-fd
    /// limit and fail with EMFILE far from the cause.
    ///
    /// Null is the EOF sentinel. A line is always a string and a blank line is
    /// `""`, so an interior empty line cannot end the loop early.
    fn compile_for_in_file(
        &mut self,
        for_in: &ForIn,
        path: &Expression,
        start_line: Option<&Expression>,
        end_line: Option<&Expression>,
        instructions: &mut Vec<BytecodeOp>,
    ) {
        let loc = for_in.location;
        // `__`-prefixed and suffixed with a per-loop id so nested file loops
        // get distinct handles and neither name reaches the variables-scope
        // writeback.
        let uniq = NEXT_FILE_LOOP_ID.fetch_add(1, Ordering::Relaxed);
        let handle = format!("__filehandle_{}", uniq);
        let line = format!("__fileline_{}", uniq);

        // A dotted loop variable (`item="ctx.item"`, `item="local.v"` — the
        // lucee-spreadsheet lib does the latter) must become a MemberAccess
        // chain, not an `Identifier` whose name literally contains a dot:
        // codegen cannot resolve `ctx.item` as one opaque name and throws
        // "Variable 'item' is undefined". Same construction the tag lowering
        // uses for the other loop forms.
        let ident = |n: &str| -> Expression {
            match n.find('.') {
                None => Expression::Identifier(Identifier {
                    name: n.to_string(),
                    location: loc,
                }),
                Some(pos) => {
                    let mut expr = Expression::Identifier(Identifier {
                        name: n[..pos].to_string(),
                        location: loc,
                    });
                    for part in n[pos + 1..].split('.') {
                        expr = Expression::MemberAccess(Box::new(MemberAccess {
                            object: Box::new(expr),
                            member: part.to_string(),
                            null_safe: false,
                            location: loc,
                        }));
                    }
                    expr
                }
            }
        };
        let call = |n: &str, args: Vec<Expression>| {
            Expression::FunctionCall(Box::new(FunctionCall {
                name: Box::new(Expression::Identifier(Identifier {
                    name: n.to_string(),
                    location: loc,
                })),
                arguments: args,
                location: loc,
            }))
        };
        let assign = |target: Expression, value: Expression| {
            Statement::Expression(ExpressionStatement {
                expr: Expression::BinaryOp(Box::new(BinaryOp {
                    left: Box::new(target),
                    operator: BinaryOpType::Assign,
                    right: Box::new(value),
                    location: loc,
                })),
                location: loc,
            })
        };

        // __filehandle_N = __cfloop_file_open(path, startLine, endLine)
        //
        // The window is always passed positionally, with an absent bound sent
        // as a Null literal: 0 could not stand for "unbounded", because
        // `to=0` is a real value there and means "no lines" (Lucee runs the
        // body no times, it does not read the whole file).
        let null_lit = || Expression::Literal(Literal { value: LiteralValue::Null, location: loc });
        self.compile_statement(
            &assign(
                ident(&handle),
                call(
                    "__cfloop_file_open",
                    vec![
                        path.clone(),
                        start_line.cloned().unwrap_or_else(null_lit),
                        end_line.cloned().unwrap_or_else(null_lit),
                    ],
                ),
            ),
            instructions,
        );

        // while (true) { ... }
        let mut while_body = vec![
            assign(ident(&line), call("__cfloop_file_next", vec![ident(&handle)])),
            Statement::If(If {
                condition: call("isNull", vec![ident(&line)]),
                then_branch: vec![Statement::Break(Break { label: None, location: loc })],
                else_if: Vec::new(),
                else_branch: None,
                location: loc,
            }),
            // The loop variable is assigned exactly as the source spells it, so
            // `item="local.x"` / `item="ctx.item"` route through the ordinary
            // assignment path rather than a second implementation of it.
            assign(ident(&for_in.variable), ident(&line)),
        ];
        while_body.extend(for_in.body.iter().cloned());

        let loop_stmt = Statement::While(While {
            condition: Expression::Literal(Literal {
                value: LiteralValue::Bool(true),
                location: loc,
            }),
            body: while_body,
            location: loc,
        });

        // try { <loop> } finally { __cfloop_file_close(__filehandle_N) }
        self.compile_statement(
            &Statement::Try(Try {
                body: vec![loop_stmt],
                catches: Vec::new(),
                finally_body: Some(vec![Statement::Expression(ExpressionStatement {
                    expr: call("__cfloop_file_close", vec![ident(&handle)]),
                    location: loc,
                })]),
                location: loc,
            }),
            instructions,
        );
    }

    fn compile_for_in(&mut self, for_in: &ForIn, instructions: &mut Vec<BytecodeOp>) {
        // `loop file=` / `<cfloop file=>` lower to `for (x in
        // __cfloop_file_lines(path))` — a name no user source can produce.
        // Iterating that array is what forced the whole file to be resident
        // (GH #367), so re-shape it into a streaming pump instead. Detected
        // here, at the single point both lowerings converge on, so the tag and
        // script spellings cannot drift apart.
        if let Expression::FunctionCall(call) = &for_in.iterable {
            if let Expression::Identifier(name) = &*call.name {
                // Args 2/3, when present, are the optional line window
                // (`startLine`/`endLine`, a.k.a. `from`/`to` — GH #367); a
                // Null argument means that bound was not given.
                if name.name.eq_ignore_ascii_case("__cfloop_file_lines")
                    && (1..=3).contains(&call.arguments.len())
                {
                    self.compile_for_in_file(
                        for_in,
                        &call.arguments[0],
                        call.arguments.get(1),
                        call.arguments.get(2),
                        instructions,
                    );
                    return;
                }
            }
        }

        // Compile iterable
        self.compile_expression(&for_in.iterable, instructions);

        // GetKeys: if struct, convert to array of keys; arrays pass through unchanged
        instructions.push(BytecodeOp::GetKeys);

        // Unique per-loop temp names (so nested for-in don't collide).
        let iter_var = format!("__iter_{}", instructions.len());
        let idx_var = format!("__idx_{}", instructions.len());
        let limit_var = format!("__limit_{}", instructions.len());
        // Declare as function-locals so StoreLocal writes to locals (not __variables
        // in a CFC method context) — otherwise the loop counter never increments.
        instructions.push(BytecodeOp::DeclareLocal(Name::from(&iter_var)));
        instructions.push(BytecodeOp::DeclareLocal(Name::from(&idx_var)));
        instructions.push(BytecodeOp::DeclareLocal(Name::from(&limit_var)));
        instructions.push(BytecodeOp::StoreLocal(Name::from(&iter_var)));

        // Hoist len(iterable) out of the loop. The old codegen looked up the
        // `len` builtin and invoked it every iteration — a HashMap probe plus
        // full function-call trampoline per element. Compute once, reuse.
        instructions.push(BytecodeOp::LoadGlobal(Name::intern("len")));
        instructions.push(BytecodeOp::LoadLocal(Name::from(&iter_var)));
        instructions.push(BytecodeOp::Call(1));
        instructions.push(BytecodeOp::StoreLocal(Name::from(&limit_var)));

        // CFML arrays are 1-based, so start index at 1.
        instructions.push(BytecodeOp::Integer(1));
        instructions.push(BytecodeOp::StoreLocal(Name::from(&idx_var)));

        let loop_start = instructions.len();

        // Condition: idx <= limit  (both locals; no builtin call per iter).
        instructions.push(BytecodeOp::LoadLocal(Name::from(&idx_var)));
        instructions.push(BytecodeOp::LoadLocal(Name::from(&limit_var)));
        instructions.push(BytecodeOp::Lte);

        let jump_false_idx = instructions.len();
        instructions.push(BytecodeOp::JumpIfFalse(0));

        // Set loop variable = iterable[idx]
        instructions.push(BytecodeOp::LoadLocal(Name::from(&iter_var)));
        instructions.push(BytecodeOp::LoadLocal(Name::from(&idx_var)));
        instructions.push(BytecodeOp::GetIndex);
        // GH #351: `for ( local.X in … )` at TEMPLATE level. `local` is an
        // ordinary variable there, so the loop variable is `variables.local.X`
        // and stripping the prefix would write a bare `X` that the body's
        // `local.X` read never finds. That is exactly what broke Wheels'
        // `WheelsTest.cfc` pseudo-constructor — `for (local.method in
        // local.methods)` silently iterated with an unset `local.method`, so it
        // injected NO methods into the spec and 75 specs failed with missing
        // methods far from here. Hand the decision to the VM, as the assignment
        // form does.
        let template_local_key: Option<String> = for_in
            .variable
            .strip_prefix("local.")
            .filter(|rest| !rest.contains('.') && !self.local_is_scope())
            .map(|rest| rest.to_string());
        // Strip a leading `local.` prefix from the loop variable so it stores
        // as a simple local rather than a literal key containing a dot. A
        // subsequent `local.X` read resolves to that local via the normal
        // locals lookup.
        let loop_var_name = if let Some(rest) = for_in.variable.strip_prefix("local.") {
            rest.to_string()
        } else {
            for_in.variable.clone()
        };
        if let Some(key) = template_local_key {
            instructions.push(BytecodeOp::StoreLocalScopeKey(Name::from(&key)));
        } else if loop_var_name.contains('.') {
            // Member-path loop variable (e.g. `ctx.item`, `this.wheels.folder`).
            // Lucee/ACF/BoxLang assign the iterated value through the path each
            // iteration. Emit a struct write-back chain: load the deepest
            // parent, set the leaf property, then propagate the modified
            // struct back up to the root local.
            let segments: Vec<String> =
                loop_var_name.split('.').map(|s| s.to_string()).collect();
            // Single-level member path rooted at a bare (non-reserved)
            // identifier (`loc.route`): emit the auto-vivifying
            // StoreLocalProperty, mirroring the assignment side
            // (`loc.route = v`). The manual LoadLocal-based chain below loads
            // the root first, which throws "Variable 'loc' is undefined" when
            // the loop variable's root doesn't exist yet — but Lucee/ACF/
            // BoxLang auto-create it. (Wheels mapperSpec
            // `for (loc.route in application.wheels.routes)`.)
            if segments.len() == 2 && !is_reserved_scope_name(&segments[0]) {
                // Stack on entry: [element_value]. StoreLocalProperty pops the
                // value, auto-vivifies the root local as a struct if absent,
                // and sets the leaf.
                instructions.push(BytecodeOp::StoreLocalProperty(Name::from(&segments[0]),Name::from(&segments[1]),
                ));
            } else {
                let root = segments[0].clone();
                let leaf = segments[segments.len() - 1].clone();
                let intermediate = &segments[1..segments.len() - 1];
                // Stack on entry: [element_value]
                // Load deepest parent: root[.intermediate[0]...intermediate[n]]
                instructions.push(BytecodeOp::LoadLocal(Name::from(&root)));
                for seg in intermediate {
                    instructions.push(BytecodeOp::TryGetProperty(Name::from(&seg)));
                }
                // Stack: [element_value, deepest_parent]
                instructions.push(BytecodeOp::Swap);
                instructions.push(BytecodeOp::SetProperty(Name::from(leaf)));
                // Stack: [modified_deepest_parent]
                // Unwind: for each intermediate level (deepest -> shallowest),
                // reload its parent and SetProperty back in.
                for i in (0..intermediate.len()).rev() {
                    instructions.push(BytecodeOp::LoadLocal(Name::from(&root)));
                    for seg in &intermediate[..i] {
                        instructions.push(BytecodeOp::TryGetProperty(Name::from(&seg)));
                    }
                    instructions.push(BytecodeOp::Swap);
                    instructions.push(BytecodeOp::SetProperty(Name::from(&intermediate[i])));
                }
                // Stack: [modified_root]
                instructions.push(BytecodeOp::StoreLocal(Name::from(root)));
            }
        } else {
            instructions.push(BytecodeOp::DeclareLocal(Name::from(&loop_var_name)));
            instructions.push(BytecodeOp::StoreLocal(Name::from(loop_var_name)));
        }
        self.loop_stack.push((
            Vec::new(),
            Vec::new(),
            true,
            self.tag_pair_stack.len(),
            self.finally_stack.len(),
        ));

        for s in &for_in.body {
            self.compile_statement(s, instructions);
        }

        let continue_target = instructions.len();

        // idx++  (single Increment op, not Load+Int+Add+Store).
        instructions.push(BytecodeOp::Increment(Name::from(&idx_var)));

        instructions.push(BytecodeOp::Jump(loop_start));

        let loop_end = instructions.len();
        instructions[jump_false_idx] = BytecodeOp::JumpIfFalse(loop_end);

        let (break_indices, continue_indices, _, _, _) = self.loop_stack.pop().unwrap();
        for idx in break_indices {
            instructions[idx] = BytecodeOp::Jump(loop_end);
        }
        for idx in continue_indices {
            instructions[idx] = BytecodeOp::Jump(continue_target);
        }
    }

    fn compile_while(&mut self, while_stmt: &While, instructions: &mut Vec<BytecodeOp>) {
        // Counted-loop fusion: if the condition is `<ident> <cmp> <int-const>`
        // and the last body statement advances the same identifier by a
        // constant step (i++, i+=K, i = i+K, etc.), hoist the stride out and
        // emit the same ForLoopStep-based shape used by compile_for_counted.
        // Skip when a top-level `continue` is present — the hoist would
        // change whether `continue` runs the stride.
        if let Some((cond_name, c, cmp)) = Self::match_local_cmp_const(&while_stmt.condition) {
            if let Some(last) = while_stmt.body.last() {
                if let Some((stride_name, step)) = Self::match_stride_statement(last) {
                    if cond_name.eq_ignore_ascii_case(&stride_name)
                        && !Self::body_has_top_level_continue(&while_stmt.body)
                    {
                        let body_without_stride =
                            &while_stmt.body[..while_stmt.body.len() - 1];
                        self.compile_for_counted(
                            &cond_name, c, cmp, step, body_without_stride, instructions,
                        );
                        return;
                    }
                }
            }
        }

        let loop_start = instructions.len();

        let jump_false_idx = self.emit_cond_jump_false(&while_stmt.condition, instructions);

        self.loop_stack.push((
            Vec::new(),
            Vec::new(),
            true,
            self.tag_pair_stack.len(),
            self.finally_stack.len(),
        ));

        for s in &while_stmt.body {
            self.compile_statement(s, instructions);
        }

        instructions.push(BytecodeOp::Jump(loop_start));

        let loop_end = instructions.len();
        Self::patch_cond_jump_target(instructions, jump_false_idx, loop_end);

        let (break_indices, continue_indices, _, _, _) = self.loop_stack.pop().unwrap();
        for idx in break_indices {
            instructions[idx] = BytecodeOp::Jump(loop_end);
        }
        for idx in continue_indices {
            instructions[idx] = BytecodeOp::Jump(loop_start);
        }
    }

    fn compile_do(&mut self, do_stmt: &Do, instructions: &mut Vec<BytecodeOp>) {
        // Counted-do-while fusion: same shape as compile_while but no entry
        // check — do-while always runs the body at least once.
        if let Some((cond_name, c, cmp)) = Self::match_local_cmp_const(&do_stmt.condition) {
            if let Some(last) = do_stmt.body.last() {
                if let Some((stride_name, step)) = Self::match_stride_statement(last) {
                    if cond_name.eq_ignore_ascii_case(&stride_name)
                        && !Self::body_has_top_level_continue(&do_stmt.body)
                    {
                        let body_without_stride = &do_stmt.body[..do_stmt.body.len() - 1];
                        self.compile_do_counted(
                            &cond_name, c, cmp, step, body_without_stride, instructions,
                        );
                        return;
                    }
                }
            }
        }

        let loop_start = instructions.len();

        self.loop_stack.push((
            Vec::new(),
            Vec::new(),
            true,
            self.tag_pair_stack.len(),
            self.finally_stack.len(),
        ));

        for s in &do_stmt.body {
            self.compile_statement(s, instructions);
        }

        let continue_target = instructions.len();

        self.compile_expression(&do_stmt.condition, instructions);
        instructions.push(BytecodeOp::JumpIfTrue(loop_start));

        let loop_end = instructions.len();

        let (break_indices, continue_indices, _, _, _) = self.loop_stack.pop().unwrap();
        for idx in break_indices {
            instructions[idx] = BytecodeOp::Jump(loop_end);
        }
        for idx in continue_indices {
            instructions[idx] = BytecodeOp::Jump(continue_target);
        }
    }

    /// Counted-do-while fused shape: no entry check (body always runs once),
    /// stride folded into the bottom ForLoopStep.
    fn compile_do_counted(
        &mut self,
        name: &str,
        limit: i64,
        cmp: CmpOp,
        step: i64,
        body: &[Statement],
        instructions: &mut Vec<BytecodeOp>,
    ) {
        let body_start = instructions.len();

        self.loop_stack.push((
            Vec::new(),
            Vec::new(),
            true,
            self.tag_pair_stack.len(),
            self.finally_stack.len(),
        ));

        for s in body {
            self.compile_statement(s, instructions);
        }

        let continue_target = instructions.len();
        instructions.push(BytecodeOp::ForLoopStep(Name::intern(name), limit, cmp, step, body_start,
        ));

        let loop_end = instructions.len();

        let (break_indices, continue_indices, _, _, _) = self.loop_stack.pop().unwrap();
        for idx in break_indices {
            instructions[idx] = BytecodeOp::Jump(loop_end);
        }
        for idx in continue_indices {
            instructions[idx] = BytecodeOp::Jump(continue_target);
        }
    }

    fn compile_switch(&mut self, switch_stmt: &Switch, instructions: &mut Vec<BytecodeOp>) {
        // Evaluate switch expression and store
        self.compile_expression(&switch_stmt.expression, instructions);
        let switch_var = format!("__switch_{}", instructions.len());
        instructions.push(BytecodeOp::StoreLocal(Name::from(&switch_var)));

        self.loop_stack.push((
            Vec::new(),
            Vec::new(),
            false,
            self.tag_pair_stack.len(),
            self.finally_stack.len(),
        )); // break support (not a loop)

        // CFML/Lucee `switch` is C-style: matching a case transfers control to
        // its body, and execution then FALLS THROUGH into subsequent case bodies
        // until an explicit `break`. This is why stacked empty labels
        // (`case "model": case "id": { … }`) share the following body, and why a
        // non-empty case without a `break` continues into the next case.
        //
        // To model that we split the switch into two sections: a dispatch table
        // that compares the switch value against each case and jumps to the
        // matching body, followed by the case bodies emitted sequentially so
        // they fall through naturally.

        // --- Dispatch section ---
        let mut dispatch_jumps: Vec<usize> = Vec::with_capacity(switch_stmt.cases.len());
        for case in &switch_stmt.cases {
            // Compare switch value to case value(s); OR multiple values together.
            for (i, val) in case.values.iter().enumerate() {
                instructions.push(BytecodeOp::LoadLocal(Name::from(&switch_var)));
                self.compile_expression(val, instructions);
                instructions.push(BytecodeOp::Eq);

                if i > 0 {
                    instructions.push(BytecodeOp::Or);
                }
            }

            // On match, jump to this case's body (patched below).
            dispatch_jumps.push(instructions.len());
            instructions.push(BytecodeOp::JumpIfTrue(0));
        }

        // No case matched -> jump to the default body (or end if none). Patched
        // once the default position is known.
        let no_match_jump = instructions.len();
        instructions.push(BytecodeOp::Jump(0));

        // --- Case bodies (sequential, fall-through) ---
        for (ci, case) in switch_stmt.cases.iter().enumerate() {
            let body_start = instructions.len();
            instructions[dispatch_jumps[ci]] = BytecodeOp::JumpIfTrue(body_start);
            for s in &case.body {
                self.compile_statement(s, instructions);
            }
            // No implicit jump-to-end: fall through into the next case body.
        }

        // Default body is emitted last; the textually-last case falls through
        // into it when it lacks a `break`, matching Lucee.
        let default_start = instructions.len();
        instructions[no_match_jump] = BytecodeOp::Jump(default_start);
        if let Some(default) = &switch_stmt.default_case {
            for s in default {
                self.compile_statement(s, instructions);
            }
        }

        let end_pos = instructions.len();

        // Patch break statements
        let (break_indices, _, _, _, _) = self.loop_stack.pop().unwrap();
        for idx in break_indices {
            instructions[idx] = BytecodeOp::Jump(end_pos);
        }
    }

    fn compile_try(&mut self, try_stmt: &Try, instructions: &mut Vec<BytecodeOp>) {
        // Special case: `try { body } finally { ... }` with NO catch clauses.
        // CFML (and the `lock {}` desugaring — `try { body } finally { unlock }`)
        // require the finally to run AND the exception to re-propagate. The
        // generic catch-handler shape below routes every exception to
        // catch_start, runs the finally, and then continues — which *swallows*
        // the exception (and leaves the thrown error on the operand stack).
        // Emit the finally on both the normal and exception paths, and re-raise
        // on the exception path.
        if try_stmt.catches.is_empty() {
            if let Some(ref finally_body) = try_stmt.finally_body {
                let try_start_idx = instructions.len();
                instructions.push(BytecodeOp::TryStart(0)); // placeholder -> exception handler

                // While compiling the body, a `return` must run this finally
                // inline before exiting (the runtime Return op won't).
                self.finally_stack.push(finally_body.clone());
                for s in &try_stmt.body {
                    self.compile_statement(s, instructions);
                }
                self.finally_stack.pop();
                instructions.push(BytecodeOp::TryEnd);

                // Normal-path finally, then jump over the exception handler.
                for s in finally_body {
                    self.compile_statement(s, instructions);
                }
                let jump_over_handler = instructions.len();
                instructions.push(BytecodeOp::Jump(0)); // -> end

                // Exception handler: the in-flight error is on the operand stack
                // (pushed by Throw/Rethrow). Run the finally, then re-raise.
                let handler_start = instructions.len();
                instructions[try_start_idx] = BytecodeOp::TryStart(handler_start);
                instructions.push(BytecodeOp::SaveException);
                for s in finally_body {
                    self.compile_statement(s, instructions);
                }
                instructions.push(BytecodeOp::RestoreException);
                instructions.push(BytecodeOp::Rethrow);

                let end_pos = instructions.len();
                instructions[jump_over_handler] = BytecodeOp::Jump(end_pos);
                return;
            }
        }

        // TryStart points to catch handler
        let try_start_idx = instructions.len();
        instructions.push(BytecodeOp::TryStart(0)); // placeholder

        // Push the finally (if any) for the duration of the body AND catches, so
        // a `return` in either runs it inline (Return op won't) and a `rethrow`
        // in a catch runs it before propagating.
        let has_finally = try_stmt.finally_body.is_some();
        if let Some(ref finally_body) = try_stmt.finally_body {
            self.finally_stack.push(finally_body.clone());
        }

        // Try body
        for s in &try_stmt.body {
            self.compile_statement(s, instructions);
        }
        instructions.push(BytecodeOp::TryEnd);

        // Jump over catch blocks
        let jump_over_catch = instructions.len();
        instructions.push(BytecodeOp::Jump(0));

        // Catch handler. On entry the thrown exception value is on top of the
        // operand stack. Walk the catch clauses in source order, runtime-testing
        // each clause's declared type against the exception's `type`; the FIRST
        // matching clause runs and then jumps clear of the remaining clauses.
        // (Previously every clause body ran unconditionally and the declared
        // type was ignored entirely — both `catch (X)` and `catch (any)` fired.)
        let catch_start = instructions.len();
        instructions[try_start_idx] = BytecodeOp::TryStart(catch_start);

        let mut jumps_to_end = Vec::new();
        for catch in &try_stmt.catches {
            // `catch (e)` with no declared type behaves like `catch (any e)`.
            let catch_type = catch.var_type.clone().unwrap_or_else(|| "any".to_string());
            // Peek the exception (leaves it on the stack), push the match bool.
            instructions.push(BytecodeOp::CatchMatch(Name::from(catch_type)));
            let jump_if_no_match = instructions.len();
            instructions.push(BytecodeOp::JumpIfFalse(0)); // -> next clause's test
            // Matched: bind the exception to the catch variable (consumes it).
            // DeclareLocal first: the catch variable is FRAME-LOCAL on Lucee —
            // it must never fall into the classic-localmode unscoped-store
            // cascade and land in the component/tag `variables` scope (measured
            // on Lucee 7: `catch (any e)` inside a function leaves a same-named
            // `variables.e` untouched; RustCFML clobbered it — surfaced when
            // custom-tag frames gained a live `__variables` scope and a
            // harness `catch (any e)` started overwriting a test's `e`).
            instructions.push(BytecodeOp::DeclareLocal(Name::from(&catch.var_name)));
            instructions.push(BytecodeOp::StoreLocal(Name::from(&catch.var_name)));
            // GH #244: track the caught-exception variable so a `rethrow` in this
            // body re-raises THIS clause's exception even if a nested try/catch
            // clobbered `last_exception`.
            self.catch_var_stack.push(catch.var_name.clone());
            for s in &catch.body {
                self.compile_statement(s, instructions);
            }
            self.catch_var_stack.pop();
            // Skip the remaining clauses and land on the shared finally/end.
            let j = instructions.len();
            instructions.push(BytecodeOp::Jump(0));
            jumps_to_end.push(j);
            // A non-match falls through to the next clause's test.
            let next = instructions.len();
            instructions[jump_if_no_match] = BytecodeOp::JumpIfFalse(next);
        }

        // A `return`/`rethrow` inside a catch BODY must run this try's finally
        // inline, so the pop happens only after the clause bodies are compiled.
        if has_finally {
            self.finally_stack.pop();
        }

        // No clause matched the thrown type: drop the exception value, run this
        // try's finally inline (finally_stack already popped, so a return/rethrow
        // in the finally targets the ENCLOSING handler), then re-raise so an
        // outer try sees it.
        instructions.push(BytecodeOp::Pop);
        if let Some(finally_body) = &try_stmt.finally_body {
            instructions.push(BytecodeOp::SaveException);
            for s in finally_body {
                self.compile_statement(s, instructions);
            }
            instructions.push(BytecodeOp::RestoreException);
        }
        instructions.push(BytecodeOp::Rethrow);

        let end_pos = instructions.len();
        instructions[jump_over_catch] = BytecodeOp::Jump(end_pos);
        for j in jumps_to_end {
            instructions[j] = BytecodeOp::Jump(end_pos);
        }

        // Finally (normal completion + caught path)
        if let Some(finally_body) = &try_stmt.finally_body {
            for s in finally_body {
                self.compile_statement(s, instructions);
            }
        }
    }

    /// Compile a function declaration, emitting `DefineFunction` + `StoreLocal`
    /// so the function is bound in the current scope. Returns the function's
    /// process-stable `global_id` so a caller (e.g. compile_component) can
    /// re-emit `DefineFunction` to obtain a fresh reference WITHOUT a
    /// `LoadLocal(name)` round-trip — essential when the method name is a
    /// reserved scope word (`local`, `arguments`, …) where `LoadLocal` would
    /// load the scope itself rather than the just-defined function.
    fn compile_function_decl(&mut self, func: &Function, instructions: &mut Vec<BytecodeOp>) -> usize {
        // Compile the function body into a separate BytecodeFunction
        let mut func_instructions = Vec::new();

        self.function_depth += 1;
        // GH #351: a declared function body owns a `local` scope.
        self.local_scope_depth += 1;

        // Save/restore the surrounding function's declared localMode so nested
        // closures inherit *this* function's mode rather than something further
        // up the stack.
        let declared_mode = metadata_declared_local_mode(&func.metadata);
        let prev_fn_local_mode = self.current_fn_local_mode;
        self.current_fn_local_mode = declared_mode.or(prev_fn_local_mode);

        // A nested function/closure is its own control-flow boundary: a `return`
        // in its body must run only ITS finallys (none yet), never the enclosing
        // function's, and a `break`/`continue` must not target an enclosing loop.
        // Save/clear both stacks for the body, then restore. Without this, a
        // closure defined inside `lock {}` / `try{}finally{}` would emit the
        // enclosing finally inline into the closure body (the WireBox
        // `produceMetadataUDF` regression).
        let saved_finally = std::mem::take(&mut self.finally_stack);
        let saved_loops = std::mem::take(&mut self.loop_stack);
        let saved_tag_pairs = std::mem::take(&mut self.tag_pair_stack);
        let saved_catch_vars = std::mem::take(&mut self.catch_var_stack);

        // Emit default parameter value preamble:
        // For each param the caller did NOT supply, evaluate its default and seed
        // both the local and the `arguments` key. Presence is tested against the
        // `arguments` scope (JumpIfArgPresent) rather than `LoadLocal + IsNull`:
        // the VM no longer pre-seeds an omitted param as Null, so a default that
        // references a same-named outer variable (`function f(x = x)`) resolves to
        // that outer variable instead of the param's own empty slot (GitHub #240).
        for (idx, param) in func.params.iter().enumerate() {
            if let Some(ref default_expr) = param.default {
                let jump_idx = func_instructions.len();
                func_instructions.push(BytecodeOp::JumpIfArgPresent(Name::from(&param.name), 0)); // placeholder
                // Set the local variable
                self.compile_expression(default_expr, &mut func_instructions);
                // Seed the local AND the `arguments` key from the default, WITHOUT reading
                // the parameter back by bare name. A `LoadLocal(param.name)` read-back is
                // wrong for a parameter named after a built-in scope: since GH #312 a bare
                // scope name always resolves to the SCOPE, so `function f( cookie = "D" )`
                // seeded `arguments.cookie` with the live cookie scope instead of "D".
                // `Dup` keeps the freshly-evaluated value on the stack for
                // `SeedArgumentKey`, which consumes it: the local is stored by name
                // (so slot behaviour is untouched) and the frame's OWN `arguments`
                // scope gets the same value. Emitting `LoadLocal("arguments")` here
                // is what used to force the whole function onto the eager path.
                func_instructions.push(BytecodeOp::Dup);
                func_instructions.push(BytecodeOp::StoreLocal(Name::from(&param.name)));
                func_instructions.push(BytecodeOp::SeedArgumentKey(Name::from(&param.name)));
                // A DEFAULT is type-checked exactly like a supplied argument
                // (Lucee: `function f( numeric n = "abc" )` throws on `f()`).
                // A supplied argument is checked by the VM at bind time, which
                // is why this only guards the default-applied branch — the op
                // sits INSIDE the JumpIfArgPresent-skipped region.
                if declared_type_is_checkable(param.param_type.as_deref()) {
                    func_instructions.push(BytecodeOp::ValidateParamType(idx));
                }
                func_instructions[jump_idx] =
                    BytecodeOp::JumpIfArgPresent(Name::from(&param.name), func_instructions.len());
            }
        }

        for s in &func.body {
            self.compile_statement(s, &mut func_instructions);
        }

        // Ensure function returns null if no explicit return
        func_instructions.push(BytecodeOp::Null);
        func_instructions.push(BytecodeOp::Return);

        self.function_depth -= 1;
        self.local_scope_depth -= 1;
        self.current_fn_local_mode = prev_fn_local_mode;
        self.finally_stack = saved_finally;
        self.loop_stack = saved_loops;
        self.tag_pair_stack = saved_tag_pairs;
        self.catch_var_stack = saved_catch_vars;

        let bc_func = BytecodeFunction {
            name: func.name.clone(),
            params: func.params.iter().map(|p| p.name.clone()).collect(),
            param_keys: Default::default(),
                    args_needed: Default::default(),
                    args_never_escapes: Default::default(),
                    params_marker: Default::default(),
                    cfc_body: Default::default(),
            required_params: func.params.iter().map(|p| p.required).collect(),
            has_default: func.params.iter().map(|p| p.default.is_some()).collect(),
            instructions: func_instructions,
            source_file: self.source_file.clone(),
            global_id: next_global_fn_id(),
            declared_local_mode: declared_mode,
            param_types: func.params.iter().map(|p| p.param_type.clone()).collect(),
            // A return type reaches us two ways: as the prefix form
            // (`numeric function f()`, which the parser puts in `return_type`,
            // and which `<cffunction returntype=…>` also lowers to) or as a
            // post-paren attribute (`function f() returntype="numeric" {}`,
            // which lands in `metadata`). Only the first was carried, so the
            // attribute form was invisible to both getMetadata() and the §29
            // return-type check.
            return_type: func.return_type.clone().or_else(|| {
                func.metadata
                    .iter()
                    .find(|(k, _)| k.eq_ignore_ascii_case("returntype"))
                    .map(|(_, v)| v.clone())
            }),
            param_annotations: func.params.iter().map(|p| p.annotations.clone()).collect(),
            is_component_method: self.in_component_method,
            access: match func.access {
                AccessModifier::Private => cfml_common::dynamic::CfmlAccess::Private,
                AccessModifier::Package => cfml_common::dynamic::CfmlAccess::Package,
                AccessModifier::Remote => cfml_common::dynamic::CfmlAccess::Remote,
                AccessModifier::Public => cfml_common::dynamic::CfmlAccess::Public,
            },
            // `returntype` is dropped from the free-form attribute list because it
            // is now carried in `return_type` above, and the flat attribute keys
            // land in getMetadata() alongside the canonical `returnType` — one
            // declaration must not surface as two keys.
            metadata: func
                .metadata
                .iter()
                .filter(|(k, _)| !k.eq_ignore_ascii_case("returntype"))
                .cloned()
                .collect(),
            is_generated_accessor: false,
            output_suppressed: false,
            is_template_frame: false,
            chain_tier: 0,
            slot_names: Vec::new(),
        };

        let global_id = bc_func.global_id as usize;
        self.push_function(bc_func);

        // Define the function in current scope. The op carries the function's
        // process-stable global_id, resolved by the VM through its registry.
        instructions.push(BytecodeOp::DefineFunction(global_id));
        instructions.push(BytecodeOp::StoreLocal(Name::from(&func.name)));
        global_id
    }

    fn compile_interface(&mut self, interface: &Interface, instructions: &mut Vec<BytecodeOp>) {
        let mut prop_count = 0;

        // __is_interface marker
        instructions.push(BytecodeOp::String("__is_interface".to_string()));
        instructions.push(BytecodeOp::True);
        prop_count += 1;

        // __name
        instructions.push(BytecodeOp::String("__name".to_string()));
        instructions.push(BytecodeOp::String(interface.name.clone()));
        prop_count += 1;

        // __extends array (interfaces can extend multiple parents)
        if !interface.extends.is_empty() {
            instructions.push(BytecodeOp::String("__extends".to_string()));
            for parent in &interface.extends {
                instructions.push(BytecodeOp::String(parent.clone()));
            }
            instructions.push(BytecodeOp::BuildArray(interface.extends.len()));
            prop_count += 1;
        }

        // __methods struct: each entry mirrors the component `functions`
        // metadata shape ({ name, access, returntype, parameters:[{name, type,
        // required, ...annotations}] }) so getComponentMetadata(iface).functions
        // can surface an interface's declared signatures (issue #205 — MockBox
        // createStub(implements=) reads these to generate stub methods).
        // `parameters` is ALWAYS emitted (MockBox iterates it unconditionally).
        // resolve_interface_methods only reads the keys of this struct, so the
        // richer values are safe.
        if !interface.functions.is_empty() {
            instructions.push(BytecodeOp::String("__methods".to_string()));
            for func in &interface.functions {
                let method_key = func.name.to_lowercase();
                instructions.push(BytecodeOp::String(method_key));

                let mut method_prop_count = 0;

                // name
                instructions.push(BytecodeOp::String("name".to_string()));
                instructions.push(BytecodeOp::String(func.name.clone()));
                method_prop_count += 1;

                // access
                let access_str = match func.access {
                    AccessModifier::Public => "public",
                    AccessModifier::Private => "private",
                    AccessModifier::Package => "package",
                    AccessModifier::Remote => "remote",
                };
                instructions.push(BytecodeOp::String("access".to_string()));
                instructions.push(BytecodeOp::String(access_str.to_string()));
                method_prop_count += 1;

                // returntype (default "any", matching Lucee/ACF)
                instructions.push(BytecodeOp::String("returntype".to_string()));
                instructions.push(BytecodeOp::String(
                    func.return_type.clone().unwrap_or_else(|| "any".to_string()),
                ));
                method_prop_count += 1;

                // parameters: full param structs, always present
                instructions.push(BytecodeOp::String("parameters".to_string()));
                for param in &func.params {
                    let mut param_prop_count = 0;

                    instructions.push(BytecodeOp::String("name".to_string()));
                    instructions.push(BytecodeOp::String(param.name.clone()));
                    param_prop_count += 1;

                    if let Some(ref t) = param.param_type {
                        instructions.push(BytecodeOp::String("type".to_string()));
                        instructions.push(BytecodeOp::String(t.clone()));
                        param_prop_count += 1;
                    }

                    instructions.push(BytecodeOp::String("required".to_string()));
                    instructions.push(if param.required {
                        BytecodeOp::True
                    } else {
                        BytecodeOp::False
                    });
                    param_prop_count += 1;

                    // Javadoc-style param annotations (e.g. WireBox @x.inject)
                    for (k, v) in &param.annotations {
                        instructions.push(BytecodeOp::String(k.clone()));
                        instructions.push(BytecodeOp::String(v.clone()));
                        param_prop_count += 1;
                    }

                    instructions.push(BytecodeOp::BuildStruct(param_prop_count));
                }
                instructions.push(BytecodeOp::BuildArray(func.params.len()));
                method_prop_count += 1;

                instructions.push(BytecodeOp::BuildStruct(method_prop_count));
            }
            instructions.push(BytecodeOp::BuildStruct(interface.functions.len()));
            prop_count += 1;
        }

        // __metadata
        if !interface.metadata.is_empty() {
            instructions.push(BytecodeOp::String("__metadata".to_string()));
            for (k, v) in &interface.metadata {
                instructions.push(BytecodeOp::String(k.clone()));
                instructions.push(BytecodeOp::String(v.clone()));
            }
            instructions.push(BytecodeOp::BuildStruct(interface.metadata.len()));
            prop_count += 1;
        }

        // Build the interface struct
        instructions.push(BytecodeOp::BuildStruct(prop_count));

        // Store in local and global scope (same as component)
        instructions.push(BytecodeOp::StoreLocal(Name::from(&interface.name)));
        instructions.push(BytecodeOp::LoadLocal(Name::from(&interface.name)));
        instructions.push(BytecodeOp::StoreGlobal(Name::from(&interface.name)));
    }

    fn compile_component(&mut self, component: &Component, instructions: &mut Vec<BytecodeOp>) {
        // Build the component as a struct containing:
        // 1. Metadata keys (__name, __extends, __implements, __metadata)
        // 2. __variables scope with property defaults
        // 3. Compiled methods as function references
        let mut prop_count = 0;

        // Add __name metadata
        instructions.push(BytecodeOp::String("__name".to_string()));
        instructions.push(BytecodeOp::String(component.name.clone()));
        prop_count += 1;

        // Add __extends if component extends another
        if let Some(ref ext) = component.extends {
            instructions.push(BytecodeOp::String("__extends".to_string()));
            instructions.push(BytecodeOp::String(ext.clone()));
            prop_count += 1;
        }

        // Add __implements if component implements interfaces
        if !component.implements.is_empty() {
            instructions.push(BytecodeOp::String("__implements".to_string()));
            for iface_name in &component.implements {
                instructions.push(BytecodeOp::String(iface_name.clone()));
            }
            instructions.push(BytecodeOp::BuildArray(component.implements.len()));
            prop_count += 1;
        }

        // Add __metadata sub-struct if component has metadata attributes
        if !component.metadata.is_empty() {
            instructions.push(BytecodeOp::String("__metadata".to_string()));
            for (k, v) in &component.metadata {
                instructions.push(BytecodeOp::String(k.clone()));
                instructions.push(BytecodeOp::String(v.clone()));
            }
            instructions.push(BytecodeOp::BuildStruct(component.metadata.len()));
            prop_count += 1;
        }

        // Add __variables scope for component properties (needed for accessors)
        // Include property defaults here
        if component.accessors || !component.properties.is_empty() {
            instructions.push(BytecodeOp::String("__variables".to_string()));
            // Build __variables struct with property defaults. Only properties
            // that declare a `default` are seeded here — an unset property is
            // NOT a key in the variables scope until assigned (Lucee/ACF: a
            // declared-but-unset `property name="x"` makes `structKeyExists(
            // variables,"x")` false, and getters return null via the
            // missing-key fallback). Seeding them as Null instead put a
            // null-valued key into the scope, which then leaked through
            // `variables.filter(...)`/structEach as an undefined closure arg —
            // ColdBox RequestContext.getMemento crashed on it.
            let mut vars_count = 0;
            for prop in &component.properties {
                if let Some(default) = &prop.default {
                    instructions.push(BytecodeOp::String(prop.name.clone()));
                    self.compile_expression(default, instructions);
                    vars_count += 1;
                }
            }
            instructions.push(BytecodeOp::BuildStruct(vars_count));
            prop_count += 1;
        }

        // Runtime marker for the implicit accessor constructor: a component with
        // `accessors=true` and no explicit init() maps NAMED constructor args (and
        // an argumentCollection spread) onto its declared properties (Lucee/ACF
        // parity). `__`-prefixed keys are filtered out of all struct iteration /
        // serialization, so this never leaks into user-visible output.
        if component.accessors {
            instructions.push(BytecodeOp::String("__accessors".to_string()));
            instructions.push(BytecodeOp::True);
            prop_count += 1;
        }

        // Build the base struct
        instructions.push(BytecodeOp::BuildStruct(prop_count));

        // Store as a component template in local scope first
        instructions.push(BytecodeOp::StoreLocal(Name::from(&component.name)));

        // Generate accessor methods if accessors="true" (BEFORE storing globally)
        if component.accessors {
            for prop in &component.properties {
                // Generate getter: getPropertyName()
                let getter_name = format!("get{}", capitalize_first(&prop.name));
                // Read the property backing field from the VARIABLES scope, not
                // `this`. The property value lives in __variables (defaults +
                // setter writes + `variables.prop = ...` assignments all land
                // there). Reading `this.prop` works only by GetProperty's
                // fallback-to-__variables, which BREAKS when a same-named public
                // method occupies the top-level `this.prop` key (e.g. a CFC with
                // both `property name="foo"` and a method `foo()`): the function
                // shadows the property and getFoo() reads back the method instead
                // of the value. Lucee/ACF getters read the variables backing, so
                // this both fixes the collision and matches the reference engines.
                let getter_func = BytecodeFunction {
                    name: getter_name.clone(),
                    params: Vec::new(),
                    param_keys: Default::default(),
                    args_needed: Default::default(),
                    args_never_escapes: Default::default(),
                    params_marker: Default::default(),
                    cfc_body: Default::default(),
                    required_params: Vec::new(),
                    has_default: Vec::new(),
                    instructions: vec![
                        BytecodeOp::LoadLocal(Name::intern("variables")),
                        BytecodeOp::TryGetProperty(Name::from(&prop.name)),
                        BytecodeOp::Return,
                    ],
                    source_file: self.source_file.clone(),
                    global_id: next_global_fn_id(),
                    declared_local_mode: None,
                    param_types: Vec::new(),
                    return_type: prop.prop_type.clone(),
                    param_annotations: Vec::new(),
                    is_component_method: true,
                    access: cfml_common::dynamic::CfmlAccess::Public,
                    metadata: Vec::new(),
                    // Declared for metadata, NOT enforced — see the field docs.
                    is_generated_accessor: true,
                    output_suppressed: false,
                    is_template_frame: false,
            chain_tier: 0,
            slot_names: Vec::new(),
                };
                self.push_function(getter_func);
                let getter_gid = self.program.functions.last().unwrap().global_id as usize;
                instructions.push(BytecodeOp::DefineFunction(getter_gid));
                // Stack: [getter_func]

                // Add getter to component: component[getter_name] = getter_func
                // Stack: [getter_func]
                // Load component: [getter_func, component]
                // Swap: [component, getter_func]
                // SetProperty(getter_name): sets component.getter_name = getter_func, stack is [component]
                // StoreLocal: []
                instructions.push(BytecodeOp::LoadLocal(Name::from(&component.name)));
                instructions.push(BytecodeOp::Swap);
                instructions.push(BytecodeOp::SetProperty(Name::from(&getter_name)));
                instructions.push(BytecodeOp::StoreLocal(Name::from(&component.name)));

                // Generate setter: setPropertyName(value)
                // Set the property directly on this struct and __variables
                let setter_name = format!("set{}", capitalize_first(&prop.name));
                // Collision: a CFC may declare both `property name="x"` and a method
                // `x()`. The method occupies the top-level `this.x` key; writing the
                // property value to `this.x` would clobber it, making x() uncallable.
                // Lucee/ACF keep the method callable (getX/setX operate on the
                // `variables` backing). So when a same-named method exists, the setter
                // writes ONLY the `__variables` backing and leaves `this.x` (the method)
                // untouched. The getter already reads from `variables` (see above).
                let collides_with_method = component
                    .functions
                    .iter()
                    .any(|f| f.name.eq_ignore_ascii_case(&prop.name));
                let setter_instructions = if collides_with_method {
                    vec![
                        // Set on __variables only: this.__variables.name = value
                        BytecodeOp::LoadLocal(Name::intern("this")),
                        BytecodeOp::TryGetProperty(Name::intern("__variables")),
                        BytecodeOp::LoadLocal(Name::from(&prop.name)),
                        BytecodeOp::SetProperty(Name::from(&prop.name)),
                        BytecodeOp::StoreLocal(Name::intern("__variables")),
                        // Return this (unmodified — method preserved on this.name)
                        BytecodeOp::LoadLocal(Name::intern("this")),
                        BytecodeOp::Return,
                    ]
                } else {
                    vec![
                        // Set on this: this.name = value; store modified this back
                        BytecodeOp::LoadLocal(Name::intern("this")),
                        BytecodeOp::LoadLocal(Name::from(&prop.name)),
                        BytecodeOp::SetProperty(Name::from(&prop.name)),
                        BytecodeOp::StoreLocal(Name::intern("this")),
                        // Set on __variables: this.__variables.name = value
                        BytecodeOp::LoadLocal(Name::intern("this")),
                        BytecodeOp::TryGetProperty(Name::intern("__variables")),
                        BytecodeOp::LoadLocal(Name::from(&prop.name)),
                        BytecodeOp::SetProperty(Name::from(&prop.name)),
                        BytecodeOp::StoreLocal(Name::intern("__variables")),
                        // The value now sits on the top-level `this` scope (public),
                        // but Lucee keeps an accessor property PRIVATE (variables
                        // only). Mark it so introspection/for-in hide it while
                        // getX()/serializeJSON still surface it.
                        BytecodeOp::MarkAccessorPrivate(Name::from(&prop.name)),
                        // Return this
                        BytecodeOp::LoadLocal(Name::intern("this")),
                        BytecodeOp::Return,
                    ]
                };
                let setter_func = BytecodeFunction {
                    name: setter_name.clone(),
                    params: vec![prop.name.clone()],
                    param_keys: Default::default(),
                    args_needed: Default::default(),
                    args_never_escapes: Default::default(),
                    params_marker: Default::default(),
                    cfc_body: Default::default(),
                    required_params: vec![true],
                    has_default: vec![false],
                    instructions: setter_instructions,
                    source_file: self.source_file.clone(),
                    global_id: next_global_fn_id(),
                    declared_local_mode: None,
                    param_types: vec![None],
                    return_type: Some(component.name.clone()),
                    param_annotations: vec![Vec::new()],
                    is_component_method: true,
                    access: cfml_common::dynamic::CfmlAccess::Public,
                    metadata: Vec::new(),
                    // Declared for metadata, NOT enforced. Doubly so here: at
                    // codegen time an unnamed `component {}` is still called
                    // "Anonymous" (the real class name is stamped at runtime),
                    // so this declaration names a type the returned `this` could
                    // never satisfy.
                    is_generated_accessor: true,
                    output_suppressed: false,
                    is_template_frame: false,
            chain_tier: 0,
            slot_names: Vec::new(),
                };
                self.push_function(setter_func);
                let setter_gid = self.program.functions.last().unwrap().global_id as usize;
                instructions.push(BytecodeOp::DefineFunction(setter_gid));
                // Stack: [setter_func]

                // Add setter to component (same pattern)
                instructions.push(BytecodeOp::LoadLocal(Name::from(&component.name)));
                instructions.push(BytecodeOp::Swap);
                instructions.push(BytecodeOp::SetProperty(Name::from(&setter_name)));
                instructions.push(BytecodeOp::StoreLocal(Name::from(&component.name)));
            }
        }

        // Now store as a component template in global scope (with accessors included)
        instructions.push(BytecodeOp::LoadLocal(Name::from(&component.name)));
        instructions.push(BytecodeOp::StoreGlobal(Name::from(&component.name)));

        // Compile component methods and add them to the component struct.
        // Set in_component_method so the resulting BytecodeFunction is flagged
        // as a method — the VM's DefineFunction guard against builtin-name
        // collisions skips methods (Lucee allows `obj.canonicalize()` etc.).
        let prev_in_method = self.in_component_method;
        self.in_component_method = true;
        for func in &component.functions {
            let gid = self.compile_function_decl(func, instructions);
            // SetProperty needs: stack = [object, value]. Load the component
            // struct, then push a fresh function reference via DefineFunction.
            // Re-emitting DefineFunction (rather than LoadLocal(func.name))
            // avoids loading the local *scope* when the method name is a
            // reserved scope word like `local` (Preside Config.cfc environment
            // methods: `function local(){}`).
            instructions.push(BytecodeOp::LoadLocal(Name::from(&component.name)));
            instructions.push(BytecodeOp::DefineFunction(gid));
            instructions.push(BytecodeOp::SetProperty(Name::from(&func.name)));
            instructions.push(BytecodeOp::StoreLocal(Name::from(&component.name)));
        }
        self.in_component_method = prev_in_method;

        // Emit per-function metadata as __funcmeta_<name> keys
        for func in &component.functions {
            if !func.metadata.is_empty() {
                let meta_key = format!("__funcmeta_{}", func.name);
                for (k, v) in &func.metadata {
                    instructions.push(BytecodeOp::String(k.clone()));
                    instructions.push(BytecodeOp::String(v.clone()));
                }
                instructions.push(BytecodeOp::BuildStruct(func.metadata.len()));
                instructions.push(BytecodeOp::LoadLocal(Name::from(&component.name)));
                instructions.push(BytecodeOp::Swap);
                instructions.push(BytecodeOp::SetProperty(Name::from(meta_key)));
                instructions.push(BytecodeOp::StoreLocal(Name::from(&component.name)));
            }
        }

        // Emit __properties array listing property metadata structs
        if !component.properties.is_empty() {
            let prop_count = component.properties.len();
            for prop in &component.properties {
                // Each property is a struct with name, type, required, and any custom attributes
                let mut attr_count = 1; // always have "name"
                instructions.push(BytecodeOp::String("name".to_string()));
                instructions.push(BytecodeOp::String(prop.name.clone()));
                // `type` ALWAYS appears in property metadata — Lucee/ACF default an
                // undeclared property's type to "any" (verified: getMetaData shows
                // `type=any`). Preside's PresideObjectReader reads `prop.type`
                // directly (`if ( prop.type == "any" )`), which threw
                // "Variable 'type' is undefined" (post-v0.408) when the key was
                // absent. Skip only if a custom `type` attribute already provides it.
                let has_type_attr = prop
                    .attributes
                    .iter()
                    .any(|(k, _)| k.eq_ignore_ascii_case("type"));
                if let Some(ref pt) = prop.prop_type {
                    instructions.push(BytecodeOp::String("type".to_string()));
                    instructions.push(BytecodeOp::String(pt.clone()));
                    attr_count += 1;
                } else if !has_type_attr {
                    instructions.push(BytecodeOp::String("type".to_string()));
                    instructions.push(BytecodeOp::String("any".to_string()));
                    attr_count += 1;
                }
                if prop.required {
                    instructions.push(BytecodeOp::String("required".to_string()));
                    instructions.push(BytecodeOp::True);
                    attr_count += 1;
                }
                // `default="…"` must surface in getMetadata().properties (Lucee
                // keeps it) — frameworks read it to auto-populate unprovided fields
                // (e.g. Preside insertData defaults). The default is parsed as an
                // expression; for the literal forms used in practice
                // (`default="property_a default"`, `"cfml:Now()"`,
                // `"method:CalculatePropC"`) compiling it yields exactly the source
                // string Lucee stores. This mirrors the accessor-default emission
                // above, so it is no riskier to evaluate here.
                if let Some(ref default_expr) = prop.default {
                    instructions.push(BytecodeOp::String("default".to_string()));
                    self.compile_expression(default_expr, instructions);
                    attr_count += 1;
                }
                // Custom attributes (inject, hint, etc.)
                for (key, val) in &prop.attributes {
                    instructions.push(BytecodeOp::String(key.clone()));
                    instructions.push(BytecodeOp::String(val.clone()));
                    attr_count += 1;
                }
                instructions.push(BytecodeOp::BuildStruct(attr_count));
            }
            instructions.push(BytecodeOp::BuildArray(prop_count));
            instructions.push(BytecodeOp::LoadLocal(Name::from(&component.name)));
            instructions.push(BytecodeOp::Swap);
            instructions.push(BytecodeOp::SetProperty(Name::intern("__properties")));
            instructions.push(BytecodeOp::StoreLocal(Name::from(&component.name)));
        }

        // Update global copy after methods and metadata are added
        if !component.functions.is_empty() || !component.metadata.is_empty() || !component.properties.is_empty() {
            instructions.push(BytecodeOp::LoadLocal(Name::from(&component.name)));
            instructions.push(BytecodeOp::StoreGlobal(Name::from(&component.name)));
        }

        // Compile component body statements (e.g., this.name = "xxx", this.mappings = {...})
        // These execute as init code that modifies the component struct via `this`
        if !component.body.is_empty() {
            // Bind `this` to the component struct so `this.xxx = val` works
            instructions.push(BytecodeOp::LoadLocal(Name::from(&component.name)));
            instructions.push(BytecodeOp::StoreLocal(Name::intern("this")));

            for stmt in &component.body {
                self.compile_statement(stmt, instructions);
            }

            // Copy modified `this` back to component name and global
            instructions.push(BytecodeOp::LoadLocal(Name::intern("this")));
            instructions.push(BytecodeOp::StoreLocal(Name::from(&component.name)));
            instructions.push(BytecodeOp::LoadLocal(Name::from(&component.name)));
            instructions.push(BytecodeOp::StoreGlobal(Name::from(&component.name)));
        }

        // Compile the `static { ... }` initialization block into a standalone
        // `__cfc_static_init__` function. The VM runs it once per component type
        // (resolve_component_template), captures its locals, and freezes them into
        // the shared `static` scope. Compiling at function depth so unscoped
        // assignments inside the block (e.g. `GREETING = "x"`) lower to StoreLocal
        // (a static-scope member, captured on return) rather than page globals,
        // and `static.X` routes through the reserved-scope chain.
        if !component.static_body.is_empty() {
            let mut static_instrs = Vec::new();
            let prev_depth = self.function_depth;
            self.function_depth += 1;
            for stmt in &component.static_body {
                self.compile_statement(stmt, &mut static_instrs);
            }
            self.function_depth = prev_depth;
            static_instrs.push(BytecodeOp::Null);
            static_instrs.push(BytecodeOp::Return);
            let static_func = BytecodeFunction {
                name: "__cfc_static_init__".to_string(),
                params: Vec::new(),
                param_keys: Default::default(),
                    args_needed: Default::default(),
                    args_never_escapes: Default::default(),
                    params_marker: Default::default(),
                    cfc_body: Default::default(),
                required_params: Vec::new(),
                has_default: Vec::new(),
                instructions: static_instrs,
                source_file: self.source_file.clone(),
                global_id: next_global_fn_id(),
                declared_local_mode: None,
                param_types: Vec::new(),
                return_type: None,
                param_annotations: Vec::new(),
                is_component_method: true,
                access: cfml_common::dynamic::CfmlAccess::Public,
                metadata: Vec::new(),
                is_generated_accessor: false,
                output_suppressed: false,
                is_template_frame: false,
            chain_tier: 0,
            slot_names: Vec::new(),
            };
            self.push_function(static_func);
        }
    }

    /// Compile constructor arguments for a `new X(...)` expression (the class
    /// name has already been pushed). Emits `NewObjectNamed` when any argument
    /// is named so init() binds by name; otherwise the positional `NewObject`.
    fn compile_new_args(&mut self, args: &[Expression], instructions: &mut Vec<BytecodeOp>) {
        let has_named = args
            .iter()
            .any(|a| matches!(a, Expression::NamedArgument(_)));
        if has_named {
            let mut names = Vec::with_capacity(args.len());
            for arg in args {
                if let Expression::NamedArgument(named) = arg {
                    names.push(named.name.clone());
                    self.compile_expression(&named.value, instructions);
                } else {
                    // Positional arg mixed with named — empty name (mirrors CallNamed).
                    names.push(String::new());
                    self.compile_expression(arg, instructions);
                }
            }
            instructions.push(BytecodeOp::NewObjectNamed(names, args.len()));
        } else {
            for arg in args {
                self.compile_expression(arg, instructions);
            }
            instructions.push(BytecodeOp::NewObject(args.len()));
        }
    }

    fn compile_expression(&mut self, expr: &Expression, instructions: &mut Vec<BytecodeOp>) {
        match expr {
            Expression::Literal(lit) => match &lit.value {
                LiteralValue::Null => instructions.push(BytecodeOp::Null),
                LiteralValue::Bool(true) => instructions.push(BytecodeOp::True),
                LiteralValue::Bool(false) => instructions.push(BytecodeOp::False),
                LiteralValue::Int(i) => instructions.push(BytecodeOp::Integer(*i)),
                LiteralValue::Double(d) => instructions.push(BytecodeOp::Double(*d)),
                LiteralValue::String(s) => instructions.push(BytecodeOp::String(s.clone())),
            },
            Expression::Identifier(id) => {
                instructions.push(BytecodeOp::LoadLocal(Name::from(&id.name)));
            }
            Expression::BinaryOp(binop) => {
                if binop.operator == BinaryOpType::Assign {
                    // Does THIS assignment need to leave its value on the stack?
                    // True only when it is in value position (the RHS of an
                    // enclosing assignment). Captured-and-reset here so the
                    // recursive RHS compile below starts from a clean slate.
                    let want_value = self.need_assign_value;
                    self.need_assign_value = false;

                    // Dynamic/quoted-string LHS: `"variables.x" = v` (literal) or
                    // `"#scope#.#prop#" = v` (interpolated). CFML treats a
                    // string-valued lvalue as a runtime scope path. Push
                    // [pathString, value] and resolve the target at runtime.
                    // SetDynamicVar already pushes the value back, so this path
                    // satisfies `want_value` either way.
                    if matches!(
                        &*binop.left,
                        Expression::Literal(Literal { value: LiteralValue::String(_), .. })
                            | Expression::StringInterpolation(_)
                    ) {
                        self.compile_expression(&binop.left, instructions);
                        self.compile_expression(&binop.right, instructions);
                        instructions.push(BytecodeOp::SetDynamicVar);
                        return;
                    }

                    // A chained RHS (`b = c` in `a = b = c`) must itself leave a
                    // value for this assignment's store to consume.
                    self.need_assign_value = matches!(
                        &*binop.right,
                        Expression::BinaryOp(b) if b.operator == BinaryOpType::Assign
                    );
                    self.compile_expression(&binop.right, instructions);
                    self.need_assign_value = false;

                    // CFML null-assignment semantics: `x = voidFn()` (a Null RHS)
                    // must DELETE the target, not materialize a null-valued key.
                    // Guard the value-CONSUMING store paths with JumpIfNotNull
                    // (peeks, doesn't pop): a non-null RHS jumps straight to the
                    // store; a Null falls through to Pop + UnsetPath, leaving the
                    // stack empty like the store branch. Scope-rooted-nested
                    // targets aren't guarded here — they store via SetDynamicVar,
                    // whose store_runtime_path already deletes on Null. Only a
                    // possibly-null RHS pays for the guard.
                    let mut unset_end_jump = None;
                    if Self::expr_may_be_null(&binop.right) {
                        if let Some(path) = Self::expr_assign_unset_path(&binop.left) {
                            instructions.push(BytecodeOp::JumpIfNotNull(0)); // -> store (patched)
                            let guard_idx = instructions.len() - 1;
                            instructions.push(BytecodeOp::Pop); // drop the Null
                            instructions.push(BytecodeOp::UnsetPath(path));
                            instructions.push(BytecodeOp::Jump(0)); // -> end (patched)
                            unset_end_jump = Some(instructions.len() - 1);
                            instructions[guard_idx] = BytecodeOp::JumpIfNotNull(instructions.len());
                        }
                    }

                    // This is assignment in EXPRESSION position (e.g. the RHS of
                    // a chained assignment `a = b = expr`, or an assignment used
                    // as a value). Such an expression must LEAVE its assigned
                    // value on the stack for the enclosing context. The
                    // value-consuming store ops below (StoreLocal/
                    // StoreLocalProperty) would otherwise leave nothing, so the
                    // outer assignment got no value (Preside Config.cfc:
                    // `settings.x = application.x = expr` left `settings.x`
                    // unset). A `Dup` before each consuming store keeps one copy.
                    // (The SetDynamicVar paths above already push the value back,
                    // so they are intentionally left untouched.)
                    match &*binop.left {
                        Expression::Identifier(ident) => {
                            // `Dup` only when the value is needed (chained
                            // assignment); a statement-level store leaves the
                            // bytecode JIT-admissible (no stray Dup).
                            if want_value {
                                instructions.push(BytecodeOp::Dup);
                            }
                            instructions.push(BytecodeOp::StoreLocal(Name::from(&ident.name)));
                        }
                        Expression::MemberAccess(access) => {
                            // Nested write to an undeclared scope-qualified
                            // container used as an expression value
                            // (`x = (variables.a.b = v)`): route through the
                            // runtime scope-path store so missing intermediates
                            // auto-vivify. Stack on entry is [value]; SetDynamicVar
                            // wants [path, value] and pushes the value back (the
                            // expression's result), so no trailing Pop here.
                            if let Some(path) =
                                Self::scope_rooted_nested_path(&access.object, &access.member)
                            {
                                instructions.push(BytecodeOp::String(path));
                                instructions.push(BytecodeOp::Swap);
                                instructions.push(BytecodeOp::SetDynamicVar);
                            } else if let Some(path) =
                                Self::bare_rooted_nested_path(&access.object, &access.member)
                            {
                                // Undeclared bare root ≥2 levels deep in value
                                // position (`x = (copies.request.cgi = v)`):
                                // auto-vivify through the runtime store, which
                                // pushes the value back for the outer store.
                                instructions.push(BytecodeOp::String(path));
                                instructions.push(BytecodeOp::Swap);
                                instructions.push(BytecodeOp::SetDynamicVar);
                            } else if want_value
                                && matches!(&*access.object, Expression::Identifier(id) if is_reserved_scope_name(&id.name))
                            {
                                // Single-level reserved-scope member in VALUE
                                // position (`x = application.y = v`): the normal
                                // SetProperty+writeback path consumes the value
                                // without leaving it. Route through SetDynamicVar
                                // (`scope.member` path), which writes the scope
                                // AND pushes the value back for the outer store.
                                let id = match &*access.object {
                                    Expression::Identifier(id) => id.name.clone(),
                                    _ => unreachable!(),
                                };
                                instructions.push(BytecodeOp::String(format!("{}.{}", id, access.member)));
                                instructions.push(BytecodeOp::Swap);
                                instructions.push(BytecodeOp::SetDynamicVar);
                            } else if let Expression::Identifier(ref ident) = *access.object {
                                // Stack has [value]. When the object is a bare,
                                // non-scope identifier, use the fused
                                // StoreLocalProperty op, which auto-vivifies the
                                // local as a struct if it does not yet exist
                                // (Lucee/ACF/BoxLang semantics). Loading the object
                                // directly would throw "Variable 'x' is undefined"
                                // for an undeclared base.
                                if !is_reserved_scope_name(&ident.name) {
                                    if want_value {
                                        instructions.push(BytecodeOp::Dup);
                                    }
                                    instructions.push(BytecodeOp::StoreLocalProperty(Name::from(&ident.name),Name::from(&access.member),
                                    ));
                                } else if ident.name.eq_ignore_ascii_case("local") && self.local_is_scope() {
                                    // GH #351: only inside a function body.
                                    // `local.X = v` is identical to `var X = v` —
                                    // function-frame scope, must NOT propagate to
                                    // caller at return. Same fix as the
                                    // Statement::Assignment path above.
                                    if want_value {
                                        instructions.push(BytecodeOp::Dup);
                                    }
                                    instructions.push(BytecodeOp::DeclareLocal(Name::from(&access.member)));
                                    instructions.push(BytecodeOp::StoreLocal(Name::from(&access.member)));
                                } else if ident.name.eq_ignore_ascii_case("local") {
                                    // GH #351: see the Statement::Assignment twin.
                                    if want_value {
                                        instructions.push(BytecodeOp::Dup);
                                    }
                                    instructions.push(BytecodeOp::StoreLocalScopeKey(Name::from(&access.member)));
                                } else {
                                    // SetProperty needs [obj, value].
                                    self.compile_expression(&access.object, instructions);
                                    instructions.push(BytecodeOp::Swap);
                                    instructions.push(BytecodeOp::SetProperty(Name::from(&access.member)));
                                    self.emit_nested_writeback(&access.object, instructions);
                                }
                            } else {
                                // Object is `this` (Expression::This) or another
                                // non-identifier base. In VALUE position
                                // (`variables.x = this.y = v`) the result of this
                                // inner assignment must remain for the outer store;
                                // SetProperty + the This/nested writeback consume
                                // [obj,value] and leave nothing, so Dup the value
                                // first. The Dup'd copy sits beneath [obj,value] and
                                // survives the writeback as this expression's result.
                                if want_value {
                                    instructions.push(BytecodeOp::Dup);
                                }
                                // SetProperty needs [obj, value].
                                self.compile_expression(&access.object, instructions);
                                instructions.push(BytecodeOp::Swap);
                                instructions.push(BytecodeOp::SetProperty(Name::from(&access.member)));
                                // Write back through nested chain
                                self.emit_nested_writeback(&access.object, instructions);
                            }
                        }
                        Expression::ArrayAccess(access) => {
                            // In VALUE position (`a = b[k] = v`), the inner
                            // assignment must leave the assigned value for the
                            // outer store. SetIndex consumes [value, collection,
                            // index] and leaves only the modified collection (then
                            // the writeback consumes that), so Dup the value first;
                            // the spare copy sits at the bottom and is what remains
                            // after the writeback. (Was: `column = sql.columns[k] =
                            // StructNew()` left `column` undefined — Preside
                            // SqlSchemaSynchronizer.)
                            if want_value {
                                instructions.push(BytecodeOp::Dup);
                            }
                            self.compile_index_assign_base(&access.array, instructions);
                            self.compile_expression(&access.index, instructions);
                            instructions.push(BytecodeOp::SetIndex);
                            // SetIndex leaves modified collection on stack; write it back
                            self.emit_nested_writeback(&access.array, instructions);
                        }
                        _ => {}
                    }

                    // Close the null-delete guard: the store branch jumps here,
                    // past the Pop+UnsetPath sequence emitted before it.
                    if let Some(idx) = unset_end_jump {
                        instructions[idx] = BytecodeOp::Jump(instructions.len());
                    }
                    return;
                }

                // Logical AND/OR short-circuit: emit jump sequence so the
                // right-hand side is only evaluated when it can change the
                // result. Matches Lucee/ACF semantics; any side-effect or
                // throwing call on the RHS must NOT fire when the LHS already
                // decides the result (e.g. `false AND throws()`).
                if matches!(binop.operator, BinaryOpType::And | BinaryOpType::Or) {
                    let jump_on_short_circuit = matches!(binop.operator, BinaryOpType::Or);
                    self.compile_expression(&binop.left, instructions);
                    let short_jump_idx = instructions.len();
                    instructions.push(if jump_on_short_circuit {
                        BytecodeOp::JumpIfTrue(0)
                    } else {
                        BytecodeOp::JumpIfFalse(0)
                    });
                    self.compile_expression(&binop.right, instructions);
                    let second_jump_idx = instructions.len();
                    instructions.push(if jump_on_short_circuit {
                        BytecodeOp::JumpIfTrue(0)
                    } else {
                        BytecodeOp::JumpIfFalse(0)
                    });
                    // Fall-through path: neither short-circuited — result is
                    // !jump_on_short_circuit for AND (true), and FALSE for OR
                    // (we got here because both were not-true).
                    if jump_on_short_circuit {
                        instructions.push(BytecodeOp::False);
                    } else {
                        instructions.push(BytecodeOp::True);
                    }
                    let done_jump_idx = instructions.len();
                    instructions.push(BytecodeOp::Jump(0));
                    // Short-circuit landing pad.
                    let short_target = instructions.len();
                    if jump_on_short_circuit {
                        instructions.push(BytecodeOp::True);
                    } else {
                        instructions.push(BytecodeOp::False);
                    }
                    let end_target = instructions.len();
                    // Patch the three forward jumps.
                    match &mut instructions[short_jump_idx] {
                        BytecodeOp::JumpIfTrue(off) | BytecodeOp::JumpIfFalse(off) => {
                            *off = short_target;
                        }
                        _ => unreachable!(),
                    }
                    match &mut instructions[second_jump_idx] {
                        BytecodeOp::JumpIfTrue(off) | BytecodeOp::JumpIfFalse(off) => {
                            *off = short_target;
                        }
                        _ => unreachable!(),
                    }
                    if let BytecodeOp::Jump(off) = &mut instructions[done_jump_idx] {
                        *off = end_target;
                    }
                    return;
                }

                self.compile_expression(&binop.left, instructions);
                self.compile_expression(&binop.right, instructions);

                let op = match binop.operator {
                    BinaryOpType::Add => BytecodeOp::Add,
                    BinaryOpType::Sub => BytecodeOp::Sub,
                    BinaryOpType::Mul => BytecodeOp::Mul,
                    BinaryOpType::Div => BytecodeOp::Div,
                    BinaryOpType::Mod => BytecodeOp::Mod,
                    BinaryOpType::Pow => BytecodeOp::Pow,
                    BinaryOpType::IntDiv => BytecodeOp::IntDiv,
                    BinaryOpType::Concat => BytecodeOp::Concat,
                    BinaryOpType::Equal => BytecodeOp::Eq,
                    BinaryOpType::NotEqual => BytecodeOp::Neq,
                    BinaryOpType::StrictEqual => BytecodeOp::StrictEq,
                    BinaryOpType::StrictNotEqual => BytecodeOp::StrictNeq,
                    BinaryOpType::Less => BytecodeOp::Lt,
                    BinaryOpType::LessEqual => BytecodeOp::Lte,
                    BinaryOpType::Greater => BytecodeOp::Gt,
                    BinaryOpType::GreaterEqual => BytecodeOp::Gte,
                    BinaryOpType::And | BinaryOpType::Or => unreachable!(), // handled above
                    BinaryOpType::Xor => BytecodeOp::Xor,
                    BinaryOpType::Contains => BytecodeOp::Contains,
                    BinaryOpType::DoesNotContain => BytecodeOp::DoesNotContain,
                    BinaryOpType::Eqv => BytecodeOp::Eqv,
                    BinaryOpType::Imp => BytecodeOp::Imp,
                    BinaryOpType::Assign => BytecodeOp::Null, // Should not reach here
                };
                instructions.push(op);
            }
            Expression::UnaryOp(unary) => {
                match unary.operator {
                    UnaryOpType::PrefixIncrement | UnaryOpType::PrefixDecrement => {
                        // ++i / --i: increment/decrement and leave NEW value on stack
                        let delta = if matches!(unary.operator, UnaryOpType::PrefixIncrement) {
                            1
                        } else {
                            -1
                        };
                        if let Expression::Identifier(ident) = &*unary.operand {
                            instructions.push(BytecodeOp::LoadLocal(Name::from(&ident.name)));
                            instructions.push(BytecodeOp::Integer(delta));
                            instructions.push(BytecodeOp::Add);
                            instructions.push(BytecodeOp::Dup);
                            instructions.push(BytecodeOp::StoreLocal(Name::from(&ident.name)));
                        } else if matches!(
                            &*unary.operand,
                            Expression::MemberAccess(_) | Expression::ArrayAccess(_)
                        ) {
                            // `++obj.member` / `++obj[key]` (and `--`): write back AND
                            // leave the NEW value (the old fallback computed the value
                            // but never persisted the increment, and never handled
                            // index targets at all).
                            self.compile_expression(&unary.operand, instructions); // [old]
                            instructions.push(BytecodeOp::Integer(delta));
                            instructions.push(BytecodeOp::Add); // [new]
                            instructions.push(BytecodeOp::Dup); // new value is the result
                            self.emit_nested_writeback(&unary.operand, instructions);
                            // New value remains on the stack as the expression result.
                        } else {
                            // Fallback: evaluate operand, add/subtract 1
                            self.compile_expression(&unary.operand, instructions);
                            instructions.push(BytecodeOp::Integer(delta));
                            instructions.push(BytecodeOp::Add);
                        }
                    }
                    _ => {
                        self.compile_expression(&unary.operand, instructions);
                        let op = match unary.operator {
                            UnaryOpType::Minus => BytecodeOp::Negate,
                            UnaryOpType::Not => BytecodeOp::Not,
                            UnaryOpType::BitNot => BytecodeOp::Not,
                            _ => unreachable!(),
                        };
                        instructions.push(op);
                    }
                }
            }
            Expression::PostfixOp(postfix) => {
                if let Expression::Identifier(ident) = &*postfix.operand {
                    match postfix.operator {
                        PostfixOpType::Increment => {
                            instructions.push(BytecodeOp::LoadLocal(Name::from(&ident.name)));
                            instructions.push(BytecodeOp::Dup);
                            instructions.push(BytecodeOp::Integer(1));
                            instructions.push(BytecodeOp::Add);
                            instructions.push(BytecodeOp::StoreLocal(Name::from(&ident.name)));
                            // The original value stays on the stack
                        }
                        PostfixOpType::Decrement => {
                            instructions.push(BytecodeOp::LoadLocal(Name::from(&ident.name)));
                            instructions.push(BytecodeOp::Dup);
                            instructions.push(BytecodeOp::Integer(1));
                            instructions.push(BytecodeOp::Sub);
                            instructions.push(BytecodeOp::StoreLocal(Name::from(&ident.name)));
                        }
                    }
                } else if matches!(
                    &*postfix.operand,
                    Expression::MemberAccess(_) | Expression::ArrayAccess(_)
                ) {
                    // `obj.member++` / `obj[key]++` (and `--`) as an rvalue. The
                    // identifier arm above never matched, so this previously emitted
                    // NOTHING — leaving no value on the stack and silently shifting
                    // any surrounding struct literal / arg list by one slot (TestBox's
                    // `"order": this.$specOrderIndex++` spec literal), and making
                    // `variables.lookup[id]["totalPass"]++` a no-op (stats stuck at 0).
                    // Read the OLD value as the result, then write back old±1.
                    let delta = match postfix.operator {
                        PostfixOpType::Increment => 1,
                        PostfixOpType::Decrement => -1,
                    };
                    self.compile_expression(&postfix.operand, instructions); // [old]
                    instructions.push(BytecodeOp::Dup); // keep old value as result
                    instructions.push(BytecodeOp::Integer(delta));
                    instructions.push(BytecodeOp::Add); // [old, new]
                    self.emit_nested_writeback(&postfix.operand, instructions);
                    // The original value remains on the stack as the expression result.
                }
            }
            Expression::StaticMember(sm) => {
                // `Component::member` — read a static member without an instance.
                if let Some(name) = Self::static_class_name(&sm.class) {
                    instructions.push(BytecodeOp::LoadStaticHolder(Name::from(name)));
                } else {
                    self.compile_expression(&sm.class, instructions);
                }
                instructions.push(BytecodeOp::GetStaticProperty(Name::from(&sm.member)));
            }
            Expression::StaticCall(sc) => {
                // `Component::method(args)` — call a static method without an
                // instance. The holder carries `__variables.__static`, so
                // `static.X` inside the method resolves through the normal chain.
                if let Some(name) = Self::static_class_name(&sc.class) {
                    instructions.push(BytecodeOp::LoadStaticHolder(Name::from(name)));
                } else {
                    self.compile_expression(&sc.class, instructions);
                }
                let has_named = sc
                    .arguments
                    .iter()
                    .any(|a| matches!(a, Expression::NamedArgument(_)));
                let mut names = Vec::with_capacity(sc.arguments.len());
                for arg in &sc.arguments {
                    if let Expression::NamedArgument(named) = arg {
                        names.push(named.name.clone());
                        self.compile_expression(&named.value, instructions);
                    } else {
                        names.push(String::new());
                        self.compile_expression(arg, instructions);
                    }
                }
                if has_named {
                    instructions.push(BytecodeOp::CallMethodNamed(Name::from(&sc.method),
                        Box::new(names),
                        sc.arguments.len(),
                        None,
                    ));
                } else {
                    instructions.push(BytecodeOp::CallMethod(Name::from(&sc.method),
                        sc.arguments.len(),
                        None,
                    ));
                }
            }
            Expression::MemberAccess(access) => {
                // Phase H peephole: at page scope, `variables.foo` clones the entire
                // globals map before reading one key. LoadGlobal semantics match
                // page-scope `variables.x` reads exactly (locals-then-globals).
                // Unsafe inside function bodies: `variables` there means the locals
                // merge or a CFC's `__variables` struct — LoadGlobal would hit page
                // globals instead. Also unsafe for null-safe `variables?.foo`.
                if !access.null_safe && self.function_depth == 0 {
                    if let Expression::Identifier(ref ident) = *access.object {
                        if ident.name.eq_ignore_ascii_case("variables") {
                            instructions
                                .push(BytecodeOp::LoadVariablesKey(Name::from(&access.member)));
                            return;
                        }
                    }
                }
                // Peephole: `<ident>.<member>` with no null-safe → fuse into
                // LoadLocalProperty. Skips the intermediate stack push of the
                // receiver plus the separate GetProperty dispatch.
                //
                // Skip when the identifier is a CFML-reserved scope name —
                // those resolve through the scope chain (globals, request,
                // __variables fallback, etc.) not just the locals map, and the
                // simple `locals.get(name)` lookup would return the wrong
                // value (typically null).
                if !access.null_safe {
                    if let Expression::Identifier(ref ident) = *access.object {
                        // `local.foo` read: fuse into LoadLocalKey so we read one
                        // key directly instead of materializing the whole per-call
                        // `local` scope view (see LoadLocalKey docs). Reads only —
                        // `local.foo = x` writes go through the assignment path.
                        // GH #351: emitted at EVERY depth. The op itself resolves
                        // `local` against the frame's real local-scope status, so a
                        // template included from inside a function keeps the caller's
                        // scope while a true page reads the ordinary `local` variable.
                        if ident.name.eq_ignore_ascii_case("local") {
                            instructions
                                .push(BytecodeOp::LoadLocalKey(Name::from(&access.member)));
                            return;
                        }
                        if !is_reserved_scope_name(&ident.name) {
                            instructions.push(BytecodeOp::LoadLocalProperty(Name::from(&ident.name),Name::from(&access.member),
                            ));
                            return;
                        }
                    }
                }
                // For null-safe access, the prefix must also read tolerantly — the
                // whole point of `?.` is that a missing/undefined link yields Null
                // rather than throwing (`s.deep?.x` with `deep` absent → Null).
                if access.null_safe {
                    if let Expression::Identifier(ref ident) = *access.object {
                        instructions.push(BytecodeOp::TryLoadLocal(Name::from(&ident.name)));
                    } else if matches!(
                        *access.object,
                        Expression::MemberAccess(_) | Expression::ArrayAccess(_)
                    ) {
                        self.compile_member_read_tolerant(&access.object, instructions);
                    } else {
                        self.compile_expression(&access.object, instructions);
                    }
                } else {
                    self.compile_expression(&access.object, instructions);
                }
                if access.null_safe {
                    // Null-safe: if object is null, skip property access (null stays on stack)
                    // JumpIfNotNull peeks without popping, so no Dup needed
                    let jump_idx = instructions.len();
                    instructions.push(BytecodeOp::JumpIfNotNull(0)); // placeholder
                    // Object is null - it's on the stack, skip the GetProperty
                    let jump_end = instructions.len();
                    instructions.push(BytecodeOp::Jump(0)); // placeholder
                    // Object is not null - do the property access. Null-safe reads
                    // stay Null-tolerant on a missing member (the `?.` contract),
                    // so use the Try* twin rather than the throwing GetProperty.
                    instructions[jump_idx] = BytecodeOp::JumpIfNotNull(instructions.len());
                    instructions.push(BytecodeOp::TryGetProperty(Name::from(&access.member)));
                    instructions[jump_end] = BytecodeOp::Jump(instructions.len());
                } else {
                    instructions.push(BytecodeOp::GetProperty(Name::from(&access.member)));
                }
            }
            Expression::ArrayAccess(access) => {
                self.compile_expression(&access.array, instructions);
                self.compile_expression(&access.index, instructions);
                instructions.push(BytecodeOp::GetIndex);
            }
            Expression::FunctionCall(call) => {
                // Special-case: super(args) — explicit Rust-parent ctor call from
                // inside a CFC init() method. Compile args then emit a dedicated
                // op that re-runs the registered constructor and replaces
                // this.__super with the freshly-built NativeObject.
                if matches!(&*call.name, Expression::Super(_)) {
                    let n = call.arguments.len();
                    for arg in &call.arguments {
                        self.compile_expression(arg, instructions);
                    }
                    instructions.push(BytecodeOp::CallRustSuperCtor(n));
                    return;
                }
                // Special-case: isDefined("varName") -> IsDefined bytecode
                if let Expression::Identifier(ident) = &*call.name {
                    if ident.name.to_lowercase() == "isdefined" && call.arguments.len() == 1 {
                        if let Expression::Literal(Literal { value: LiteralValue::String(ref var_name), .. }) = call.arguments[0] {
                            instructions.push(BytecodeOp::IsDefined(Name::from(&var_name)));
                            return;
                        }
                    }
                    // Special-case: isNull(varName) -> TryLoadLocal + IsNull
                    // Uses TryLoadLocal so undefined vars return Null (true) rather than erroring
                    if ident.name.to_lowercase() == "isnull" && call.arguments.len() == 1 {
                        if let Expression::Identifier(ref arg_ident) = call.arguments[0] {
                            instructions.push(BytecodeOp::TryLoadLocal(Name::from(&arg_ident.name)));
                            instructions.push(BytecodeOp::IsNull);
                            return;
                        }
                        // `isNull(a.b.c)` must not throw on a missing member. Keep
                        // isNull on the builtin-call path (which the JIT specialises
                        // via the isnull shim) but read the member argument
                        // null-tolerantly so a missing link is `true`, not a throw.
                        // (Emitting the `IsNull` bytecode op here would work but is
                        // not JIT-admissible, needlessly demoting the caller.)
                        if matches!(
                            call.arguments[0],
                            Expression::MemberAccess(_) | Expression::ArrayAccess(_)
                        ) {
                            instructions.push(BytecodeOp::LoadGlobal(Name::from(&ident.name)));
                            self.compile_member_read_tolerant(&call.arguments[0], instructions);
                            instructions.push(BytecodeOp::Call(1));
                            return;
                        }
                    }
                }

                let has_spread = call.arguments.iter().any(|a| matches!(a, Expression::Spread(_)));
                let has_named = call.arguments.iter().any(|a| matches!(a, Expression::NamedArgument(_)));
                // Computed-name method call: `obj[ nameExpr ]( args )`. Dispatch
                // as a method on `obj` (binds the receiver's component scope)
                // rather than indexing out a bare Function and calling it with the
                // caller's scope. Spread args keep the legacy path (rare; the
                // dynamic-receiver + spread combination isn't exercised).
                if !has_spread {
                    if let Expression::ArrayAccess(aa) = &*call.name {
                        // object
                        self.compile_expression(&aa.array, instructions);
                        // method name
                        self.compile_expression(&aa.index, instructions);
                        if has_named {
                            let mut names = Vec::new();
                            for arg in &call.arguments {
                                if let Expression::NamedArgument(named) = arg {
                                    names.push(named.name.clone());
                                    self.compile_expression(&named.value, instructions);
                                } else {
                                    names.push(String::new());
                                    self.compile_expression(arg, instructions);
                                }
                            }
                            instructions.push(BytecodeOp::CallComputedMethodNamed(
                                Box::new(names),
                                call.arguments.len(),
                            ));
                        } else {
                            for arg in &call.arguments {
                                self.compile_expression(arg, instructions);
                            }
                            instructions.push(BytecodeOp::CallComputedMethod(call.arguments.len()));
                        }
                        return;
                    }
                }
                if has_spread {
                    // Push function reference first
                    if let Expression::Identifier(ident) = &*call.name {
                        instructions.push(BytecodeOp::LoadGlobal(Name::from(&ident.name)));
                    } else {
                        self.compile_expression(&call.name, instructions);
                    }
                    // Build args array using concat pattern
                    instructions.push(BytecodeOp::BuildArray(0));
                    for arg in &call.arguments {
                        if let Expression::Spread(inner) = arg {
                            self.compile_expression(inner, instructions);
                            instructions.push(BytecodeOp::ConcatArrays);
                        } else {
                            self.compile_expression(arg, instructions);
                            instructions.push(BytecodeOp::BuildArray(1));
                            instructions.push(BytecodeOp::ConcatArrays);
                        }
                    }
                    instructions.push(BytecodeOp::CallSpread);
                } else if has_named {
                    // Named arguments: push function ref, then compile values, emit CallNamed
                    if let Expression::Identifier(ident) = &*call.name {
                        instructions.push(BytecodeOp::LoadGlobal(Name::from(&ident.name)));
                    } else {
                        self.compile_expression(&call.name, instructions);
                    }
                    let mut names = Vec::new();
                    for arg in &call.arguments {
                        if let Expression::NamedArgument(named) = arg {
                            names.push(named.name.clone());
                            self.compile_expression(&named.value, instructions);
                        } else {
                            // Positional arg mixed with named — use empty name
                            names.push(String::new());
                            self.compile_expression(arg, instructions);
                        }
                    }
                    instructions.push(BytecodeOp::CallNamed(names, call.arguments.len()));
                } else {
                    // Compile-time-bound builtin: skip the LoadGlobal + generic
                    // Call dispatch entirely. Preside's warm admin render makes
                    // 33,484 builtin calls, 62% of them type/existence
                    // predicates (structKeyExists alone is 28%), and Part 1
                    // measured the cost as the CALL BOUNDARY (~105-150 ns) not
                    // the body — the worst Lucee ratios are on body-free
                    // predicates. Same shape Lucee emits (see
                    // `VariableImpl._writeOutFirstBIF`).
                    let direct = match &*call.name {
                        Expression::Identifier(ident)
                            if call.arguments.len() <= u8::MAX as usize
                                && is_direct_builtin(&ident.name) =>
                        {
                            Some(ident.name.to_lowercase())
                        }
                        _ => None,
                    };
                    if let Some(lower) = direct {
                        for arg in &call.arguments {
                            self.compile_expression(arg, instructions);
                        }
                        instructions.push(BytecodeOp::CallBuiltin(
                            Name::from(&lower),
                            call.arguments.len() as u8,
                        ));
                        return;
                    }
                    // Push function reference first
                    if let Expression::Identifier(ident) = &*call.name {
                        instructions.push(BytecodeOp::LoadGlobal(Name::from(&ident.name)));
                    } else {
                        self.compile_expression(&call.name, instructions);
                    }
                    // Push arguments
                    for arg in &call.arguments {
                        self.compile_expression(arg, instructions);
                    }
                    instructions.push(BytecodeOp::Call(call.arguments.len()));
                }
            }
            Expression::MethodCall(call) => {
                // Determine write-back target from the AST.
                // this.items.append(x) → write_back = Some(("this", Some("items")))
                // dog.method(x)        → write_back = Some(("dog", None))
                let write_back = Self::method_call_write_back(&call.object);
                let has_named = call
                    .arguments
                    .iter()
                    .any(|arg| matches!(arg, Expression::NamedArgument(_)));

                // Compile each argument value onto the stack, collecting the
                // call-site names (empty string for positional args). Mirrors
                // the named-arg handling for free-function calls (CallNamed).
                let compile_args =
                    |compiler: &mut Self, instructions: &mut Vec<BytecodeOp>| -> Vec<String> {
                        let mut names = Vec::with_capacity(call.arguments.len());
                        for arg in &call.arguments {
                            if let Expression::NamedArgument(named) = arg {
                                names.push(named.name.clone());
                                compiler.compile_expression(&named.value, instructions);
                            } else {
                                names.push(String::new());
                                compiler.compile_expression(arg, instructions);
                            }
                        }
                        names
                    };

                // For null-safe method calls, use TryLoadLocal for simple identifiers
                if call.null_safe {
                    if let Expression::Identifier(ref ident) = *call.object {
                        instructions.push(BytecodeOp::TryLoadLocal(Name::from(&ident.name)));
                    } else {
                        self.compile_expression(&call.object, instructions);
                    }
                } else {
                    self.compile_expression(&call.object, instructions);
                }
                if call.null_safe {
                    let jump_idx = instructions.len();
                    instructions.push(BytecodeOp::JumpIfNotNull(0));
                    let jump_end = instructions.len();
                    instructions.push(BytecodeOp::Jump(0));
                    instructions[jump_idx] = BytecodeOp::JumpIfNotNull(instructions.len());
                    let names = compile_args(self, instructions);
                    if has_named {
                        instructions.push(BytecodeOp::CallMethodNamed(Name::from(&call.method),
                            Box::new(names),
                            call.arguments.len(),
                            write_back.clone(),
                        ));
                    } else {
                        instructions.push(BytecodeOp::CallMethod(Name::from(&call.method),
                            call.arguments.len(),
                            write_back.clone(),
                        ));
                    }
                    instructions[jump_end] = BytecodeOp::Jump(instructions.len());
                } else {
                    let names = compile_args(self, instructions);
                    if has_named {
                        instructions.push(BytecodeOp::CallMethodNamed(Name::from(&call.method),
                            Box::new(names),
                            call.arguments.len(),
                            write_back,
                        ));
                    } else {
                        instructions.push(BytecodeOp::CallMethod(Name::from(&call.method),
                            call.arguments.len(),
                            write_back,
                        ));
                    }
                }
            }
            Expression::Array(arr) => {
                let has_spread = arr.elements.iter().any(|e| matches!(e, Expression::Spread(_)));
                if has_spread {
                    // Start with empty array
                    instructions.push(BytecodeOp::BuildArray(0));
                    for elem in &arr.elements {
                        if let Expression::Spread(inner) = elem {
                            // Compile spread expr (should be array), concat
                            self.compile_expression(inner, instructions);
                            instructions.push(BytecodeOp::ConcatArrays);
                        } else {
                            // Compile single element, wrap in 1-element array, concat
                            self.compile_expression(elem, instructions);
                            instructions.push(BytecodeOp::BuildArray(1));
                            instructions.push(BytecodeOp::ConcatArrays);
                        }
                    }
                } else {
                    for elem in &arr.elements {
                        self.compile_expression(elem, instructions);
                    }
                    instructions.push(BytecodeOp::BuildArray(arr.elements.len()));
                }
            }
            Expression::Struct(st) => {
                let has_spread = st.pairs.iter().any(|(k, _)| matches!(k, Expression::Spread(_)));
                if has_spread {
                    // Start with empty struct
                    instructions.push(BytecodeOp::BuildStruct(0));
                    for (key, value) in &st.pairs {
                        if let Expression::Spread(_inner) = key {
                            // Spread: compile the value (which is the spread expr), merge
                            self.compile_expression(value, instructions);
                            instructions.push(BytecodeOp::MergeStructs);
                        } else {
                            // Normal pair: compile key/value, build 1-pair struct, merge
                            match key {
                                Expression::Identifier(ident) => {
                                    instructions.push(BytecodeOp::String(ident.name.clone()));
                                }
                                _ => {
                                    self.compile_expression(key, instructions);
                                }
                            }
                            self.compile_expression(value, instructions);
                            instructions.push(BytecodeOp::BuildStruct(1));
                            instructions.push(BytecodeOp::MergeStructs);
                        }
                    }
                } else {
                    // Lucee/ACF: a struct literal with dotted-path keys builds a
                    // nested struct — `{ obj_a.meta = X }` is `{ obj_a: { meta: X } }`.
                    // Detect when every key flattens to a literal path AND at least
                    // one is multi-segment; if so, build the nested tree (correct
                    // deep-merge + ordering) and emit nested BuildStruct ops. Any
                    // computed/bracketed key falls back to the flat path below,
                    // where its key expression is evaluated at runtime.
                    let key_paths: Vec<Option<Vec<String>>> = st
                        .pairs
                        .iter()
                        .map(|(k, _)| Self::flatten_struct_key_path(k))
                        .collect();
                    let use_nested = key_paths
                        .iter()
                        .any(|p| matches!(p, Some(segs) if segs.len() > 1));

                    if use_nested {
                        let mut root: Vec<(StructKey, StructKeyNode)> = Vec::new();
                        for ((key, value), segs) in st.pairs.iter().zip(key_paths.iter()) {
                            match segs {
                                // Literal (identifier/quoted/dotted) key — merge
                                // into the nested tree.
                                Some(segs) => {
                                    Self::insert_struct_path(&mut root, segs, value.clone());
                                }
                                // Computed key (`{ "#k#" = v }`) — a single
                                // runtime-evaluated top-level entry; never nests.
                                None => {
                                    root.push((
                                        StructKey::Computed(key.clone()),
                                        StructKeyNode::Leaf(value.clone()),
                                    ));
                                }
                            }
                        }
                        self.emit_struct_tree(&root, instructions);
                    } else {
                        for (key, value) in &st.pairs {
                            match key {
                                Expression::Identifier(ident) => {
                                    instructions.push(BytecodeOp::String(ident.name.clone()));
                                }
                                _ => {
                                    self.compile_expression(key, instructions);
                                }
                            }
                            self.compile_expression(value, instructions);
                        }
                        instructions.push(BytecodeOp::BuildStruct(st.pairs.len()));
                    }
                }
            }
            Expression::Ternary(tern) => {
                self.compile_expression(&tern.condition, instructions);
                let jump_false = instructions.len();
                instructions.push(BytecodeOp::JumpIfFalse(0));

                self.compile_expression(&tern.then_expr, instructions);
                let jump_end = instructions.len();
                instructions.push(BytecodeOp::Jump(0));

                instructions[jump_false] = BytecodeOp::JumpIfFalse(instructions.len());
                self.compile_expression(&tern.else_expr, instructions);
                instructions[jump_end] = BytecodeOp::Jump(instructions.len());
            }
            Expression::New(new_expr) => {
                // Parser may parse `new Dog(args)` as class=FunctionCall(Dog, args)
                // Extract the class name and push it for VM resolution
                match &*new_expr.class {
                    Expression::FunctionCall(call) => {
                        // Try flattening dot-path: new a.b.c(args) parses as FunctionCall(MemberAccess(a,b).c, args)
                        if let Some(path) = Self::flatten_member_access(&call.name) {
                            instructions.push(BytecodeOp::String(path));
                        } else if let Expression::Identifier(ident) = &*call.name {
                            instructions.push(BytecodeOp::String(ident.name.clone()));
                        } else {
                            self.compile_expression(&call.name, instructions);
                        }
                        self.compile_new_args(&call.arguments, instructions);
                    }
                    Expression::Identifier(ident) => {
                        // Push class name as string - VM will look up in locals, globals, or .cfc files
                        instructions.push(BytecodeOp::String(ident.name.clone()));
                        self.compile_new_args(&new_expr.arguments, instructions);
                    }
                    Expression::MemberAccess(_) => {
                        // Handle bare dotted path: new a.b.c without parens
                        if let Some(path) = Self::flatten_member_access(&new_expr.class) {
                            instructions.push(BytecodeOp::String(path));
                        } else {
                            self.compile_expression(&new_expr.class, instructions);
                        }
                        self.compile_new_args(&new_expr.arguments, instructions);
                    }
                    _ => {
                        self.compile_expression(&new_expr.class, instructions);
                        self.compile_new_args(&new_expr.arguments, instructions);
                    }
                }
            }
            Expression::Closure(closure) => {
                // Compile closure body into separate function.
                // Lucee: closure inherits its enclosing function's localMode
                // when it doesn't carry its own attribute. Track current_fn for
                // nested closures-inside-closures too.
                let closure_declared = metadata_declared_local_mode(&closure.metadata);
                let effective_declared = closure_declared.or(self.current_fn_local_mode);
                let prev_fn_local_mode = self.current_fn_local_mode;
                self.current_fn_local_mode = effective_declared;
                // A closure/arrow body owns a `local` scope, exactly like a
                // declared function's. Tracked SEPARATELY from `function_depth`
                // (which closures deliberately do not bump — it also gates the
                // `variables.foo` → LoadVariablesKey peephole, and changing that
                // for closure bodies broke `attributes` resolution inside a
                // cfthread body nested in a custom tag).
                self.local_scope_depth += 1;

                // Function boundary: isolate finally/loop stacks for the closure
                // body (see compile_function_decl for why).
                let saved_finally = std::mem::take(&mut self.finally_stack);
                let saved_loops = std::mem::take(&mut self.loop_stack);
                let saved_tag_pairs = std::mem::take(&mut self.tag_pair_stack);
        let saved_catch_vars = std::mem::take(&mut self.catch_var_stack);

                let mut func_instructions = Vec::new();
                // Emit default parameter value preamble for closures.
                // Presence is tested against the `arguments` scope
                // (JumpIfArgPresent), NOT `LoadLocal + IsNull`: the VM no longer
                // pre-seeds an omitted param as a Null local, so `LoadLocal` on an
                // absent param now THROWS `Variable 'X' is undefined` (post-v0.408
                // strict undefined reads). Named functions were switched to this
                // pattern for GitHub #240; closures/arrows must match (GitHub #255).
                for (idx, param) in closure.params.iter().enumerate() {
                    if let Some(ref default_expr) = param.default {
                        let jump_idx = func_instructions.len();
                        func_instructions.push(BytecodeOp::JumpIfArgPresent(Name::from(&param.name), 0));
                        self.compile_expression(default_expr, &mut func_instructions);
                        // Seed the local AND the `arguments` key from the default, WITHOUT reading
                        // the parameter back by bare name. A `LoadLocal(param.name)` read-back is
                        // wrong for a parameter named after a built-in scope: since GH #312 a bare
                        // scope name always resolves to the SCOPE, so `function f( cookie = "D" )`
                        // seeded `arguments.cookie` with the live cookie scope instead of "D".
                        // `Dup` keeps the freshly-evaluated value on the stack for
                        // `SeedArgumentKey`, which consumes it: the local is stored by name
                        // (so slot behaviour is untouched) and the frame's OWN `arguments`
                        // scope gets the same value. Emitting `LoadLocal("arguments")` here
                        // is what used to force the whole function onto the eager path.
                        func_instructions.push(BytecodeOp::Dup);
                        func_instructions.push(BytecodeOp::StoreLocal(Name::from(&param.name)));
                        func_instructions.push(BytecodeOp::SeedArgumentKey(Name::from(&param.name)));
                        // Type-check the applied default (see compile_function_decl).
                        if declared_type_is_checkable(param.param_type.as_deref()) {
                            func_instructions.push(BytecodeOp::ValidateParamType(idx));
                        }
                        func_instructions[jump_idx] =
                            BytecodeOp::JumpIfArgPresent(Name::from(&param.name), func_instructions.len());
                    }
                }
                for s in &closure.body {
                    self.compile_statement(s, &mut func_instructions);
                }
                func_instructions.push(BytecodeOp::Null);
                func_instructions.push(BytecodeOp::Return);
                self.finally_stack = saved_finally;
                self.loop_stack = saved_loops;
                self.tag_pair_stack = saved_tag_pairs;
        self.catch_var_stack = saved_catch_vars;

                let func_name = format!("__closure_{}", self.program.functions.len());
                let bc_func = BytecodeFunction {
                    name: func_name.clone(),
                    params: closure.params.iter().map(|p| p.name.clone()).collect(),
                    param_keys: Default::default(),
                    args_needed: Default::default(),
                    args_never_escapes: Default::default(),
                    params_marker: Default::default(),
                    cfc_body: Default::default(),
                    required_params: closure.params.iter().map(|p| p.required).collect(),
                    has_default: closure.params.iter().map(|p| p.default.is_some()).collect(),
                    instructions: func_instructions,
                    source_file: self.source_file.clone(),
                    global_id: next_global_fn_id(),
                    declared_local_mode: effective_declared,
                    param_types: closure.params.iter().map(|p| p.param_type.clone()).collect(),
                    // `function( x ) returntype="numeric" { … }` — the attribute
                    // parses into the closure's metadata list, so a closure's
                    // declared return type is enforceable (and reportable) just
                    // like a named function's. Was hardcoded `None`.
                    return_type: closure
                        .metadata
                        .iter()
                        .find(|(k, _)| k.eq_ignore_ascii_case("returntype"))
                        .map(|(_, v)| v.clone()),
                    param_annotations: closure.params.iter().map(|p| p.annotations.clone()).collect(),
                    is_component_method: false,
                    access: cfml_common::dynamic::CfmlAccess::Public,
                    metadata: Vec::new(),
                    is_generated_accessor: false,
                    output_suppressed: false,
                    is_template_frame: false,
            chain_tier: 0,
            slot_names: Vec::new(),
                };

                let global_id = bc_func.global_id as usize;
                self.push_function(bc_func);
                instructions.push(BytecodeOp::DefineFunction(global_id));
                self.current_fn_local_mode = prev_fn_local_mode;
                self.local_scope_depth -= 1;
            }
            Expression::ArrowFunction(arrow) => {
                // Arrow functions inherit enclosing function's mode too
                // (they have no attribute syntax of their own).
                let arrow_effective = self.current_fn_local_mode;
                let prev_fn_local_mode = self.current_fn_local_mode;
                self.current_fn_local_mode = arrow_effective;
                // A closure/arrow body owns a `local` scope, exactly like a
                // declared function's. Tracked SEPARATELY from `function_depth`
                // (which closures deliberately do not bump — it also gates the
                // `variables.foo` → LoadVariablesKey peephole, and changing that
                // for closure bodies broke `attributes` resolution inside a
                // cfthread body nested in a custom tag).
                self.local_scope_depth += 1;
                // Function boundary: isolate finally/loop stacks for the body.
                let saved_finally = std::mem::take(&mut self.finally_stack);
                let saved_loops = std::mem::take(&mut self.loop_stack);
                let saved_tag_pairs = std::mem::take(&mut self.tag_pair_stack);
        let saved_catch_vars = std::mem::take(&mut self.catch_var_stack);
                let mut func_instructions = Vec::new();
                // Emit default parameter value preamble for arrow functions.
                // Uses JumpIfArgPresent (arguments-scope presence) for the same
                // reason as closures/named functions — see GitHub #255 / #240.
                for (idx, param) in arrow.params.iter().enumerate() {
                    if let Some(ref default_expr) = param.default {
                        let jump_idx = func_instructions.len();
                        func_instructions.push(BytecodeOp::JumpIfArgPresent(Name::from(&param.name), 0));
                        self.compile_expression(default_expr, &mut func_instructions);
                        // Seed the local AND the `arguments` key from the default, WITHOUT reading
                        // the parameter back by bare name. A `LoadLocal(param.name)` read-back is
                        // wrong for a parameter named after a built-in scope: since GH #312 a bare
                        // scope name always resolves to the SCOPE, so `function f( cookie = "D" )`
                        // seeded `arguments.cookie` with the live cookie scope instead of "D".
                        // `Dup` keeps the freshly-evaluated value on the stack for
                        // `SeedArgumentKey`, which consumes it: the local is stored by name
                        // (so slot behaviour is untouched) and the frame's OWN `arguments`
                        // scope gets the same value. Emitting `LoadLocal("arguments")` here
                        // is what used to force the whole function onto the eager path.
                        func_instructions.push(BytecodeOp::Dup);
                        func_instructions.push(BytecodeOp::StoreLocal(Name::from(&param.name)));
                        func_instructions.push(BytecodeOp::SeedArgumentKey(Name::from(&param.name)));
                        // Type-check the applied default (see compile_function_decl).
                        if declared_type_is_checkable(param.param_type.as_deref()) {
                            func_instructions.push(BytecodeOp::ValidateParamType(idx));
                        }
                        func_instructions[jump_idx] =
                            BytecodeOp::JumpIfArgPresent(Name::from(&param.name), func_instructions.len());
                    }
                }
                self.compile_expression(&arrow.body, &mut func_instructions);
                func_instructions.push(BytecodeOp::Return);
                self.finally_stack = saved_finally;
                self.loop_stack = saved_loops;
                self.tag_pair_stack = saved_tag_pairs;
        self.catch_var_stack = saved_catch_vars;

                let func_name = format!("__arrow_{}", self.program.functions.len());
                let bc_func = BytecodeFunction {
                    name: func_name.clone(),
                    params: arrow.params.iter().map(|p| p.name.clone()).collect(),
                    param_keys: Default::default(),
                    args_needed: Default::default(),
                    args_never_escapes: Default::default(),
                    params_marker: Default::default(),
                    cfc_body: Default::default(),
                    required_params: arrow.params.iter().map(|p| p.required).collect(),
                    has_default: arrow.params.iter().map(|p| p.default.is_some()).collect(),
                    instructions: func_instructions,
                    source_file: self.source_file.clone(),
                    global_id: next_global_fn_id(),
                    declared_local_mode: arrow_effective,
                    param_types: arrow.params.iter().map(|p| p.param_type.clone()).collect(),
                    return_type: None,
                    param_annotations: arrow.params.iter().map(|p| p.annotations.clone()).collect(),
                    is_component_method: false,
                    access: cfml_common::dynamic::CfmlAccess::Public,
                    metadata: Vec::new(),
                    is_generated_accessor: false,
                    output_suppressed: false,
                    is_template_frame: false,
            chain_tier: 0,
            slot_names: Vec::new(),
                };

                let global_id = bc_func.global_id as usize;
                self.push_function(bc_func);
                instructions.push(BytecodeOp::DefineFunction(global_id));
                self.current_fn_local_mode = prev_fn_local_mode;
                self.local_scope_depth -= 1;
            }
            Expression::This(_) => {
                instructions.push(BytecodeOp::LoadLocal(Name::intern("this")));
            }
            Expression::Super(_) => {
                instructions.push(BytecodeOp::LoadSuper);
            }
            Expression::StringInterpolation(interp) => {
                if interp.parts.is_empty() {
                    instructions.push(BytecodeOp::String(String::new()));
                } else if interp.parts.len() == 1 {
                    // Single-part interpolation: a quoted string whose ENTIRE
                    // content is one `#expr#` (or one literal). Lucee/ACF/BoxLang
                    // preserve the expression's native value/type here — e.g.
                    // `"#someStruct#"` IS the struct, not a stringified copy.
                    // Skip the empty-string Concat coercion. Multi-part
                    // interpolation below keeps the string-concat semantics.
                    self.compile_expression(&interp.parts[0], instructions);
                } else {
                    // Compile first part
                    self.compile_expression(&interp.parts[0], instructions);
                    // Convert to string via Concat with empty string if needed
                    if !matches!(&interp.parts[0], Expression::Literal(Literal { value: LiteralValue::String(_), .. })) {
                        instructions.push(BytecodeOp::String(String::new()));
                        instructions.push(BytecodeOp::Concat);
                    }
                    // Concat remaining parts
                    for part in &interp.parts[1..] {
                        self.compile_expression(part, instructions);
                        instructions.push(BytecodeOp::Concat);
                    }
                }
            }
            Expression::Elvis(elvis) => {
                // Elvis operator: left ?: right
                // Eval left, if not null use it, otherwise eval right
                // JumpIfNotNull peeks without popping, so no Dup needed
                //
                // Lucee's `?:` absorbs ANY exception raised while evaluating the
                // left operand — not merely an undefined read — so a defensive
                // one-liner like `getBaseTagData(n, i).attributes.marker ?: "d"`
                // yields the default when the CALL throws. Tolerant member reads
                // alone are not enough: the throw can come from a function call
                // anywhere inside the operand, including nested in an argument
                // (`len(boom()) ?: "d"`). Guard the whole operand with the
                // try/catch pair and route the exception path to the default.
                // GH #329. (The `?:`-less form still propagates — the guard
                // covers the left operand only.)
                let fallible = !Self::elvis_left_is_infallible(&elvis.left);
                let try_start_idx = instructions.len();
                if fallible {
                    instructions.push(BytecodeOp::TryStart(0)); // placeholder -> handler
                }

                // Use TryLoadLocal for simple identifiers (undefined vars → Null, not error)
                if let Expression::Identifier(ref ident) = *elvis.left {
                    instructions.push(BytecodeOp::TryLoadLocal(Name::from(&ident.name)));
                } else if matches!(
                    *elvis.left,
                    Expression::MemberAccess(_) | Expression::ArrayAccess(_)
                ) {
                    // `a.b.c ?: d` must read a missing member/link as Null (so the
                    // default applies) rather than throwing on the genuine miss.
                    self.compile_member_read_tolerant(&elvis.left, instructions);
                } else {
                    self.compile_expression(&elvis.left, instructions);
                }
                if fallible {
                    instructions.push(BytecodeOp::TryEnd);
                }

                let jump_idx = instructions.len();
                instructions.push(BytecodeOp::JumpIfNotNull(0)); // placeholder -> end
                // Left is null: pop the null and fall through to the default.
                instructions.push(BytecodeOp::Pop);
                let jump_to_default = if fallible {
                    // Skip the exception handler on the null path.
                    let idx = instructions.len();
                    instructions.push(BytecodeOp::Jump(0)); // placeholder -> default
                    // Exception handler. The unwind truncated the operand stack to
                    // the TryStart depth and pushed the exception value; drop it and
                    // evaluate the default in its place.
                    instructions[try_start_idx] = BytecodeOp::TryStart(instructions.len());
                    instructions.push(BytecodeOp::Pop);
                    Some(idx)
                } else {
                    None
                };
                if let Some(idx) = jump_to_default {
                    instructions[idx] = BytecodeOp::Jump(instructions.len());
                }
                self.compile_expression(&elvis.right, instructions);
                instructions[jump_idx] = BytecodeOp::JumpIfNotNull(instructions.len());
            }
            Expression::NamedArgument(named) => {
                // Named arguments are handled at the call site; if we get here
                // in a non-call context, just compile the value
                self.compile_expression(&named.value, instructions);
            }
            Expression::Spread(inner) => {
                // Spread in a general context just compiles the inner expression
                self.compile_expression(inner, instructions);
            }
            Expression::Empty => {
                instructions.push(BytecodeOp::Null);
            }
        }
    }
}

impl Default for CfmlCompiler {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod size_probe {
    //! PR-0 size probe (RustCFML performance plan). `BytecodeOp` is the icache
    //! cost: a 500-instruction function body at 64 B is 32 KB; shrinking the op
    //! (interned `u32` identifier ids instead of `String` payloads) targets L1.
    //!
    //! Run with: `cargo test -p cfml-codegen size_probe -- --nocapture`
    use super::*;
    use std::mem::size_of;

    #[test]
    fn report_sizes() {
        let op = size_of::<BytecodeOp>();
        eprintln!("size_of::<BytecodeOp>() = {op} B");
        assert!(
            op <= 48,
            "BytecodeOp grew to {op} B (ceiling 48 B, set when Phase 3.1 name \
             interning shrank it from 64 B) — a perf regression. If \
             intentional, justify and raise the ceiling."
        );
    }
}

#[cfg(test)]
mod slot_tests {
    use super::*;

    fn compile_named(src: &str, name: &str) -> BytecodeFunction {
        let ast = cfml_compiler::parser::Parser::new(src.to_string())
            .parse()
            .expect("parse");
        let program = CfmlCompiler::new().compile(ast);
        program
            .functions
            .iter()
            .find(|f| f.name.eq_ignore_ascii_case(name))
            .expect("function present")
            .as_ref()
            .clone()
    }

    #[test]
    fn var_locals_get_slots() {
        let f = compile_named(
            "function sumTo(n) { var t = 0; for (var i = 1; i <= n; i++) { t = t + i; } return t; }",
            "sumTo",
        );
        assert_eq!(f.slot_names.len(), 2, "t and i should be slotted");
        assert!(f
            .instructions
            .iter()
            .any(|op| matches!(op, BytecodeOp::StoreSlot(..))));
        assert!(f
            .instructions
            .iter()
            .any(|op| matches!(op, BytecodeOp::IncrementSlot(..))));
        // Const-bound loops take the fused super-instruction — slot twin form.
        let g = compile_named(
            "function sumK() { var t = 0; for (var i = 1; i <= 100; i++) { t = t + i; } return t; }",
            "sumK",
        );
        assert!(g
            .instructions
            .iter()
            .any(|op| matches!(op, BytecodeOp::ForSlotStep(..))));
        // No leftover named twins for the slotted names.
        assert!(!f.instructions.iter().any(|op| matches!(
            op,
            BytecodeOp::DeclareLocal(_) | BytecodeOp::ForLoopStep(..)
        )));
    }

    #[test]
    fn closure_defining_fn_is_ineligible() {
        let f = compile_named(
            "function outer() { var t = 1; var cl = function() { return 2; }; return t; }",
            "outer",
        );
        assert!(f.slot_names.is_empty(), "DefineFunction disqualifies");
    }

    /// Stage 1.5: writes through the explicit `local.` prefix must not
    /// materialize/merge the whole `local` scope view. `local.i++` used to emit
    /// LoadLocal("local") + SetProperty + StoreLocal("local"), whose whole-scope
    /// merge also permanently deactivated the frame's slots.
    #[test]
    fn local_prefixed_member_writes_use_slots() {
        let f = compile_named(
            "function f() { local.i = 1; local.i++; local.t = 0; local.t += 2; return local.i; }",
            "f",
        );
        assert!(
            f.slot_names.iter().any(|n| n.lower() == "i")
                && f.slot_names.iter().any(|n| n.lower() == "t"),
            "local.i / local.t should be slotted, got {:?}",
            f.slot_names
        );
        // No whole-scope view load or merge left behind.
        assert!(
            !f.instructions.iter().any(|op| matches!(
                op,
                BytecodeOp::LoadLocal(n) | BytecodeOp::StoreLocal(n) if n.lower() == "local"
            )),
            "`local` scope view still materialized: {:?}",
            f.instructions
        );
        assert!(f
            .instructions
            .iter()
            .any(|op| matches!(op, BytecodeOp::StoreSlot(..))));
        assert!(f
            .instructions
            .iter()
            .any(|op| matches!(op, BytecodeOp::LoadSlotKey(..) | BytecodeOp::TryLoadSlotKey(..))));
    }

    /// A nested `local.a.b` write still needs the parent read + set, but the
    /// `local.a` link itself resolves through the frame (slot twin), never
    /// through a materialized scope copy.
    #[test]
    fn nested_local_prefixed_write_reads_frame_key() {
        let f = compile_named(
            "function f() { local.a = { b = 1 }; local.a.b++; return local.a.b; }",
            "f",
        );
        assert!(f.slot_names.iter().any(|n| n.lower() == "a"));
        assert!(!f.instructions.iter().any(|op| matches!(
            op,
            BytecodeOp::LoadLocal(n) | BytecodeOp::StoreLocal(n) if n.lower() == "local"
        )));
        assert!(f
            .instructions
            .iter()
            .any(|op| matches!(op, BytecodeOp::TryLoadSlotKey(..))));
    }

    #[test]
    fn scope_names_and_params_never_slotted() {
        let f = compile_named(
            "function f(p) { var request = 1; var p2 = p; return p2; }",
            "f",
        );
        assert!(f.slot_names.iter().all(|n| n.lower() != "request"));
        assert!(f.slot_names.iter().all(|n| n.lower() != "p"));
    }
}
