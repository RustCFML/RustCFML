//! CFML Code Generator - AST to bytecode

use cfml_compiler::ast::*;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

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
    /// Stack of (break_placeholder_indices, continue_placeholder_indices, is_loop)
    /// for loops and `switch` blocks. `is_loop` is true for real loops and false
    /// for `switch`: `break` targets the nearest frame (loop OR switch, C-style),
    /// but `continue` must skip `switch` frames and target the enclosing loop.
    loop_stack: Vec<(Vec<usize>, Vec<usize>, bool)>,
    /// Stack of enclosing `finally` bodies (one entry per enclosing
    /// try-with-finally / `lock {}`, innermost last). A `return` must run ALL of
    /// them (innermost first) before the Return op exits the function, since the
    /// runtime Return op does not run finallys; a `rethrow` in a catch runs the
    /// innermost (its own try's) finally before propagating.
    finally_stack: Vec<Vec<Statement>>,
    /// Nesting depth of function-body compilation. 0 means page-scope; inside any
    /// UDF or CFC method this is > 0. Used to gate the `variables.x` peephole:
    /// at page scope `variables.x` is a read of globals (LoadGlobal semantics),
    /// but inside a function body `variables` refers to the local-scope merge or
    /// a CFC's `__variables` struct — different semantics entirely.
    function_depth: usize,
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

#[derive(Debug, Clone)]
pub struct BytecodeProgram {
    pub functions: Vec<Arc<BytecodeFunction>>,
}

#[derive(Debug, Clone)]
pub struct BytecodeFunction {
    pub name: String,
    pub params: Vec<String>,
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
    LoadLocal(String),
    StoreLocal(String),
    /// Fused arrayAppend(<ident>, value): pops the value off the stack and
    /// appends it to the array held in the named variable, in place. Avoids the
    /// LoadLocal/clone + builtin call + StoreLocal round-trip whose Arc aliasing
    /// makes a loop of appends O(n²) (every `make_mut` deep-clones the backing
    /// Vec). Emitted only for a 2-arg call with a simple, non-scope identifier.
    ArrayAppendLocal(String),
    LoadGlobal(String),
    /// Page-scope `variables.foo` read peephole. Same locals-then-globals
    /// resolution chain as LoadGlobal, but READ position: a plain data value
    /// is always returned as-is. LoadGlobal is otherwise emitted only in
    /// call position, where data inherited from an ancestor frame (and data
    /// under a builtin name) must stay invisible to function-name
    /// resolution (PR #97) — semantics that would corrupt reads of
    /// variables named like builtins (`variables.log`, `variables.len`).
    LoadVariablesKey(String),
    StoreGlobal(String),

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
    JumpIfLocalCmpConstFalse(String, i64, CmpOp, usize),
    /// For-loop step super-instruction: `locals[name] += step; if (locals[name] CMP const) jump target`.
    /// Fuses Increment + LoadLocal + Integer + Cmp + JumpIfFalse-style test into one
    /// dispatch. `step` is +1 (for `i++`) or -1 (for `i--`). The jump fires on the
    /// TRUE arm (back to body); falling through means the loop has finished.
    ForLoopStep(String, i64, CmpOp, i64, usize),
    Call(usize),
    Return,

    // Collections
    BuildArray(usize),   // Build array from top N stack items
    BuildStruct(usize),  // Build struct from top N key-value pairs
    GetIndex,            // Get array[index] or struct[key]
    SetIndex,            // Set array[index] = value or struct[key] = value
    GetProperty(String), // Get object.property — THROWS "Variable '<name>' is undefined" on a genuine miss (Lucee/ACF parity)
    /// Null-tolerant twin of GetProperty: a missing struct/component member reads
    /// as Null instead of throwing. Emitted only in contexts that must tolerate a
    /// missing member — the `?:` (Elvis) left operand, `isNull()`, null-safe `?.`,
    /// and nested-write auto-vivification bases (`q["a"]["b"] = v` with `q` unset).
    TryGetProperty(String),
    /// Push the `super` receiver for the CURRENTLY EXECUTING method, resolved
    /// relative to that method's defining class (not the leaf instance). Reads
    /// `this.__super_map[<defining source>]` keyed by the active `source_file`,
    /// falling back to `this.__super`. Fixes multi-level `super.method()` so an
    /// intermediate class's method reaches ITS parent rather than recursing.
    LoadSuper,
    /// Push the static "holder" (a cached, lazily-built template instance whose
    /// `__variables.__static` is the shared static scope) for a named component.
    /// Used by the `::` operator to reach static members without instantiating.
    LoadStaticHolder(String),
    /// Pop a static holder (or any component value) and push the named member of
    /// its static scope (`Component::member`). Pushes Null if absent.
    GetStaticProperty(String),
    /// Fused LoadLocal(name) + GetProperty(member) — reads a struct field from a
    /// named local in one dispatch. Only emitted for non-null-safe accesses
    /// where the receiver is a plain identifier (the common `s.foo` pattern).
    LoadLocalProperty(String, String),
    /// Fused LoadLocal(name) + SetProperty(member) — stores a value into a struct
    /// field of a named local in one dispatch. Only emitted for non-null-safe
    /// accesses where the receiver is a plain identifier (the common `s.foo = x` pattern).
    StoreLocalProperty(String, String),
    /// Fused LoadLocal("local") + GetProperty(member) for an explicit `local.foo`
    /// read. The generic path materializes the ENTIRE per-call `local` scope view
    /// (cloning every visible key+value into a fresh struct) just to extract one
    /// key — profiling stock Wheels showed `build_local_scope_view` was ~35% of
    /// request allocations. This op reads the single member directly from the
    /// frame's `locals`, applying the same per-call visibility filter
    /// (`build_local_scope_view`): inherited/param keys, `this`/`super`, and
    /// `__`-prefixed bridge keys are invisible; a miss yields Null (matching
    /// GetProperty on the materialized view).
    LoadLocalKey(String),
    /// Null-tolerant twins of the fused read ops (see TryGetProperty). A missing
    /// receiver variable OR a missing member reads as Null instead of throwing.
    TryLoadLocalProperty(String, String),
    TryLoadLocalKey(String),
    SetProperty(String), // Set object.property = value
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
    DeleteScopeKey(String),

    // Object
    NewObject(usize),  // arg_count for constructor
    // arg_count for constructor + call-site argument names (empty string = positional).
    // Used when `new X(...)` supplies named arguments so init() binds by name, not position.
    NewObjectNamed(Vec<String>, usize),

    // Function definition
    DefineFunction(usize), // BytecodeFunction.global_id (resolved via the VM's fn_registry)

    // Postfix ops
    Increment(String),  // Increment variable (+1)
    Decrement(String),  // Decrement variable (-1)
    AddLocalConst(String, i64),  // Add constant to local: i += K or i = i + K
    MulLocalConst(String, i64),  // Multiply local by constant: i *= K

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
    // Peek the exception value on top of the stack (does NOT consume it) and
    // push a boolean: does its `type` match this catch clause's declared type?
    // "any"/empty matches everything; otherwise case-insensitive exact match or
    // dotted-hierarchy prefix (catch "Foo" also catches "Foo.Bar").
    CatchMatch(String),

    // Method call: object is on stack, then args, method name + arg count
    // Optional write-back: (object_var, Option<property_name>)
    //   - Some(vec!["dog"]) for dog.method() — write modified this back to dog
    //   - Some(vec!["this", "items"]) for this.items.method() — write result back to this.items
    //   - Some(vec!["local", "_taffy", "factory"]) for local._taffy.factory.method()
    //   - None — no write-back needed
    CallMethod(String, usize, Option<Vec<String>>),
    // Method call with named arguments: like CallMethod but carries the
    // call-site argument names (empty string for positional args), so the VM
    // can rebind them to the resolved method's parameters by name. Mirrors
    // CallNamed for free-function calls. The names are boxed so this variant
    // does not grow BytecodeOp past its size ceiling (it is the rare path).
    CallMethodNamed(String, Box<Vec<String>>, usize, Option<Vec<String>>),

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

    // Null handling
    IsNull,                // Pop value, push bool (true if Null)
    JumpIfNotNull(usize),  // Pop value, jump if not null (pushes value back)

    // Output
    Print,
    Halt,

    // Variable existence check
    IsDefined(String),

    // Spread operator support
    ConcatArrays,
    MergeStructs,
    CallSpread,

    // Source location tracking
    LineInfo(usize, usize),  // (line, column) — emitted before statements for stack traces

    // Safe variable load: returns Null for undefined vars (used by Elvis, null-safe, isNull)
    TryLoadLocal(String),

    // Declare a variable as function-local (var keyword) — prevents writeback to parent scope
    DeclareLocal(String),

    // Named function call: like Call but carries argument names for name-to-param mapping
    // (names, arg_count) — names[i] corresponds to the i-th arg on the stack
    CallNamed(Vec<String>, usize),

    // Explicit super(args) constructor call for a CFC whose parent is a Rust class.
    // Pops arg_count values, looks up the constructor registered under
    // this.__rust_extends, calls it, and stores the new NativeObject on
    // this.__super (replacing any default-constructed one). Pushes Null.
    CallRustSuperCtor(usize),
}

impl CfmlCompiler {
    pub fn new() -> Self {
        Self {
            program: BytecodeProgram {
                functions: vec![Arc::new(BytecodeFunction {
                    name: "__main__".to_string(),
                    params: Vec::new(),
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
                })],
            },
            loop_stack: Vec::new(),
            finally_stack: Vec::new(),
            function_depth: 0,
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
                instructions.push(BytecodeOp::StoreLocal(fd.func.name.clone()));
            } else {
                self.compile_node(node, &mut instructions);
            }
        }

        instructions.push(BytecodeOp::Halt);

        Arc::get_mut(&mut self.program.functions[0]).unwrap().instructions = instructions;

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
    fn is_mutating_standalone_call(expr: &Expression) -> bool {
        if let Expression::FunctionCall(call) = expr {
            if let Expression::Identifier(ident) = &*call.name {
                let name_lower = ident.name.to_lowercase();
                // NB: structDelete is intentionally absent — it mutates the
                // shared struct handle in place AND returns a BOOLEAN (Lucee/ACF
                // semantics), so storing its return value back over the first
                // arg would clobber the struct variable with `true`/`false`.
                return matches!(name_lower.as_str(),
                    "structappend" | "structinsert" | "structupdate" |
                    "structclear" | "arrayclear" | "arrayappend" | "arrayprepend" |
                    "arrayinsert" | "arrayinsertat" | "arraydeleteat" | "arraysort" |
                    "arrayresize" | "arrayswap" | "arrayreverse" | "arrayset" |
                    "queryaddcolumn" |
                    "querydeleterow" | "querydeletecolumn" | "querysort"
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
                    Some(format!("{}.{}", ident.name, member))
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
                    Some(format!("{}.{}", ident.name, access.member))
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
                instructions.push(BytecodeOp::TryLoadLocal(ident.name.clone()));
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
                instructions.push(BytecodeOp::TryGetProperty(access.member.clone()));
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
                instructions.push(BytecodeOp::LoadLocal(name.clone()));
            }
            AssignTarget::StructAccess(obj, member) => {
                // Compound assign (`s.x += 1`) reads the current value; preserve the
                // pre-existing Null-on-miss behaviour (treated as 0/"") rather than
                // throwing when the target member doesn't exist yet.
                self.compile_expression(obj, instructions);
                instructions.push(BytecodeOp::TryGetProperty(member.clone()));
            }
            AssignTarget::ArrayAccess(arr, idx) => {
                self.compile_expression(arr, instructions);
                self.compile_expression(idx, instructions);
                instructions.push(BytecodeOp::GetIndex);
            }
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
                instructions.push(BytecodeOp::TryLoadLocal(ident.name.clone()));
            }
            Expression::MemberAccess(access) if !access.null_safe => {
                // Mirror the normal read path's fusion peepholes so the JIT sees
                // the same op shapes it already specialises — just the Null-
                // tolerant twins (`local.foo` → TryLoadLocalKey, `<ident>.foo` →
                // TryLoadLocalProperty). Reserved-scope and nested roots recurse
                // then TryGetProperty.
                if let Expression::Identifier(ref ident) = *access.object {
                    if ident.name.eq_ignore_ascii_case("local") {
                        instructions.push(BytecodeOp::TryLoadLocalKey(access.member.clone()));
                        return;
                    }
                    if !is_reserved_scope_name(&ident.name) {
                        instructions.push(BytecodeOp::TryLoadLocalProperty(
                            ident.name.clone(),
                            access.member.clone(),
                        ));
                        return;
                    }
                }
                self.compile_member_read_tolerant(&access.object, instructions);
                instructions.push(BytecodeOp::TryGetProperty(access.member.clone()));
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
                instructions.push(BytecodeOp::StoreLocal(ident.name.clone()));
            }
            Expression::This(_) => {
                instructions.push(BytecodeOp::StoreLocal("this".to_string()));
            }
            Expression::MemberAccess(access) => {
                // Stack has modified child value. Load the parent, swap, set property.
                // Then recurse to write back the parent.
                self.emit_load_for_writeback(&access.object, instructions);
                instructions.push(BytecodeOp::Swap);
                instructions.push(BytecodeOp::SetProperty(access.member.clone()));
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
                    instructions.push(BytecodeOp::LoadLocal(ident.name.clone()));
                } else {
                    instructions.push(BytecodeOp::TryLoadLocal(ident.name.clone()));
                }
            }
            Expression::This(_) => {
                instructions.push(BytecodeOp::LoadLocal("this".to_string()));
            }
            Expression::MemberAccess(access) => {
                // For nested access like loading "s.a", we load s then get property a.
                // Write-back reads a not-yet-existing link as Null (auto-viv), so the
                // Try* twin — a throwing GetProperty would abort `s.a.b = v` when
                // `s.a` doesn't exist yet.
                self.emit_load_for_writeback(&access.object, instructions);
                instructions.push(BytecodeOp::TryGetProperty(access.member.clone()));
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
                if matches!(&expr_stmt.expr, Expression::Identifier(_)) {
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
                    instructions.push(BytecodeOp::DeleteScopeKey(scope));
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
                                instructions.push(BytecodeOp::ArrayAppendLocal(ident.name.clone()));
                            }
                            Expression::Identifier(ident) => {
                                // Simple: structAppend(a, b) → compile call → StoreLocal(a)
                                self.compile_expression(&expr_stmt.expr, instructions);
                                instructions.push(BytecodeOp::StoreLocal(ident.name.clone()));
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
                let name = match var.name.to_lowercase().strip_prefix("local.") {
                    Some(rest) if !rest.contains('.') => var.name[6..].to_string(),
                    _ => var.name.clone(),
                };
                instructions.push(BytecodeOp::DeclareLocal(name.clone()));
                if let Some(value) = &var.value {
                    self.compile_expression(value, instructions);
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
                        instructions.push(BytecodeOp::StoreLocal(name.clone()));
                        instructions[end_idx] = BytecodeOp::Jump(instructions.len());
                    } else {
                        instructions.push(BytecodeOp::StoreLocal(name.clone()));
                    }
                } else {
                    instructions.push(BytecodeOp::Null);
                    instructions.push(BytecodeOp::StoreLocal(name.clone()));
                }
            }
            Statement::Assignment(assign) => {
                // Hot-path: x += <int>, x -= <int>, x *= <int> compile to a single
                // load-compute-store op inside locals. No stack traffic, no trailing
                // StoreLocal.
                if let AssignTarget::Variable(name) = &assign.target {
                    if let Some(k) = int_lit(&assign.value) {
                        let op = match &assign.operator {
                            AssignOp::PlusEqual  => Some(BytecodeOp::AddLocalConst(name.clone(),  k)),
                            AssignOp::MinusEqual => Some(BytecodeOp::AddLocalConst(name.clone(), -k)),
                            AssignOp::StarEqual  => Some(BytecodeOp::MulLocalConst(name.clone(),  k)),
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
                        instructions.push(BytecodeOp::StoreLocal(name.clone()));
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
                                instructions.push(BytecodeOp::StoreLocalProperty(
                                    ident.name.clone(),
                                    member.clone(),
                                ));
                            } else if ident.name.eq_ignore_ascii_case("local") {
                                // `local.X = v` is identical to `var X = v` in CFML —
                                // function-frame scope, must NOT propagate to caller at
                                // return. Compile to DeclareLocal + StoreLocal so the
                                // classic-localmode writeback loop skips it (same as `var`).
                                instructions.push(BytecodeOp::DeclareLocal(member.clone()));
                                instructions.push(BytecodeOp::StoreLocal(member.clone()));
                            } else {
                                self.compile_expression(obj, instructions);
                                instructions.push(BytecodeOp::Swap);
                                instructions.push(BytecodeOp::SetProperty(member.clone()));
                                self.emit_nested_writeback(obj, instructions);
                            }
                        } else {
                            // obj is itself an index/member chain (`q["a"].foo = v`).
                            // Use the auto-vivifying base compile so an undefined
                            // root reads as Null (and SetProperty/the write-back
                            // build the chain) rather than throwing.
                            self.compile_index_assign_base(obj, instructions);
                            instructions.push(BytecodeOp::Swap);
                            instructions.push(BytecodeOp::SetProperty(member.clone()));
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
                    self.compile_expression(value, instructions);
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
                    instructions.push(BytecodeOp::StoreLocal("__cf_finally_retval".to_string()));
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
                    instructions.push(BytecodeOp::LoadLocal("__cf_finally_retval".to_string()));
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
                // Push a placeholder jump that will be patched later
                let idx = instructions.len();
                instructions.push(BytecodeOp::Jump(0)); // placeholder
                if let Some(loop_ctx) = self.loop_stack.last_mut() {
                    loop_ctx.0.push(idx); // break indices
                }
            }
            Statement::Continue(_) => {
                let idx = instructions.len();
                instructions.push(BytecodeOp::Jump(0)); // placeholder
                // `continue` targets the enclosing LOOP, not a `switch` it sits
                // inside (a switch has no loop semantics). Skip switch frames.
                if let Some(loop_ctx) = self.loop_stack.iter_mut().rev().find(|c| c.2) {
                    loop_ctx.1.push(idx); // continue indices
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
            instructions.push(BytecodeOp::JumpIfLocalCmpConstFalse(name, c, cmp, 0));
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
                            instructions.push(BytecodeOp::Increment(ident.name.clone()));
                            return true;
                        }
                        PostfixOpType::Decrement => {
                            instructions.push(BytecodeOp::Decrement(ident.name.clone()));
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
                            instructions.push(BytecodeOp::Increment(ident.name.clone()));
                            return true;
                        }
                        UnaryOpType::PrefixDecrement => {
                            instructions.push(BytecodeOp::Decrement(ident.name.clone()));
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
                instructions.push(BytecodeOp::JumpIfLocalCmpConstFalse(name, c, cmp, 0));
                idx
            } else {
                self.compile_expression(condition, instructions);
                let idx = instructions.len();
                instructions.push(BytecodeOp::JumpIfFalse(0));
                idx
            };

            self.loop_stack.push((Vec::new(), Vec::new(), true));

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

            let (break_indices, continue_indices, _) = self.loop_stack.pop().unwrap();
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
        instructions.push(BytecodeOp::JumpIfLocalCmpConstFalse(
            name.to_string(), limit, cmp, 0,
        ));

        let body_start = instructions.len();

        self.loop_stack.push((Vec::new(), Vec::new(), true));

        for s in body {
            self.compile_statement(s, instructions);
        }

        // continue target = the step — continue runs the step, then re-tests.
        let continue_target = instructions.len();
        instructions.push(BytecodeOp::ForLoopStep(
            name.to_string(), limit, cmp, step, body_start,
        ));

        let loop_end = instructions.len();

        // Patch the entry-check to exit to loop_end if condition initially false.
        if let BytecodeOp::JumpIfLocalCmpConstFalse(_, _, _, off) =
            &mut instructions[entry_check_idx]
        {
            *off = loop_end;
        }

        let (break_indices, continue_indices, _) = self.loop_stack.pop().unwrap();
        for idx in break_indices {
            instructions[idx] = BytecodeOp::Jump(loop_end);
        }
        for idx in continue_indices {
            instructions[idx] = BytecodeOp::Jump(continue_target);
        }
    }

    fn compile_for_in(&mut self, for_in: &ForIn, instructions: &mut Vec<BytecodeOp>) {
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
        instructions.push(BytecodeOp::DeclareLocal(iter_var.clone()));
        instructions.push(BytecodeOp::DeclareLocal(idx_var.clone()));
        instructions.push(BytecodeOp::DeclareLocal(limit_var.clone()));
        instructions.push(BytecodeOp::StoreLocal(iter_var.clone()));

        // Hoist len(iterable) out of the loop. The old codegen looked up the
        // `len` builtin and invoked it every iteration — a HashMap probe plus
        // full function-call trampoline per element. Compute once, reuse.
        instructions.push(BytecodeOp::LoadGlobal("len".to_string()));
        instructions.push(BytecodeOp::LoadLocal(iter_var.clone()));
        instructions.push(BytecodeOp::Call(1));
        instructions.push(BytecodeOp::StoreLocal(limit_var.clone()));

        // CFML arrays are 1-based, so start index at 1.
        instructions.push(BytecodeOp::Integer(1));
        instructions.push(BytecodeOp::StoreLocal(idx_var.clone()));

        let loop_start = instructions.len();

        // Condition: idx <= limit  (both locals; no builtin call per iter).
        instructions.push(BytecodeOp::LoadLocal(idx_var.clone()));
        instructions.push(BytecodeOp::LoadLocal(limit_var.clone()));
        instructions.push(BytecodeOp::Lte);

        let jump_false_idx = instructions.len();
        instructions.push(BytecodeOp::JumpIfFalse(0));

        // Set loop variable = iterable[idx]
        instructions.push(BytecodeOp::LoadLocal(iter_var.clone()));
        instructions.push(BytecodeOp::LoadLocal(idx_var.clone()));
        instructions.push(BytecodeOp::GetIndex);
        // Strip a leading `local.` prefix from the loop variable so it stores
        // as a simple local rather than a literal key containing a dot. A
        // subsequent `local.X` read resolves to that local via the normal
        // locals lookup.
        let loop_var_name = if let Some(rest) = for_in.variable.strip_prefix("local.") {
            rest.to_string()
        } else {
            for_in.variable.clone()
        };
        if loop_var_name.contains('.') {
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
                instructions.push(BytecodeOp::StoreLocalProperty(
                    segments[0].clone(),
                    segments[1].clone(),
                ));
            } else {
                let root = segments[0].clone();
                let leaf = segments[segments.len() - 1].clone();
                let intermediate = &segments[1..segments.len() - 1];
                // Stack on entry: [element_value]
                // Load deepest parent: root[.intermediate[0]...intermediate[n]]
                instructions.push(BytecodeOp::LoadLocal(root.clone()));
                for seg in intermediate {
                    instructions.push(BytecodeOp::TryGetProperty(seg.clone()));
                }
                // Stack: [element_value, deepest_parent]
                instructions.push(BytecodeOp::Swap);
                instructions.push(BytecodeOp::SetProperty(leaf));
                // Stack: [modified_deepest_parent]
                // Unwind: for each intermediate level (deepest -> shallowest),
                // reload its parent and SetProperty back in.
                for i in (0..intermediate.len()).rev() {
                    instructions.push(BytecodeOp::LoadLocal(root.clone()));
                    for seg in &intermediate[..i] {
                        instructions.push(BytecodeOp::TryGetProperty(seg.clone()));
                    }
                    instructions.push(BytecodeOp::Swap);
                    instructions.push(BytecodeOp::SetProperty(intermediate[i].clone()));
                }
                // Stack: [modified_root]
                instructions.push(BytecodeOp::StoreLocal(root));
            }
        } else {
            instructions.push(BytecodeOp::DeclareLocal(loop_var_name.clone()));
            instructions.push(BytecodeOp::StoreLocal(loop_var_name));
        }

        self.loop_stack.push((Vec::new(), Vec::new(), true));

        for s in &for_in.body {
            self.compile_statement(s, instructions);
        }

        let continue_target = instructions.len();

        // idx++  (single Increment op, not Load+Int+Add+Store).
        instructions.push(BytecodeOp::Increment(idx_var.clone()));

        instructions.push(BytecodeOp::Jump(loop_start));

        let loop_end = instructions.len();
        instructions[jump_false_idx] = BytecodeOp::JumpIfFalse(loop_end);

        let (break_indices, continue_indices, _) = self.loop_stack.pop().unwrap();
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

        self.loop_stack.push((Vec::new(), Vec::new(), true));

        for s in &while_stmt.body {
            self.compile_statement(s, instructions);
        }

        instructions.push(BytecodeOp::Jump(loop_start));

        let loop_end = instructions.len();
        Self::patch_cond_jump_target(instructions, jump_false_idx, loop_end);

        let (break_indices, continue_indices, _) = self.loop_stack.pop().unwrap();
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

        self.loop_stack.push((Vec::new(), Vec::new(), true));

        for s in &do_stmt.body {
            self.compile_statement(s, instructions);
        }

        let continue_target = instructions.len();

        self.compile_expression(&do_stmt.condition, instructions);
        instructions.push(BytecodeOp::JumpIfTrue(loop_start));

        let loop_end = instructions.len();

        let (break_indices, continue_indices, _) = self.loop_stack.pop().unwrap();
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

        self.loop_stack.push((Vec::new(), Vec::new(), true));

        for s in body {
            self.compile_statement(s, instructions);
        }

        let continue_target = instructions.len();
        instructions.push(BytecodeOp::ForLoopStep(
            name.to_string(), limit, cmp, step, body_start,
        ));

        let loop_end = instructions.len();

        let (break_indices, continue_indices, _) = self.loop_stack.pop().unwrap();
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
        instructions.push(BytecodeOp::StoreLocal(switch_var.clone()));

        self.loop_stack.push((Vec::new(), Vec::new(), false)); // break support (not a loop)

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
                instructions.push(BytecodeOp::LoadLocal(switch_var.clone()));
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
        let (break_indices, _, _) = self.loop_stack.pop().unwrap();
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
            instructions.push(BytecodeOp::CatchMatch(catch_type));
            let jump_if_no_match = instructions.len();
            instructions.push(BytecodeOp::JumpIfFalse(0)); // -> next clause's test
            // Matched: bind the exception to the catch variable (consumes it).
            instructions.push(BytecodeOp::StoreLocal(catch.var_name.clone()));
            for s in &catch.body {
                self.compile_statement(s, instructions);
            }
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

        // Emit default parameter value preamble:
        // For each param with a default, if the arg is null, assign the default
        // and also update the arguments scope
        for param in &func.params {
            if let Some(ref default_expr) = param.default {
                func_instructions.push(BytecodeOp::LoadLocal(param.name.clone()));
                func_instructions.push(BytecodeOp::IsNull);
                let jump_idx = func_instructions.len();
                func_instructions.push(BytecodeOp::JumpIfFalse(0)); // placeholder
                // Set the local variable
                self.compile_expression(default_expr, &mut func_instructions);
                func_instructions.push(BytecodeOp::StoreLocal(param.name.clone()));
                // Also update the arguments scope
                func_instructions.push(BytecodeOp::LoadLocal("arguments".to_string()));
                func_instructions.push(BytecodeOp::LoadLocal(param.name.clone()));
                func_instructions.push(BytecodeOp::SetProperty(param.name.clone()));
                func_instructions.push(BytecodeOp::StoreLocal("arguments".to_string()));
                func_instructions[jump_idx] = BytecodeOp::JumpIfFalse(func_instructions.len());
            }
        }

        for s in &func.body {
            self.compile_statement(s, &mut func_instructions);
        }

        // Ensure function returns null if no explicit return
        func_instructions.push(BytecodeOp::Null);
        func_instructions.push(BytecodeOp::Return);

        self.function_depth -= 1;
        self.current_fn_local_mode = prev_fn_local_mode;
        self.finally_stack = saved_finally;
        self.loop_stack = saved_loops;

        let bc_func = BytecodeFunction {
            name: func.name.clone(),
            params: func.params.iter().map(|p| p.name.clone()).collect(),
            required_params: func.params.iter().map(|p| p.required).collect(),
            has_default: func.params.iter().map(|p| p.default.is_some()).collect(),
            instructions: func_instructions,
            source_file: self.source_file.clone(),
            global_id: next_global_fn_id(),
            declared_local_mode: declared_mode,
            param_types: func.params.iter().map(|p| p.param_type.clone()).collect(),
            return_type: func.return_type.clone(),
            param_annotations: func.params.iter().map(|p| p.annotations.clone()).collect(),
            is_component_method: self.in_component_method,
            access: match func.access {
                AccessModifier::Private => cfml_common::dynamic::CfmlAccess::Private,
                AccessModifier::Package => cfml_common::dynamic::CfmlAccess::Package,
                AccessModifier::Remote => cfml_common::dynamic::CfmlAccess::Remote,
                AccessModifier::Public => cfml_common::dynamic::CfmlAccess::Public,
            },
            metadata: func.metadata.clone(),
        };

        let global_id = bc_func.global_id as usize;
        self.program.functions.push(Arc::new(bc_func));

        // Define the function in current scope. The op carries the function's
        // process-stable global_id, resolved by the VM through its registry.
        instructions.push(BytecodeOp::DefineFunction(global_id));
        instructions.push(BytecodeOp::StoreLocal(func.name.clone()));
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
        instructions.push(BytecodeOp::StoreLocal(interface.name.clone()));
        instructions.push(BytecodeOp::LoadLocal(interface.name.clone()));
        instructions.push(BytecodeOp::StoreGlobal(interface.name.clone()));
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
        instructions.push(BytecodeOp::StoreLocal(component.name.clone()));

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
                    required_params: Vec::new(),
                    has_default: Vec::new(),
                    instructions: vec![
                        BytecodeOp::LoadLocal("variables".to_string()),
                        BytecodeOp::TryGetProperty(prop.name.clone()),
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
                };
                self.program.functions.push(Arc::new(getter_func));
                let getter_gid = self.program.functions.last().unwrap().global_id as usize;
                instructions.push(BytecodeOp::DefineFunction(getter_gid));
                // Stack: [getter_func]

                // Add getter to component: component[getter_name] = getter_func
                // Stack: [getter_func]
                // Load component: [getter_func, component]
                // Swap: [component, getter_func]
                // SetProperty(getter_name): sets component.getter_name = getter_func, stack is [component]
                // StoreLocal: []
                instructions.push(BytecodeOp::LoadLocal(component.name.clone()));
                instructions.push(BytecodeOp::Swap);
                instructions.push(BytecodeOp::SetProperty(getter_name.clone()));
                instructions.push(BytecodeOp::StoreLocal(component.name.clone()));

                // Generate setter: setPropertyName(value)
                // Set the property directly on this struct and __variables
                let setter_name = format!("set{}", capitalize_first(&prop.name));
                let setter_func = BytecodeFunction {
                    name: setter_name.clone(),
                    params: vec![prop.name.clone()],
                    required_params: vec![true],
                    has_default: vec![false],
                    instructions: vec![
                        // Set on this: this.name = value; store modified this back
                        BytecodeOp::LoadLocal("this".to_string()),
                        BytecodeOp::LoadLocal(prop.name.clone()),
                        BytecodeOp::SetProperty(prop.name.clone()),
                        BytecodeOp::StoreLocal("this".to_string()),
                        // Set on __variables: this.__variables.name = value
                        BytecodeOp::LoadLocal("this".to_string()),
                        BytecodeOp::TryGetProperty("__variables".to_string()),
                        BytecodeOp::LoadLocal(prop.name.clone()),
                        BytecodeOp::SetProperty(prop.name.clone()),
                        BytecodeOp::StoreLocal("__variables".to_string()),
                        // Return this
                        BytecodeOp::LoadLocal("this".to_string()),
                        BytecodeOp::Return,
                    ],
                    source_file: self.source_file.clone(),
                    global_id: next_global_fn_id(),
                    declared_local_mode: None,
                    param_types: vec![None],
                    return_type: Some(component.name.clone()),
                    param_annotations: vec![Vec::new()],
                    is_component_method: true,
                    access: cfml_common::dynamic::CfmlAccess::Public,
                    metadata: Vec::new(),
                };
                self.program.functions.push(Arc::new(setter_func));
                let setter_gid = self.program.functions.last().unwrap().global_id as usize;
                instructions.push(BytecodeOp::DefineFunction(setter_gid));
                // Stack: [setter_func]

                // Add setter to component (same pattern)
                instructions.push(BytecodeOp::LoadLocal(component.name.clone()));
                instructions.push(BytecodeOp::Swap);
                instructions.push(BytecodeOp::SetProperty(setter_name.clone()));
                instructions.push(BytecodeOp::StoreLocal(component.name.clone()));
            }
        }

        // Now store as a component template in global scope (with accessors included)
        instructions.push(BytecodeOp::LoadLocal(component.name.clone()));
        instructions.push(BytecodeOp::StoreGlobal(component.name.clone()));

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
            instructions.push(BytecodeOp::LoadLocal(component.name.clone()));
            instructions.push(BytecodeOp::DefineFunction(gid));
            instructions.push(BytecodeOp::SetProperty(func.name.clone()));
            instructions.push(BytecodeOp::StoreLocal(component.name.clone()));
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
                instructions.push(BytecodeOp::LoadLocal(component.name.clone()));
                instructions.push(BytecodeOp::Swap);
                instructions.push(BytecodeOp::SetProperty(meta_key));
                instructions.push(BytecodeOp::StoreLocal(component.name.clone()));
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
                if let Some(ref pt) = prop.prop_type {
                    instructions.push(BytecodeOp::String("type".to_string()));
                    instructions.push(BytecodeOp::String(pt.clone()));
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
            instructions.push(BytecodeOp::LoadLocal(component.name.clone()));
            instructions.push(BytecodeOp::Swap);
            instructions.push(BytecodeOp::SetProperty("__properties".to_string()));
            instructions.push(BytecodeOp::StoreLocal(component.name.clone()));
        }

        // Update global copy after methods and metadata are added
        if !component.functions.is_empty() || !component.metadata.is_empty() || !component.properties.is_empty() {
            instructions.push(BytecodeOp::LoadLocal(component.name.clone()));
            instructions.push(BytecodeOp::StoreGlobal(component.name.clone()));
        }

        // Compile component body statements (e.g., this.name = "xxx", this.mappings = {...})
        // These execute as init code that modifies the component struct via `this`
        if !component.body.is_empty() {
            // Bind `this` to the component struct so `this.xxx = val` works
            instructions.push(BytecodeOp::LoadLocal(component.name.clone()));
            instructions.push(BytecodeOp::StoreLocal("this".to_string()));

            for stmt in &component.body {
                self.compile_statement(stmt, instructions);
            }

            // Copy modified `this` back to component name and global
            instructions.push(BytecodeOp::LoadLocal("this".to_string()));
            instructions.push(BytecodeOp::StoreLocal(component.name.clone()));
            instructions.push(BytecodeOp::LoadLocal(component.name.clone()));
            instructions.push(BytecodeOp::StoreGlobal(component.name.clone()));
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
            };
            self.program.functions.push(Arc::new(static_func));
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
                instructions.push(BytecodeOp::LoadLocal(id.name.clone()));
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
                            instructions.push(BytecodeOp::StoreLocal(ident.name.clone()));
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
                                    instructions.push(BytecodeOp::StoreLocalProperty(
                                        ident.name.clone(),
                                        access.member.clone(),
                                    ));
                                } else if ident.name.eq_ignore_ascii_case("local") {
                                    // `local.X = v` is identical to `var X = v` —
                                    // function-frame scope, must NOT propagate to
                                    // caller at return. Same fix as the
                                    // Statement::Assignment path above.
                                    if want_value {
                                        instructions.push(BytecodeOp::Dup);
                                    }
                                    instructions.push(BytecodeOp::DeclareLocal(access.member.clone()));
                                    instructions.push(BytecodeOp::StoreLocal(access.member.clone()));
                                } else {
                                    // SetProperty needs [obj, value].
                                    self.compile_expression(&access.object, instructions);
                                    instructions.push(BytecodeOp::Swap);
                                    instructions.push(BytecodeOp::SetProperty(access.member.clone()));
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
                                instructions.push(BytecodeOp::SetProperty(access.member.clone()));
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
                            instructions.push(BytecodeOp::LoadLocal(ident.name.clone()));
                            instructions.push(BytecodeOp::Integer(delta));
                            instructions.push(BytecodeOp::Add);
                            instructions.push(BytecodeOp::Dup);
                            instructions.push(BytecodeOp::StoreLocal(ident.name.clone()));
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
                            instructions.push(BytecodeOp::LoadLocal(ident.name.clone()));
                            instructions.push(BytecodeOp::Dup);
                            instructions.push(BytecodeOp::Integer(1));
                            instructions.push(BytecodeOp::Add);
                            instructions.push(BytecodeOp::StoreLocal(ident.name.clone()));
                            // The original value stays on the stack
                        }
                        PostfixOpType::Decrement => {
                            instructions.push(BytecodeOp::LoadLocal(ident.name.clone()));
                            instructions.push(BytecodeOp::Dup);
                            instructions.push(BytecodeOp::Integer(1));
                            instructions.push(BytecodeOp::Sub);
                            instructions.push(BytecodeOp::StoreLocal(ident.name.clone()));
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
                    instructions.push(BytecodeOp::LoadStaticHolder(name));
                } else {
                    self.compile_expression(&sm.class, instructions);
                }
                instructions.push(BytecodeOp::GetStaticProperty(sm.member.clone()));
            }
            Expression::StaticCall(sc) => {
                // `Component::method(args)` — call a static method without an
                // instance. The holder carries `__variables.__static`, so
                // `static.X` inside the method resolves through the normal chain.
                if let Some(name) = Self::static_class_name(&sc.class) {
                    instructions.push(BytecodeOp::LoadStaticHolder(name));
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
                    instructions.push(BytecodeOp::CallMethodNamed(
                        sc.method.clone(),
                        Box::new(names),
                        sc.arguments.len(),
                        None,
                    ));
                } else {
                    instructions.push(BytecodeOp::CallMethod(
                        sc.method.clone(),
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
                                .push(BytecodeOp::LoadVariablesKey(access.member.clone()));
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
                        if ident.name.eq_ignore_ascii_case("local") {
                            instructions
                                .push(BytecodeOp::LoadLocalKey(access.member.clone()));
                            return;
                        }
                        if !is_reserved_scope_name(&ident.name) {
                            instructions.push(BytecodeOp::LoadLocalProperty(
                                ident.name.clone(),
                                access.member.clone(),
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
                        instructions.push(BytecodeOp::TryLoadLocal(ident.name.clone()));
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
                    instructions.push(BytecodeOp::TryGetProperty(access.member.clone()));
                    instructions[jump_end] = BytecodeOp::Jump(instructions.len());
                } else {
                    instructions.push(BytecodeOp::GetProperty(access.member.clone()));
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
                            instructions.push(BytecodeOp::IsDefined(var_name.clone()));
                            return;
                        }
                    }
                    // Special-case: isNull(varName) -> TryLoadLocal + IsNull
                    // Uses TryLoadLocal so undefined vars return Null (true) rather than erroring
                    if ident.name.to_lowercase() == "isnull" && call.arguments.len() == 1 {
                        if let Expression::Identifier(ref arg_ident) = call.arguments[0] {
                            instructions.push(BytecodeOp::TryLoadLocal(arg_ident.name.clone()));
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
                            instructions.push(BytecodeOp::LoadGlobal(ident.name.clone()));
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
                        instructions.push(BytecodeOp::LoadGlobal(ident.name.clone()));
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
                        instructions.push(BytecodeOp::LoadGlobal(ident.name.clone()));
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
                    // Push function reference first
                    if let Expression::Identifier(ident) = &*call.name {
                        instructions.push(BytecodeOp::LoadGlobal(ident.name.clone()));
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
                        instructions.push(BytecodeOp::TryLoadLocal(ident.name.clone()));
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
                        instructions.push(BytecodeOp::CallMethodNamed(
                            call.method.clone(),
                            Box::new(names),
                            call.arguments.len(),
                            write_back.clone(),
                        ));
                    } else {
                        instructions.push(BytecodeOp::CallMethod(
                            call.method.clone(),
                            call.arguments.len(),
                            write_back.clone(),
                        ));
                    }
                    instructions[jump_end] = BytecodeOp::Jump(instructions.len());
                } else {
                    let names = compile_args(self, instructions);
                    if has_named {
                        instructions.push(BytecodeOp::CallMethodNamed(
                            call.method.clone(),
                            Box::new(names),
                            call.arguments.len(),
                            write_back,
                        ));
                    } else {
                        instructions.push(BytecodeOp::CallMethod(
                            call.method.clone(),
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

                // Function boundary: isolate finally/loop stacks for the closure
                // body (see compile_function_decl for why).
                let saved_finally = std::mem::take(&mut self.finally_stack);
                let saved_loops = std::mem::take(&mut self.loop_stack);

                let mut func_instructions = Vec::new();
                // Emit default parameter value preamble for closures
                for param in &closure.params {
                    if let Some(ref default_expr) = param.default {
                        func_instructions.push(BytecodeOp::LoadLocal(param.name.clone()));
                        func_instructions.push(BytecodeOp::IsNull);
                        let jump_idx = func_instructions.len();
                        func_instructions.push(BytecodeOp::JumpIfFalse(0));
                        self.compile_expression(default_expr, &mut func_instructions);
                        func_instructions.push(BytecodeOp::StoreLocal(param.name.clone()));
                        // Also update the arguments scope
                        func_instructions.push(BytecodeOp::LoadLocal("arguments".to_string()));
                        func_instructions.push(BytecodeOp::LoadLocal(param.name.clone()));
                        func_instructions.push(BytecodeOp::SetProperty(param.name.clone()));
                        func_instructions.push(BytecodeOp::StoreLocal("arguments".to_string()));
                        func_instructions[jump_idx] = BytecodeOp::JumpIfFalse(func_instructions.len());
                    }
                }
                for s in &closure.body {
                    self.compile_statement(s, &mut func_instructions);
                }
                func_instructions.push(BytecodeOp::Null);
                func_instructions.push(BytecodeOp::Return);
                self.finally_stack = saved_finally;
                self.loop_stack = saved_loops;

                let func_name = format!("__closure_{}", self.program.functions.len());
                let bc_func = BytecodeFunction {
                    name: func_name.clone(),
                    params: closure.params.iter().map(|p| p.name.clone()).collect(),
                    required_params: closure.params.iter().map(|p| p.required).collect(),
                    has_default: closure.params.iter().map(|p| p.default.is_some()).collect(),
                    instructions: func_instructions,
                    source_file: self.source_file.clone(),
                    global_id: next_global_fn_id(),
                    declared_local_mode: effective_declared,
                    param_types: closure.params.iter().map(|p| p.param_type.clone()).collect(),
                    return_type: None,
                    param_annotations: closure.params.iter().map(|p| p.annotations.clone()).collect(),
                    is_component_method: false,
                    access: cfml_common::dynamic::CfmlAccess::Public,
                    metadata: Vec::new(),
                };

                let global_id = bc_func.global_id as usize;
                self.program.functions.push(Arc::new(bc_func));
                instructions.push(BytecodeOp::DefineFunction(global_id));
                self.current_fn_local_mode = prev_fn_local_mode;
            }
            Expression::ArrowFunction(arrow) => {
                // Arrow functions inherit enclosing function's mode too
                // (they have no attribute syntax of their own).
                let arrow_effective = self.current_fn_local_mode;
                let prev_fn_local_mode = self.current_fn_local_mode;
                self.current_fn_local_mode = arrow_effective;
                // Function boundary: isolate finally/loop stacks for the body.
                let saved_finally = std::mem::take(&mut self.finally_stack);
                let saved_loops = std::mem::take(&mut self.loop_stack);
                let mut func_instructions = Vec::new();
                // Emit default parameter value preamble for arrow functions
                for param in &arrow.params {
                    if let Some(ref default_expr) = param.default {
                        func_instructions.push(BytecodeOp::LoadLocal(param.name.clone()));
                        func_instructions.push(BytecodeOp::IsNull);
                        let jump_idx = func_instructions.len();
                        func_instructions.push(BytecodeOp::JumpIfFalse(0));
                        self.compile_expression(default_expr, &mut func_instructions);
                        func_instructions.push(BytecodeOp::StoreLocal(param.name.clone()));
                        // Also update the arguments scope
                        func_instructions.push(BytecodeOp::LoadLocal("arguments".to_string()));
                        func_instructions.push(BytecodeOp::LoadLocal(param.name.clone()));
                        func_instructions.push(BytecodeOp::SetProperty(param.name.clone()));
                        func_instructions.push(BytecodeOp::StoreLocal("arguments".to_string()));
                        func_instructions[jump_idx] = BytecodeOp::JumpIfFalse(func_instructions.len());
                    }
                }
                self.compile_expression(&arrow.body, &mut func_instructions);
                func_instructions.push(BytecodeOp::Return);
                self.finally_stack = saved_finally;
                self.loop_stack = saved_loops;

                let func_name = format!("__arrow_{}", self.program.functions.len());
                let bc_func = BytecodeFunction {
                    name: func_name.clone(),
                    params: arrow.params.iter().map(|p| p.name.clone()).collect(),
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
                };

                let global_id = bc_func.global_id as usize;
                self.program.functions.push(Arc::new(bc_func));
                instructions.push(BytecodeOp::DefineFunction(global_id));
                self.current_fn_local_mode = prev_fn_local_mode;
            }
            Expression::This(_) => {
                instructions.push(BytecodeOp::LoadLocal("this".to_string()));
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
                // Use TryLoadLocal for simple identifiers (undefined vars → Null, not error)
                if let Expression::Identifier(ref ident) = *elvis.left {
                    instructions.push(BytecodeOp::TryLoadLocal(ident.name.clone()));
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
                let jump_idx = instructions.len();
                instructions.push(BytecodeOp::JumpIfNotNull(0)); // placeholder
                // Left is null, pop the null and eval right
                instructions.push(BytecodeOp::Pop);
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
            op <= 64,
            "BytecodeOp grew to {op} B (ceiling 64 B) — a perf regression. \
             If intentional, justify and raise the ceiling."
        );
    }
}
