//! v0.88.0 coverage instrumentation for the polymorphic-representation
//! roadmap (`JIT_POLY_DESIGN.md`).
//!
//! Today's JIT analyser only admits a body composed of a small op-subset
//! over `Int`/`Float`/`Bool` slots. The Option-γ tag-pointer work (v0.90.0
//! onwards) widens that subset by adding a `Kind::Boxed` representation for
//! values whose CFML type isn't statically known. **Before writing any of
//! that codegen,** we want a real signal about how much of a representative
//! workload's bytecode would actually benefit.
//!
//! This module is a side-channel scanner: it walks every function in a
//! compiled `BytecodeProgram` and bins each opcode into one of three
//! buckets:
//!
//! * `Supported` — the JIT handles this op today (Tier-1 / Tier-1.5 /
//!   Option-A builtin calls / OSR-eligible). A function composed entirely
//!   of `Supported` ops is already a JIT candidate.
//! * `BoxedPromising` — would become handleable under Option-γ. A
//!   function whose only "non-supported" ops are `BoxedPromising` is the
//!   *target* of the v0.90.0+ work.
//! * `Hopeless` — uses ops that no tagged-value representation can fix
//!   alone (closure creation, try/catch, dynamic include, etc.). A
//!   function containing any of these will stay on the interpreter
//!   regardless.
//!
//! The classification is **deliberately rough**. A `BoxedPromising` tag
//! means "the v0.90.0+ work *could in principle* admit this op"; it does
//! not guarantee a specific phase will. The coverage signal exists to
//! point at the largest opportunity, not predict exact gains.
//!
//! Aggregate output: per-program op counts + per-function admissibility
//! tally. Dumped via [`JitEngine::coverage_report`] when
//! `RUSTCFML_JIT_COVERAGE=1` (or `--jit-coverage` from the CLI).

use std::collections::BTreeMap;

use cfml_codegen::{BytecodeOp, BytecodeProgram};

/// Outcome bucket for a single opcode.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum OpClass {
    /// Already part of the JIT-supported op-subset today.
    Supported,
    /// Not supported today but a polymorphic tag-pointer representation
    /// (Option-γ in `JIT_POLY_DESIGN.md`) could plausibly handle it. The
    /// v0.90.0+ phases choose which of these actually land.
    BoxedPromising,
    /// Uses non-numeric semantics that a tagged value alone cannot rescue
    /// (closure creation, try/catch, dynamic-path include, etc.). Will
    /// stay on the interpreter even after Option-γ ships.
    Hopeless,
}

/// Classify a single op. Pattern-matches over `BytecodeOp` rather than
/// using its `Debug` because the latter is brittle to renames. New
/// variants default to `Hopeless` so we never silently overcount the
/// promising bucket.
pub fn classify(op: &BytecodeOp) -> OpClass {
    use BytecodeOp::*;
    match op {
        // ── SUPPORTED today ────────────────────────────────────────────
        Integer(_) | Double(_) | True | False | Null
        | LoadLocal(_) | StoreLocal(_) | DeclareLocal(_)
        | Add | Sub | Mul | Div | IntDiv | Mod | Pow | Negate
        | Eq | Neq | Lt | Lte | Gt | Gte
        | And | Or | Xor | Not
        | Jump(_) | JumpIfFalse(_) | JumpIfTrue(_)
        | JumpIfLocalCmpConstFalse(..) | ForLoopStep(..)
        | Increment(_) | Decrement(_)
        | AddLocalConst(..) | MulLocalConst(..)
        | Pop | Dup | LineInfo(..)
        | Return
        => OpClass::Supported,

        // Calls are partially supported: builtin shims + JIT-eligible UDFs
        // count as Supported; the call-target check happens elsewhere.
        // For a coarse op-level signal, we tag any call as Supported when
        // the analyser would admit it on its own; otherwise BoxedPromising.
        // To stay conservative, we tag *all* Call variants as
        // BoxedPromising here — the analyser will tell us at try-call time
        // whether it actually JITs.
        LoadGlobal(_) | StoreGlobal(_) | Call(_) | CallNamed(..)
        => OpClass::BoxedPromising,

        // Member access — primary target of v0.91.0 (member ICs).
        GetProperty(_) | TryGetProperty(_) | SetProperty(_)
        | LoadLocalProperty(..) | TryLoadLocalProperty(..) | StoreLocalProperty(..)
        => OpClass::BoxedPromising,

        // String / array / struct construction & indexing — eventually
        // reachable under boxed values (v0.92.0+ shim surface).
        String(_) | BuildArray(_) | BuildStruct(_)
        | IsDefined(_) | TryLoadLocal(_)
        => OpClass::BoxedPromising,

        // ── HOPELESS — won't be rescued by Option-γ alone ──────────────
        // Component construction, exception handling, dynamic include,
        // method dispatch with named args, closure machinery, etc. Each
        // has its own non-scalar runtime path.
        _ => OpClass::Hopeless,
    }
}

/// Aggregate counters for one program.
#[derive(Default, Debug, Clone)]
pub struct Report {
    pub total_functions: usize,
    pub total_ops: usize,
    pub supported_ops: usize,
    pub boxed_promising_ops: usize,
    pub hopeless_ops: usize,
    /// Functions whose every op is `Supported`. These are JIT candidates
    /// *modulo* the analyser's other checks (definite-assignment, no
    /// fall-through, etc.).
    pub all_supported_functions: usize,
    /// Functions whose every op is either `Supported` or `BoxedPromising`.
    /// These are the v0.90.0+ targets — the bucket the polymorphic work
    /// could unlock.
    pub boxed_admissible_functions: usize,
    /// Functions containing at least one `Hopeless` op. Stays interpreter.
    pub hopeless_functions: usize,
    /// Top-N opcodes by frequency (after the Supported set, since those
    /// already JIT). Keyed by an op-name string for human reading.
    pub top_bailing_ops: Vec<(String, usize)>,
}

/// Walk every function in `program` and return aggregate coverage.
pub fn scan_program(program: &BytecodeProgram) -> Report {
    let mut r = Report::default();
    let mut by_name: BTreeMap<String, usize> = BTreeMap::new();
    for f in &program.functions {
        r.total_functions += 1;
        let mut has_hopeless = false;
        let mut has_boxed = false;
        for op in &f.instructions {
            r.total_ops += 1;
            match classify(op) {
                OpClass::Supported => r.supported_ops += 1,
                OpClass::BoxedPromising => {
                    r.boxed_promising_ops += 1;
                    has_boxed = true;
                    *by_name.entry(op_name(op).to_string()).or_insert(0) += 1;
                }
                OpClass::Hopeless => {
                    r.hopeless_ops += 1;
                    has_hopeless = true;
                    *by_name.entry(op_name(op).to_string()).or_insert(0) += 1;
                }
            }
        }
        if has_hopeless {
            r.hopeless_functions += 1;
        } else if has_boxed {
            r.boxed_admissible_functions += 1;
        } else {
            r.all_supported_functions += 1;
        }
    }
    // Top-N (default 10) bailing ops by frequency for the report.
    let mut all: Vec<(String, usize)> = by_name.into_iter().collect();
    all.sort_by(|a, b| b.1.cmp(&a.1));
    r.top_bailing_ops = all.into_iter().take(10).collect();
    r
}

/// A static debug-name for an opcode — used in the coverage report.
/// Hand-rolled (not `Debug`) because it must stay stable across BytecodeOp
/// renames; only the variant tag is informative.
fn op_name(op: &BytecodeOp) -> &'static str {
    use BytecodeOp::*;
    match op {
        Integer(_) => "Integer",
        Double(_) => "Double",
        String(_) => "String",
        True => "True",
        False => "False",
        Null => "Null",
        LoadLocal(_) => "LoadLocal",
        StoreLocal(_) => "StoreLocal",
        DeclareLocal(_) => "DeclareLocal",
        ArrayAppendLocal(_) => "ArrayAppendLocal",
        LoadGlobal(_) => "LoadGlobal",
        StoreGlobal(_) => "StoreGlobal",
        Add => "Add", Sub => "Sub", Mul => "Mul",
        Div => "Div", IntDiv => "IntDiv", Mod => "Mod", Pow => "Pow",
        Negate => "Negate",
        Eq => "Eq", Neq => "Neq",
        Lt => "Lt", Lte => "Lte", Gt => "Gt", Gte => "Gte",
        And => "And", Or => "Or", Xor => "Xor", Not => "Not",
        Jump(_) => "Jump", JumpIfFalse(_) => "JumpIfFalse",
        JumpIfTrue(_) => "JumpIfTrue", JumpIfNotNull(_) => "JumpIfNotNull",
        JumpIfLocalCmpConstFalse(..) => "JumpIfLocalCmpConstFalse",
        ForLoopStep(..) => "ForLoopStep",
        Call(_) => "Call", CallNamed(..) => "CallNamed",
        CallMethod(..) => "CallMethod", CallMethodNamed(..) => "CallMethodNamed",
        BuildArray(_) => "BuildArray", BuildStruct(_) => "BuildStruct",
        GetProperty(_) => "GetProperty", SetProperty(_) => "SetProperty",
        LoadLocalProperty(..) => "LoadLocalProperty",
        LoadLocalKey(_) => "LoadLocalKey",
        StoreLocalProperty(..) => "StoreLocalProperty",
        NewObject(_) => "NewObject", NewObjectNamed(..) => "NewObjectNamed",
        DefineFunction(_) => "DefineFunction",
        Increment(_) => "Increment", Decrement(_) => "Decrement",
        AddLocalConst(..) => "AddLocalConst", MulLocalConst(..) => "MulLocalConst",
        TryStart(_) => "TryStart",
        Include(_) => "Include",
        IsDefined(_) => "IsDefined",
        TryLoadLocal(_) => "TryLoadLocal",
        CallRustSuperCtor(_) => "CallRustSuperCtor",
        LineInfo(..) => "LineInfo",
        Pop => "Pop", Dup => "Dup",
        Return => "Return",
        _ => "<other>",
    }
}

impl Report {
    /// Pretty-printed multi-line report for stderr / `--jit-coverage`.
    pub fn render(&self) -> String {
        let mut out = String::new();
        out.push_str("=== JIT coverage report (v0.88.0 / Option-\u{03b3} forecast) ===\n");
        out.push_str(&format!(
            "  functions:       {} total\n    \
             - all-supported (JIT today):       {}\n    \
             - boxed-admissible (Option-\u{03b3} target): {}\n    \
             - hopeless (stays interpreter):    {}\n",
            self.total_functions,
            self.all_supported_functions,
            self.boxed_admissible_functions,
            self.hopeless_functions,
        ));
        let admissible_pct = if self.total_functions > 0 {
            (self.boxed_admissible_functions + self.all_supported_functions) * 100
                / self.total_functions
        } else {
            0
        };
        out.push_str(&format!(
            "  admissible-after-\u{03b3}:    {}% of all functions\n",
            admissible_pct
        ));
        out.push_str(&format!(
            "  ops:             {} total (supported {}, boxed-promising {}, hopeless {})\n",
            self.total_ops,
            self.supported_ops,
            self.boxed_promising_ops,
            self.hopeless_ops,
        ));
        if !self.top_bailing_ops.is_empty() {
            out.push_str("  top non-supported ops (count):\n");
            for (name, n) in &self.top_bailing_ops {
                out.push_str(&format!("    {:>8}  {}\n", n, name));
            }
        }
        out
    }
}
