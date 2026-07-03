//! Bytecode → Cranelift IR translation and the owning JIT [`Backend`].
//!
//! The translator consumes an accepted [`Plan`] (see `analysis.rs`) and emits a
//! native function with the [`CompiledFn`](super::CompiledFn) ABI:
//! `fn(args: *const i64, nargs: i64, bail: *mut i64) -> i64`.
//!
//! Values are typed `Int` (native `i64`) or `Float` (native `f64`) per the
//! analysis' slot/return kinds; booleans live transiently as `i64` `0`/`1`.
//! Arguments arrive as `i64` (the engine bails unless every argument is a
//! `CfmlValue::Int`), so param slots are always `Int`. A `Float` return is
//! bit-cast to `i64` before the ABI return and re-interpreted as an `f64` by
//! the engine — see [`super::run_compiled`].
//!
//! Key choices (see `JIT_DESIGN.md`):
//! * **Locals are Cranelift `Variable`s** (`def_var`/`use_var`), typed `I64` or
//!   `F64` per `Plan::slot_kind`; `seal_all_blocks()` runs once at the end.
//! * **The operand stack is a compile-time `Vec<(Value, Kind)>`**, reset per
//!   block; the analysis guarantees it is empty at every block boundary.
//! * **Arithmetic matches the interpreter exactly**: `+ - *` on two ints use
//!   wrapping `iadd`/`isub`/`imul`; any float operand promotes (`fcvt_from_sint`)
//!   and uses `fadd`/`fsub`/`fmul`. `/` always produces an `f64` (`fdiv`) and
//!   **bails** (→ interpreter, which *throws*) on a zero divisor. `%`/`\` keep
//!   the existing integer bail on a zero/`INT_MIN`-overflow divisor.

use std::collections::HashMap;

use cfml_codegen::{BytecodeOp, BytecodeFunction, CmpOp};
use cranelift_codegen::ir::condcodes::{FloatCC, IntCC};
use cranelift_codegen::ir::{
    types, AbiParam, InstBuilder, MemFlags, Signature, StackSlotData, StackSlotKind, Value,
};
use cranelift_codegen::settings::{self, Configurable};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext, Variable};
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::{FuncId, Linkage, Module};

use super::analysis::{Kind, Plan};
use super::builtins::{self, SHIMS};
use super::shims::{
    cfml_jit_add_boxed, cfml_jit_box_float, cfml_jit_box_int, cfml_jit_concat_boxed,
    cfml_jit_mul_boxed, cfml_jit_sub_boxed,
};
use super::CompiledFn;

const I64: types::Type = types::I64;
const F64: types::Type = types::F64;

/// `extern "C"` shims registered with the JIT module so emitted IR can call
/// them. Defining them in Rust (rather than relying on a libc/libm symbol) keeps
/// behaviour bit-exact with the interpreter (Rust `%` on `f64` is IEEE-754
/// `frem`, and `f64::powf` is the same primitive `to_number(.).powf(..)` uses).
extern "C" fn cfml_fmod(a: f64, b: f64) -> f64 {
    a % b
}
extern "C" fn cfml_pow(a: f64, b: f64) -> f64 {
    a.powf(b)
}

/// v0.90.0 — materialise a string literal from a stable `(ptr, len)` view
/// of UTF-8 bytes owned by the [`Backend`]. The shim allocates a fresh
/// `CfmlValue::String(Arc<String>)` into the active per-call arena and
/// returns its tagged pointer.
///
/// Safety: `ptr`/`len` describe a live `Box<str>` stored in
/// `Backend::string_literals`, which lives as long as the JIT module.
#[no_mangle]
extern "C" fn cfml_jit_str_literal(ptr: *const u8, len: i64) -> i64 {
    use cfml_common::dynamic::CfmlValue;
    // SAFETY: see the doc comment above; backed by a stable `Box<str>` for
    // the lifetime of the JIT engine.
    let s = unsafe { std::slice::from_raw_parts(ptr, len as usize) };
    // The slice is guaranteed UTF-8 because we built it from a Rust
    // `Box<str>` / `String`.
    let s = unsafe { std::str::from_utf8_unchecked(s) };
    super::arena::box_into_active(CfmlValue::string(s.to_owned())) as i64
}

/// Dispatch a UDF→UDF call from inside a JIT'd body. The thread-local engine
/// pointer (set by `JitEngine::try_call`) is consulted to look up the
/// callee's compiled `(global_id, sig)` entry. Bail-code semantics:
/// * `*bail = 0` — success, return value is in `i64` (re-interpreted per
///   the caller's declared `expected_ret_kind`).
/// * `*bail = 1` — normal deopt: cache miss, callee not yet compiled, or the
///   callee's own body bailed (e.g. div-by-zero). Outer `try_call` falls back
///   to the interpreter but the caller's cache entry stays Compiled — the
///   bail is local to this dynamic call chain.
/// * `*bail = 2` — speculation mismatch: caller compiled assuming a specific
///   callee `ret_kind`, but the now-cached callee differs. The outer
///   `try_call` evicts the caller's cache entry so it re-analyses against
///   the now-known callee `ret_kind` on its next hot trip. See Phase-2
///   (`JIT_PHASE2_PLAN.md` v0.87.0) for the full handshake.
///
/// v0.90.1 widens `expected_ret_float: i64` (boolean 0/1) to
/// `expected_ret_kind: i64` (0=Int / 1=Float / 2=Boxed) so a JIT'd caller
/// can invoke a Boxed-returning JIT'd UDF.
#[no_mangle]
extern "C" fn cfml_call_jit_udf(
    global_id: i64,
    sig: i64,
    expected_ret_kind: i64,
    args: *const i64,
    nargs: i64,
    bail: *mut i64,
) -> i64 {
    // Widened ABI: global_id arrives as i64 to dodge cross-platform
    // small-int ABI ambiguity. Truncate to u32 here.
    crate::jit::dispatch_jit_udf(
        global_id as u32,
        sig as u64,
        expected_ret_kind,
        args,
        nargs,
        bail,
    )
}

/// Owns the Cranelift module (and thus all executable memory it allocates) plus
/// a reusable `FunctionBuilderContext`. Fields are `pub(super)` so the `osr`
/// sibling module can declare additional functions in the same `JITModule`
/// (and therefore share registered shim symbols and executable memory).
pub struct Backend {
    pub(super) module: JITModule,
    pub(super) fbc: FunctionBuilderContext,
    /// Monotonic counter for unique per-function symbol names.
    pub(super) func_counter: u32,
    /// Pre-declared `fn(f64, f64) -> f64` shims (`cfml_fmod`, `cfml_pow`).
    pub(super) fmod_id: FuncId,
    pub(super) pow_id: FuncId,
    /// Pre-declared `fn(u32, u64, *const i64, i64, *mut i64) -> i64` shim for
    /// dispatching JIT'd UDF→UDF calls (see `cfml_call_jit_udf`).
    pub(super) udf_dispatch_id: FuncId,
    /// Per-entry `FuncId` for every shim in [`SHIMS`] (parallel slice). Each
    /// `compile()` turns these into per-function `FuncRef`s via
    /// `declare_func_in_func`.
    pub(super) shim_ids: Vec<FuncId>,
    /// v0.90.0 — pre-declared FuncIds for the Boxed-value shims:
    /// `cfml_jit_box_int`, `cfml_jit_box_float`, `cfml_jit_concat_boxed`,
    /// `cfml_jit_add_boxed`, `cfml_jit_str_literal`. Imported into each
    /// compiled function's IR namespace via `declare_func_in_func`.
    pub(super) box_int_id: FuncId,
    pub(super) box_float_id: FuncId,
    pub(super) concat_boxed_id: FuncId,
    /// v0.141.0 — `cfml_jit_is_null(tagged) -> i64`, the null test for the
    /// PR #112 null-delete assignment guard (deopt-on-null).
    pub(super) is_null_id: FuncId,
    pub(super) add_boxed_id: FuncId,
    /// v0.99.7 — Sub/Mul Boxed slow shims (mirror `add_boxed_id`).
    pub(super) sub_boxed_id: FuncId,
    pub(super) mul_boxed_id: FuncId,
    pub(super) str_literal_id: FuncId,
    /// String-literal storage. Each unique `BytecodeOp::String(s)` interned
    /// during a compile is appended here (deduped). The `Box<str>` keeps
    /// each literal at a stable address for the life of the JIT module, so
    /// the IR can hold a raw pointer to it.
    pub(super) string_literals: Vec<Box<str>>,
    /// v0.99.5 — FuncId for `cfml_jit_member_get_boxed`, the IC shim for
    /// `obj.prop` member access on a `CfmlValue::Struct` receiver.
    pub(super) member_get_id: FuncId,
    /// v0.100.0 — FuncId for `cfml_jit_member_set_boxed`, the IC shim for
    /// `obj.prop = value` writes on a `CfmlValue::Struct` receiver.
    pub(super) member_set_id: FuncId,
    /// v0.99.5 — IC slot storage. Each entry is a heap-allocated
    /// `[cached_shape, cached_idx]` pair. `Box::as_ref().get()` returns a
    /// raw pointer that's stable for the box's lifetime; the Vec just
    /// holds the boxes (growing the Vec moves the 8-byte Box handles,
    /// not the heap allocations they point at), so the IC pointers we
    /// hand into JIT'd code remain valid until Backend drops. Each box
    /// is initialised to `[0, 0]` — shape `0` is reserved as the
    /// "never populated" sentinel (CfmlStruct shape IDs start at 1), so
    /// the first call's shape check always misses and the slow path
    /// populates the IC.
    pub(super) member_ic_slots: Vec<Box<std::cell::UnsafeCell<[u64; 3]>>>,
    /// v0.99.5 — interned property names referenced by emitted IC calls.
    /// Same stable-address rationale as `string_literals`.
    pub(super) member_names: Vec<Box<str>>,
}

impl Backend {
    /// Initialise the JIT backend for the host ISA. `Err` ⇒ caller declines to
    /// enable the JIT and stays on the interpreter.
    pub fn new() -> Result<Self, String> {
        let mut flag_builder = settings::builder();
        flag_builder
            .set("use_colocated_libcalls", "false")
            .map_err(|e| e.to_string())?;
        flag_builder.set("is_pic", "false").map_err(|e| e.to_string())?;
        flag_builder.set("opt_level", "speed").map_err(|e| e.to_string())?;
        let isa_builder = cranelift_native::builder().map_err(|e| e.to_string())?;
        let isa = isa_builder
            .finish(settings::Flags::new(flag_builder))
            .map_err(|e| e.to_string())?;
        let mut builder = JITBuilder::with_isa(isa, cranelift_module::default_libcall_names());
        // Register our Rust shims so emitted IR can call them by name. Both are
        // `extern "C" fn(f64, f64) -> f64`; addresses live in this crate for the
        // life of the process, so the cast is safe.
        builder.symbol("cfml_fmod", cfml_fmod as *const u8);
        builder.symbol("cfml_pow", cfml_pow as *const u8);
        builder.symbol("cfml_call_jit_udf", cfml_call_jit_udf as *const u8);
        // v0.90.0 boxed-value shims.
        builder.symbol("cfml_jit_box_int", cfml_jit_box_int as *const u8);
        builder.symbol("cfml_jit_box_float", cfml_jit_box_float as *const u8);
        builder.symbol("cfml_jit_concat_boxed", cfml_jit_concat_boxed as *const u8);
        builder.symbol("cfml_jit_is_null", super::shims::cfml_jit_is_null as *const u8);
        builder.symbol("cfml_jit_add_boxed", cfml_jit_add_boxed as *const u8);
        builder.symbol("cfml_jit_sub_boxed", cfml_jit_sub_boxed as *const u8);
        builder.symbol("cfml_jit_mul_boxed", cfml_jit_mul_boxed as *const u8);
        builder.symbol("cfml_jit_str_literal", cfml_jit_str_literal as *const u8);
        // v0.99.5 member-access IC shim.
        builder.symbol(
            "cfml_jit_member_get_boxed",
            super::shims::cfml_jit_member_get_boxed as *const u8,
        );
        // v0.100.0 member-write IC shim.
        builder.symbol(
            "cfml_jit_member_set_boxed",
            super::shims::cfml_jit_member_set_boxed as *const u8,
        );
        // Register every allowlisted builtin shim (Option A — Tier-1 native
        // calls). Each is a pure `extern "C"` Rust fn whose semantics mirror
        // the interpreter's `cfml-stdlib::builtins::fn_*` entry.
        for shim in SHIMS {
            builder.symbol(shim.sym, shim.addr);
        }
        let mut module = JITModule::new(builder);

        // Declare them once in the module; emitted functions reference these
        // FuncIds via `declare_func_in_func` and then `call`.
        let make_ff_f = |module: &mut JITModule, name: &str| -> Result<FuncId, String> {
            let mut sig = Signature::new(module.target_config().default_call_conv);
            sig.params.push(AbiParam::new(F64));
            sig.params.push(AbiParam::new(F64));
            sig.returns.push(AbiParam::new(F64));
            module
                .declare_function(name, Linkage::Import, &sig)
                .map_err(|e| e.to_string())
        };
        let fmod_id = make_ff_f(&mut module, "cfml_fmod")?;
        let pow_id = make_ff_f(&mut module, "cfml_pow")?;

        // `cfml_call_jit_udf`:
        //   fn(global_id: u32, sig: u64, expected_ret_kind: i64,
        //      args: *const i64, nargs: i64, bail: *mut i64) -> i64
        // u32 and u64 are both ABI'd as I64 on x86_64/aarch64 here — the
        // dispatcher truncates `global_id` itself. Keeping all integer args at
        // I64 sidesteps any cross-platform ABI ambiguity around small ints.
        // `expected_ret_kind` encodes the caller's speculated return kind:
        // 0=Int / 1=Float / 2=Boxed. The dispatcher checks it against the
        // cached callee and surfaces `*bail = 2` on mismatch (Phase 2 /
        // v0.90.1).
        let udf_dispatch_id = {
            let ptr_ty = module.target_config().pointer_type();
            let mut sig = Signature::new(module.target_config().default_call_conv);
            sig.params.push(AbiParam::new(I64)); // global_id (widened to i64)
            sig.params.push(AbiParam::new(I64)); // sig
            sig.params.push(AbiParam::new(I64)); // expected_ret_kind (0/1/2)
            sig.params.push(AbiParam::new(ptr_ty)); // args
            sig.params.push(AbiParam::new(I64)); // nargs
            sig.params.push(AbiParam::new(ptr_ty)); // bail
            sig.returns.push(AbiParam::new(I64));
            module
                .declare_function("cfml_call_jit_udf", Linkage::Import, &sig)
                .map_err(|e| e.to_string())?
        };

        // Declare the allowlisted builtin shims, each with a signature derived
        // from its `args_abi` / `ret_kind`. The order matches `SHIMS` so the
        // analysis' shim index can be reused as a slice index here.
        // v0.99.3 — bailable shims get a trailing `*mut i64` bail param.
        let ptr_ty = module.target_config().pointer_type();
        let shim_ids: Vec<FuncId> = SHIMS
            .iter()
            .map(|shim| {
                let mut sig = Signature::new(module.target_config().default_call_conv);
                for k in shim.args_abi {
                    sig.params
                        .push(AbiParam::new(if *k == Kind::Float { F64 } else { I64 }));
                }
                if shim.bailable {
                    sig.params.push(AbiParam::new(ptr_ty)); // bail
                }
                sig.returns
                    .push(AbiParam::new(if shim.ret_kind == Kind::Float { F64 } else { I64 }));
                module
                    .declare_function(shim.sym, Linkage::Import, &sig)
                    .map_err(|e| e.to_string())
            })
            .collect::<Result<Vec<_>, _>>()?;

        // v0.90.0 boxed-value shim declarations. All signatures use I64
        // for tagged-pointer args/returns (Option-γ encoding).
        let box_int_id = {
            let mut sig = Signature::new(module.target_config().default_call_conv);
            sig.params.push(AbiParam::new(I64));
            sig.returns.push(AbiParam::new(I64));
            module
                .declare_function("cfml_jit_box_int", Linkage::Import, &sig)
                .map_err(|e| e.to_string())?
        };
        let box_float_id = {
            let mut sig = Signature::new(module.target_config().default_call_conv);
            sig.params.push(AbiParam::new(F64));
            sig.returns.push(AbiParam::new(I64));
            module
                .declare_function("cfml_jit_box_float", Linkage::Import, &sig)
                .map_err(|e| e.to_string())?
        };
        let concat_boxed_id = {
            let mut sig = Signature::new(module.target_config().default_call_conv);
            sig.params.push(AbiParam::new(I64));
            sig.params.push(AbiParam::new(I64));
            sig.returns.push(AbiParam::new(I64));
            module
                .declare_function("cfml_jit_concat_boxed", Linkage::Import, &sig)
                .map_err(|e| e.to_string())?
        };
        let is_null_id = {
            let mut sig = Signature::new(module.target_config().default_call_conv);
            sig.params.push(AbiParam::new(I64)); // tagged value
            sig.returns.push(AbiParam::new(I64)); // 1 if Null, else 0
            module
                .declare_function("cfml_jit_is_null", Linkage::Import, &sig)
                .map_err(|e| e.to_string())?
        };
        let declare_arith_boxed =
            |module: &mut JITModule, sym: &str| -> Result<FuncId, String> {
                let mut sig = Signature::new(module.target_config().default_call_conv);
                sig.params.push(AbiParam::new(I64));
                sig.params.push(AbiParam::new(I64));
                sig.params.push(AbiParam::new(ptr_ty)); // bail
                sig.returns.push(AbiParam::new(I64));
                module
                    .declare_function(sym, Linkage::Import, &sig)
                    .map_err(|e| e.to_string())
            };
        let add_boxed_id = declare_arith_boxed(&mut module, "cfml_jit_add_boxed")?;
        let sub_boxed_id = declare_arith_boxed(&mut module, "cfml_jit_sub_boxed")?;
        let mul_boxed_id = declare_arith_boxed(&mut module, "cfml_jit_mul_boxed")?;
        let str_literal_id = {
            let mut sig = Signature::new(module.target_config().default_call_conv);
            sig.params.push(AbiParam::new(ptr_ty)); // *const u8
            sig.params.push(AbiParam::new(I64)); // len
            sig.returns.push(AbiParam::new(I64));
            module
                .declare_function("cfml_jit_str_literal", Linkage::Import, &sig)
                .map_err(|e| e.to_string())?
        };
        // v0.99.5 — member-access IC shim:
        //   fn(obj_tagged: i64, name_ptr: *const u8, name_len: i64,
        //      ic_slot: *mut u64, bail: *mut i64) -> i64
        let member_get_id = {
            let mut sig = Signature::new(module.target_config().default_call_conv);
            sig.params.push(AbiParam::new(I64)); // obj_tagged
            sig.params.push(AbiParam::new(ptr_ty)); // name_ptr
            sig.params.push(AbiParam::new(I64)); // name_len
            sig.params.push(AbiParam::new(ptr_ty)); // ic_slot
            sig.params.push(AbiParam::new(ptr_ty)); // bail
            sig.returns.push(AbiParam::new(I64));
            module
                .declare_function("cfml_jit_member_get_boxed", Linkage::Import, &sig)
                .map_err(|e| e.to_string())?
        };
        // v0.100.0 — member-write IC shim:
        //   fn(obj_tagged: i64, name_ptr: *const u8, name_len: i64,
        //      value_tagged: i64, ic_slot: *mut u64, bail: *mut i64) -> i64
        // Returns obj_tagged unchanged (passthrough; SetProperty re-pushes it).
        let member_set_id = {
            let mut sig = Signature::new(module.target_config().default_call_conv);
            sig.params.push(AbiParam::new(I64)); // obj_tagged
            sig.params.push(AbiParam::new(ptr_ty)); // name_ptr
            sig.params.push(AbiParam::new(I64)); // name_len
            sig.params.push(AbiParam::new(I64)); // value_tagged
            sig.params.push(AbiParam::new(ptr_ty)); // ic_slot
            sig.params.push(AbiParam::new(ptr_ty)); // bail
            sig.returns.push(AbiParam::new(I64));
            module
                .declare_function("cfml_jit_member_set_boxed", Linkage::Import, &sig)
                .map_err(|e| e.to_string())?
        };

        Ok(Self {
            module,
            fbc: FunctionBuilderContext::new(),
            func_counter: 0,
            fmod_id,
            pow_id,
            udf_dispatch_id,
            shim_ids,
            box_int_id,
            box_float_id,
            concat_boxed_id,
            is_null_id,
            add_boxed_id,
            sub_boxed_id,
            mul_boxed_id,
            str_literal_id,
            string_literals: Vec::new(),
            member_get_id,
            member_set_id,
            member_ic_slots: Vec::new(),
            member_names: Vec::new(),
        })
    }

    /// v0.99.6 — allocate one IC slot
    /// (`[cached_shape, cached_idx, cached_kind]`, initialised to `[0, 0, 0]`)
    /// and return its raw pointer for embedding into emitted IR. The Box
    /// itself is owned by `self.member_ic_slots` for the life of the
    /// Backend, so the pointer stays valid.
    ///
    /// `cached_kind` semantics: 0 = never populated, 1 = Int (SMI fast
    /// encode), 2 = Double, 3 = other (heap-box clone). See
    /// `shims::cfml_jit_member_get_boxed`.
    pub(super) fn alloc_member_ic_slot(&mut self) -> *mut u64 {
        let slot = Box::new(std::cell::UnsafeCell::new([0u64; 3]));
        let ptr = slot.get() as *mut u64;
        self.member_ic_slots.push(slot);
        ptr
    }

    /// v0.99.5 — intern a property name (same stable-address discipline
    /// as `intern_literal`).
    pub(super) fn intern_member_name(&mut self, name: &str) -> (*const u8, i64) {
        for boxed in &self.member_names {
            if boxed.as_ref() == name {
                return (boxed.as_ptr(), boxed.len() as i64);
            }
        }
        let boxed: Box<str> = name.into();
        let ptr = boxed.as_ptr();
        let len = boxed.len() as i64;
        self.member_names.push(boxed);
        (ptr, len)
    }

    /// Intern a string literal at a stable heap location and return the
    /// `(ptr, len)` view the JIT can embed as a constant in IR.
    pub(super) fn intern_literal(&mut self, s: &str) -> (*const u8, i64) {
        // Linear scan — dedupe is nice-to-have but literals are bounded by
        // the compile rate, and dedup keys typically scale O(small).
        for boxed in &self.string_literals {
            if boxed.as_ref() == s {
                return (boxed.as_ptr(), boxed.len() as i64);
            }
        }
        let boxed: Box<str> = s.into();
        let ptr = boxed.as_ptr();
        let len = boxed.len() as i64;
        self.string_literals.push(boxed);
        (ptr, len)
    }

    /// Compile `func` per `plan` to native code, returning a callable pointer.
    pub fn compile(&mut self, func: &BytecodeFunction, plan: &Plan) -> Result<CompiledFn, String> {
        // Inner block: any `?` or `return Err` aborts the body without calling
        // `FunctionBuilder::finalize`, leaving `self.fbc` in a partially-built
        // state. The next `FunctionBuilder::new` then panics on an
        // `is_empty()` assertion. Catch the result here and on any error
        // reinitialise `self.fbc` so a future compile starts clean. This
        // never observes a successful compile (finalize was called inside
        // the block); only the failure path pays the cost.
        let result = self.compile_inner(func, plan);
        if let Err(ref e) = result {
            self.fbc = FunctionBuilderContext::new();
            if std::env::var("RUSTCFML_JIT_DEBUG").is_ok() {
                eprintln!("[jit] compile failed for {}: {}", func.name, e);
            }
        }
        result
    }

    fn compile_inner(
        &mut self,
        func: &BytecodeFunction,
        plan: &Plan,
    ) -> Result<CompiledFn, String> {
        let ptr_ty = self.module.target_config().pointer_type();
        let mut ctx = self.module.make_context();
        // Signature: (args: *const i64, nargs: i64, bail: *mut i64) -> i64
        ctx.func.signature.params.push(AbiParam::new(ptr_ty)); // args
        ctx.func.signature.params.push(AbiParam::new(I64)); // nargs (unused; checked in Rust)
        ctx.func.signature.params.push(AbiParam::new(ptr_ty)); // bail
        ctx.func.signature.returns.push(AbiParam::new(I64));

        // Import the shim FuncIds into this function's IR namespace so we can
        // emit `call`s to them. Done before the builder borrows `ctx.func`.
        let fmod_ref = self.module.declare_func_in_func(self.fmod_id, &mut ctx.func);
        let pow_ref = self.module.declare_func_in_func(self.pow_id, &mut ctx.func);
        let udf_dispatch_ref = self
            .module
            .declare_func_in_func(self.udf_dispatch_id, &mut ctx.func);
        // v0.90.0 boxed shims.
        let box_int_ref = self
            .module
            .declare_func_in_func(self.box_int_id, &mut ctx.func);
        let box_float_ref = self
            .module
            .declare_func_in_func(self.box_float_id, &mut ctx.func);
        let concat_boxed_ref = self
            .module
            .declare_func_in_func(self.concat_boxed_id, &mut ctx.func);
        let is_null_ref = self
            .module
            .declare_func_in_func(self.is_null_id, &mut ctx.func);
        let add_boxed_ref = self
            .module
            .declare_func_in_func(self.add_boxed_id, &mut ctx.func);
        let sub_boxed_ref = self
            .module
            .declare_func_in_func(self.sub_boxed_id, &mut ctx.func);
        let mul_boxed_ref = self
            .module
            .declare_func_in_func(self.mul_boxed_id, &mut ctx.func);
        let str_literal_ref = self
            .module
            .declare_func_in_func(self.str_literal_id, &mut ctx.func);
        // v0.99.5 member-get shim.
        let member_get_ref = self
            .module
            .declare_func_in_func(self.member_get_id, &mut ctx.func);
        // v0.100.0 member-set shim.
        let member_set_ref = self
            .module
            .declare_func_in_func(self.member_set_id, &mut ctx.func);

        // v0.90.0 — intern every String literal in the reachable code
        // before the FunctionBuilder borrows take effect. The resulting
        // (ptr, len) pairs are keyed by IP and pulled into the codegen
        // loop below as constants.
        let mut str_literal_at: HashMap<usize, (*const u8, i64)> = HashMap::new();
        // v0.99.5 — pre-scan member-access call sites (LoadLocalProperty,
        // GetProperty), allocating one IC slot per site and interning each
        // property name. The triple `(ic_slot_ptr, name_ptr, name_len)` is
        // keyed by IP and embedded into the IR as constants. Same
        // before-FunctionBuilder rationale as gotcha #15.
        let mut member_get_at: HashMap<usize, (*mut u64, *const u8, i64)> = HashMap::new();
        // v0.100.0 — IC slots for member-write sites (SetProperty,
        // StoreLocalProperty). Same slot layout as the read IC.
        let mut member_set_at: HashMap<usize, (*mut u64, *const u8, i64)> = HashMap::new();
        // v0.141.0 — null-delete assignment guards (PR #112). The `Pop`/
        // `UnsetPath` pair is skipped; the `JumpIfNotNull` becomes a
        // deopt-if-null check on the guarded RHS (see the op loop below).
        let null_guards = super::analysis::null_guard_sites(&func.instructions);
        for blk in &plan.blocks {
            for ip in blk.start..blk.end {
                match &func.instructions[ip] {
                    BytecodeOp::String(s) => {
                        str_literal_at.insert(ip, self.intern_literal(s));
                    }
                    BytecodeOp::LoadLocalProperty(_, prop)
                    | BytecodeOp::TryLoadLocalProperty(_, prop)
                    | BytecodeOp::GetProperty(prop)
                    | BytecodeOp::TryGetProperty(prop) => {
                        let ic_ptr = self.alloc_member_ic_slot();
                        let (name_ptr, name_len) = self.intern_member_name(prop);
                        member_get_at.insert(ip, (ic_ptr, name_ptr, name_len));
                    }
                    BytecodeOp::StoreLocalProperty(_, prop)
                    | BytecodeOp::SetProperty(prop) => {
                        let ic_ptr = self.alloc_member_ic_slot();
                        let (name_ptr, name_len) = self.intern_member_name(prop);
                        member_set_at.insert(ip, (ic_ptr, name_ptr, name_len));
                    }
                    _ => {}
                }
            }
        }
        // Parallel to SHIMS — only the entries the function actually calls
        // get used, but importing all of them is cheap and keeps indexing
        // straight-through.
        let shim_refs: Vec<_> = self
            .shim_ids
            .iter()
            .map(|id| self.module.declare_func_in_func(*id, &mut ctx.func))
            .collect();

        {
            let mut b = FunctionBuilder::new(&mut ctx.func, &mut self.fbc);

            let cl_blocks: Vec<_> = plan.blocks.iter().map(|_| b.create_block()).collect();
            let bail_block = b.create_block();
            let entry = b.create_block();

            // One Variable per local slot, typed I64 (Int) or F64 (Float).
            let vars: Vec<Variable> = plan
                .slot_kind
                .iter()
                .map(|k| b.declare_var(if *k == Kind::Float { F64 } else { I64 }))
                .collect();
            let bail_var = b.declare_var(ptr_ty);

            // Stack slot for marshalling UDF→UDF call arguments. Sized for
            // the widest call this function makes; only allocated when the
            // body actually calls another UDF.
            let udf_args_slot = if !plan.udf_call_at.is_empty() {
                let max_call_args = plan
                    .udf_call_at
                    .keys()
                    .map(|&ip| match &func.instructions[ip] {
                        BytecodeOp::Call(n) => *n,
                        _ => 0,
                    })
                    .max()
                    .unwrap_or(0);
                if max_call_args == 0 {
                    None
                } else {
                    let bytes = (max_call_args * 8) as u32;
                    Some(b.create_sized_stack_slot(StackSlotData::new(
                        StackSlotKind::ExplicitSlot,
                        bytes,
                        3, // log2(8) — i64-aligned
                    )))
                }
            } else {
                None
            };

            // ── Prologue ────────────────────────────────────────────────────
            b.append_block_params_for_function_params(entry);
            b.switch_to_block(entry);
            let args_ptr = b.block_params(entry)[0];
            let bail_ptr = b.block_params(entry)[2];
            b.def_var(bail_var, bail_ptr);
            // Zero-init every local (belt-and-suspenders; analysis proves no
            // observed read of the zero).
            let zero_i = b.ins().iconst(I64, 0);
            let zero_f = b.ins().f64const(0.0);
            for (slot, &var) in vars.iter().enumerate() {
                if plan.slot_kind[slot] == Kind::Float {
                    b.def_var(var, zero_f);
                } else {
                    b.def_var(var, zero_i);
                }
            }
            // Bind args positionally into their param slots. The args buffer
            // holds 8 raw bytes per slot — an `i64` for Int params, the bit-cast
            // of an `f64` for Float params (see `run_compiled`).
            for (i, &slot) in plan.param_slots.iter().enumerate() {
                let off = (i * 8) as i32;
                let v = if plan.slot_kind[slot] == Kind::Float {
                    b.ins().load(F64, MemFlags::new(), args_ptr, off)
                } else {
                    b.ins().load(I64, MemFlags::new(), args_ptr, off)
                };
                b.def_var(vars[slot], v);
            }
            b.ins().jump(cl_blocks[0], &[]);

            // ── Bail block: *bail = 1; return 0 ─────────────────────────────
            b.switch_to_block(bail_block);
            let bp = b.use_var(bail_var);
            let one = b.ins().iconst(I64, 1);
            b.ins().store(MemFlags::new(), one, bp, 0);
            let zero_ret = b.ins().iconst(I64, 0);
            b.ins().return_(&[zero_ret]);

            // ── Translate each reachable block ──────────────────────────────
            for (bidx, blk) in plan.blocks.iter().enumerate() {
                b.switch_to_block(cl_blocks[bidx]);
                let mut stack: Vec<(Value, Kind)> = Vec::new();
                let mut terminated = false;

                let target_block = |ip: usize| -> Result<cranelift_codegen::ir::Block, String> {
                    plan.block_at
                        .get(&ip)
                        .map(|&i| cl_blocks[i])
                        .ok_or_else(|| format!("jit: target ip {ip} not a block leader"))
                };
                let fallthrough = target_block(blk.end);

                for ip in blk.start..blk.end {
                    // Null-delete guard (PR #112): skip the `Pop`/`UnsetPath`
                    // pair — they are the deopt-on-null path, never run in
                    // native code (the guarded value flows straight to the
                    // store at ip+3).
                    if null_guards.skip.contains(&ip) {
                        continue;
                    }
                    // The guard's `JumpIfNotNull` becomes a deopt-if-null
                    // check: peek the RHS (it stays on the stack for the
                    // store). An Int/Float can never be Null, so no check is
                    // needed; a Boxed value is tested by `cfml_jit_is_null` and
                    // bails to the interpreter (which deletes the target /
                    // throws on a later undefined read) when Null.
                    if null_guards.jump.contains(&ip) {
                        let &(v, k) = stack.last().ok_or("jit: guard on empty stack")?;
                        if k == Kind::Boxed {
                            let call = b.ins().call(is_null_ref, &[v]);
                            let is_null = b.inst_results(call)[0];
                            let null_set = b.ins().icmp_imm(IntCC::NotEqual, is_null, 0);
                            let cont = b.create_block();
                            b.ins().brif(null_set, bail_block, &[], cont, &[]);
                            b.switch_to_block(cont);
                        }
                        continue;
                    }
                    match &func.instructions[ip] {
                        BytecodeOp::Integer(n) => stack.push((b.ins().iconst(I64, *n), Kind::Int)),
                        BytecodeOp::Double(d) => stack.push((b.ins().f64const(*d), Kind::Float)),
                        BytecodeOp::True => stack.push((b.ins().iconst(I64, 1), Kind::Bool)),
                        BytecodeOp::False => stack.push((b.ins().iconst(I64, 0), Kind::Bool)),

                        // v0.90.0 — emit a call to `cfml_jit_str_literal`
                        // with the interned (ptr, len) view. Returns a
                        // tagged Boxed pointer.
                        BytecodeOp::String(_) => {
                            let (ptr, len) = *str_literal_at
                                .get(&ip)
                                .ok_or("jit: missing interned string literal")?;
                            let ptr_v = b.ins().iconst(ptr_ty, ptr as i64);
                            let len_v = b.ins().iconst(I64, len);
                            let call = b.ins().call(str_literal_ref, &[ptr_v, len_v]);
                            let r = b.inst_results(call)[0];
                            stack.push((r, Kind::Boxed));
                        }

                        // v0.90.0 — `&` concat. Pop b, pop a; box each
                        // non-Boxed operand via the matching shim; call
                        // `cfml_jit_concat_boxed(a, b)`; push Boxed.
                        BytecodeOp::Concat => {
                            let (rhs, rk) = stack.pop().ok_or("jit: stack underflow")?;
                            let (lhs, lk) = stack.pop().ok_or("jit: stack underflow")?;
                            let a_tag = ensure_boxed(&mut b, box_int_ref, box_float_ref, lhs, lk);
                            let b_tag = ensure_boxed(&mut b, box_int_ref, box_float_ref, rhs, rk);
                            let call = b.ins().call(concat_boxed_ref, &[a_tag, b_tag]);
                            let r = b.inst_results(call)[0];
                            stack.push((r, Kind::Boxed));
                        }

                        BytecodeOp::LoadLocal(name) => {
                            let slot = plan.slot_of(name).ok_or("jit: unknown local")?;
                            stack.push((b.use_var(vars[slot]), plan.slot_kind[slot]));
                        }
                        // v0.99.5 — fused `local.prop`. Read the local
                        // (must be Boxed kind per the analyser), then call
                        // the IC shim with the pre-allocated slot pointer
                        // and interned property name. Bail wires same as
                        // any bailable shim.
                        BytecodeOp::LoadLocalProperty(name, _prop)
                        | BytecodeOp::TryLoadLocalProperty(name, _prop) => {
                            let slot = plan.slot_of(name).ok_or("jit: unknown local")?;
                            let obj = b.use_var(vars[slot]);
                            let (ic_ptr, name_ptr, name_len) = *member_get_at
                                .get(&ip)
                                .ok_or("jit: missing member IC slot")?;
                            let ic_v = b.ins().iconst(ptr_ty, ic_ptr as i64);
                            let np_v = b.ins().iconst(ptr_ty, name_ptr as i64);
                            let nl_v = b.ins().iconst(I64, name_len);
                            let bp = b.use_var(bail_var);
                            let call = b
                                .ins()
                                .call(member_get_ref, &[obj, np_v, nl_v, ic_v, bp]);
                            let r = b.inst_results(call)[0];
                            // Post-call bail check (mirrors the bailable
                            // builtin shim pattern from v0.99.3).
                            let bail_val = b.ins().load(I64, MemFlags::new(), bp, 0);
                            let bail_set =
                                b.ins().icmp_imm(IntCC::NotEqual, bail_val, 0);
                            let cont = b.create_block();
                            b.ins().brif(bail_set, bail_block, &[], cont, &[]);
                            b.switch_to_block(cont);
                            stack.push((r, Kind::Boxed));
                        }
                        // v0.99.5 — `obj.prop` where obj is on the stack
                        // (Boxed). Same IC shape as LoadLocalProperty.
                        // TryGetProperty shares the codegen — the miss/throw
                        // distinction is resolved by the shim's bail path.
                        BytecodeOp::GetProperty(_prop) | BytecodeOp::TryGetProperty(_prop) => {
                            let (obj, ok) = stack.pop().ok_or("jit: stack underflow")?;
                            if ok != Kind::Boxed {
                                return Err("jit: GetProperty receiver not Boxed".into());
                            }
                            let (ic_ptr, name_ptr, name_len) = *member_get_at
                                .get(&ip)
                                .ok_or("jit: missing member IC slot")?;
                            let ic_v = b.ins().iconst(ptr_ty, ic_ptr as i64);
                            let np_v = b.ins().iconst(ptr_ty, name_ptr as i64);
                            let nl_v = b.ins().iconst(I64, name_len);
                            let bp = b.use_var(bail_var);
                            let call = b
                                .ins()
                                .call(member_get_ref, &[obj, np_v, nl_v, ic_v, bp]);
                            let r = b.inst_results(call)[0];
                            let bail_val = b.ins().load(I64, MemFlags::new(), bp, 0);
                            let bail_set =
                                b.ins().icmp_imm(IntCC::NotEqual, bail_val, 0);
                            let cont = b.create_block();
                            b.ins().brif(bail_set, bail_block, &[], cont, &[]);
                            b.switch_to_block(cont);
                            stack.push((r, Kind::Boxed));
                        }
                        BytecodeOp::StoreLocal(name) => {
                            let slot = plan.slot_of(name).ok_or("jit: unknown local")?;
                            let (v, _) = stack.pop().ok_or("jit: stack underflow")?;
                            b.def_var(vars[slot], v);
                        }

                        // v0.100.0 — `obj.prop = value`. Pop value
                        // (Int/Float/Boxed; non-Boxed gets boxed via the
                        // same shims Concat uses), pop obj (Boxed), call
                        // the write-IC shim. Shim returns obj_tagged
                        // passthrough; push it back so the compiler-emitted
                        // writeback chain sees the same value the
                        // interpreter would.
                        BytecodeOp::SetProperty(_) => {
                            let (val_raw, vk) = stack.pop().ok_or("jit: stack underflow")?;
                            let (obj, ok) = stack.pop().ok_or("jit: stack underflow")?;
                            if ok != Kind::Boxed {
                                return Err("jit: SetProperty receiver not Boxed".into());
                            }
                            let val = ensure_boxed(&mut b, box_int_ref, box_float_ref, val_raw, vk);
                            let (ic_ptr, name_ptr, name_len) = *member_set_at
                                .get(&ip)
                                .ok_or("jit: missing member-set IC slot")?;
                            let ic_v = b.ins().iconst(ptr_ty, ic_ptr as i64);
                            let np_v = b.ins().iconst(ptr_ty, name_ptr as i64);
                            let nl_v = b.ins().iconst(I64, name_len);
                            let bp = b.use_var(bail_var);
                            let call = b
                                .ins()
                                .call(member_set_ref, &[obj, np_v, nl_v, val, ic_v, bp]);
                            let r = b.inst_results(call)[0];
                            let bail_val = b.ins().load(I64, MemFlags::new(), bp, 0);
                            let bail_set =
                                b.ins().icmp_imm(IntCC::NotEqual, bail_val, 0);
                            let cont = b.create_block();
                            b.ins().brif(bail_set, bail_block, &[], cont, &[]);
                            b.switch_to_block(cont);
                            stack.push((r, Kind::Boxed));
                        }

                        // v0.100.0 — fused `local.prop = value`. Pop value
                        // (Int/Float/Boxed; box non-Boxed), read local
                        // (Boxed); call the write-IC shim. No push.
                        BytecodeOp::StoreLocalProperty(name, _prop) => {
                            let slot = plan.slot_of(name).ok_or("jit: unknown local")?;
                            let (val_raw, vk) = stack.pop().ok_or("jit: stack underflow")?;
                            let val = ensure_boxed(&mut b, box_int_ref, box_float_ref, val_raw, vk);
                            let obj = b.use_var(vars[slot]);
                            let (ic_ptr, name_ptr, name_len) = *member_set_at
                                .get(&ip)
                                .ok_or("jit: missing member-set IC slot")?;
                            let ic_v = b.ins().iconst(ptr_ty, ic_ptr as i64);
                            let np_v = b.ins().iconst(ptr_ty, name_ptr as i64);
                            let nl_v = b.ins().iconst(I64, name_len);
                            let bp = b.use_var(bail_var);
                            let call = b
                                .ins()
                                .call(member_set_ref, &[obj, np_v, nl_v, val, ic_v, bp]);
                            let _r = b.inst_results(call)[0];
                            let bail_val = b.ins().load(I64, MemFlags::new(), bp, 0);
                            let bail_set =
                                b.ins().icmp_imm(IntCC::NotEqual, bail_val, 0);
                            let cont = b.create_block();
                            b.ins().brif(bail_set, bail_block, &[], cont, &[]);
                            b.switch_to_block(cont);
                        }

                        // v0.99.6/v0.99.7 — Add/Sub/Mul admit Boxed operands;
                        // if either side is Boxed, route through the SMI
                        // tag-check fast path with the matching slow shim.
                        op_variant @ (BytecodeOp::Add | BytecodeOp::Sub | BytecodeOp::Mul) => {
                            let op = match op_variant {
                                BytecodeOp::Add => NumOp::Add,
                                BytecodeOp::Sub => NumOp::Sub,
                                _ => NumOp::Mul,
                            };
                            let either_boxed = matches!(stack.last(), Some((_, Kind::Boxed)))
                                || matches!(stack.iter().rev().nth(1), Some((_, Kind::Boxed)));
                            if either_boxed {
                                let slow = match op {
                                    NumOp::Add => add_boxed_ref,
                                    NumOp::Sub => sub_boxed_ref,
                                    NumOp::Mul => mul_boxed_ref,
                                };
                                arith_boxed_smi(
                                    &mut b,
                                    &mut stack,
                                    op,
                                    box_int_ref,
                                    box_float_ref,
                                    slow,
                                    bail_var,
                                    ptr_ty,
                                )?;
                            } else {
                                num_bin(&mut b, &mut stack, op)?;
                            }
                        }

                        // `/` — always f64; bail (→ interpreter throws) on zero.
                        BytecodeOp::Div => {
                            let (rhs, rk) = stack.pop().ok_or("jit: stack underflow")?;
                            let (lhs, lk) = stack.pop().ok_or("jit: stack underflow")?;
                            let a = to_f64(&mut b, lhs, lk);
                            let d = to_f64(&mut b, rhs, rk);
                            let fz = b.ins().f64const(0.0);
                            let is_zero = b.ins().fcmp(FloatCC::Equal, d, fz);
                            let cont = b.create_block();
                            b.ins().brif(is_zero, bail_block, &[], cont, &[]);
                            b.switch_to_block(cont);
                            stack.push((b.ins().fdiv(a, d), Kind::Float));
                        }

                        // `%` — Int,Int → wrapping `srem` (guarded against 0 /
                        // INT_MIN÷-1). Any float operand promotes both to f64
                        // and calls the `cfml_fmod` shim (matches the
                        // interpreter's `f64 %` on mixed/float operands).
                        BytecodeOp::Mod => {
                            let (rhs, rk) = stack.pop().ok_or("jit: stack underflow")?;
                            let (lhs, lk) = stack.pop().ok_or("jit: stack underflow")?;
                            if lk == Kind::Float || rk == Kind::Float {
                                let a = to_f64(&mut b, lhs, lk);
                                let d = to_f64(&mut b, rhs, rk);
                                let call = b.ins().call(fmod_ref, &[a, d]);
                                let r = b.inst_results(call)[0];
                                stack.push((r, Kind::Float));
                            } else {
                                let cont = guard_int_div(&mut b, bail_block, lhs, rhs);
                                b.switch_to_block(cont);
                                stack.push((b.ins().srem(lhs, rhs), Kind::Int));
                            }
                        }

                        // `^` — always Double. Promote both operands to f64 and
                        // call the `cfml_pow` shim (matches `f64::powf` used by
                        // the interpreter).
                        BytecodeOp::Pow => {
                            let (rhs, rk) = stack.pop().ok_or("jit: stack underflow")?;
                            let (lhs, lk) = stack.pop().ok_or("jit: stack underflow")?;
                            if !matches!(lk, Kind::Int | Kind::Float)
                                || !matches!(rk, Kind::Int | Kind::Float)
                            {
                                return Err("jit: pow operand not numeric".into());
                            }
                            let a = to_f64(&mut b, lhs, lk);
                            let d = to_f64(&mut b, rhs, rk);
                            let call = b.ins().call(pow_ref, &[a, d]);
                            let r = b.inst_results(call)[0];
                            stack.push((r, Kind::Float));
                        }

                        // `\` — always Int; float operands truncate toward zero.
                        BytecodeOp::IntDiv => {
                            let (rhs, rk) = stack.pop().ok_or("jit: stack underflow")?;
                            let (lhs, lk) = stack.pop().ok_or("jit: stack underflow")?;
                            let a = to_i64(&mut b, lhs, lk);
                            let d = to_i64(&mut b, rhs, rk);
                            let cont = guard_int_div(&mut b, bail_block, a, d);
                            b.switch_to_block(cont);
                            stack.push((b.ins().sdiv(a, d), Kind::Int));
                        }

                        BytecodeOp::Negate => {
                            let (a, k) = stack.pop().ok_or("jit: stack underflow")?;
                            let r = if k == Kind::Float {
                                b.ins().fneg(a)
                            } else {
                                b.ins().ineg(a)
                            };
                            stack.push((r, k));
                        }

                        BytecodeOp::Eq => cmp(&mut b, &mut stack, IntCC::Equal, FloatCC::Equal)?,
                        BytecodeOp::Neq => {
                            cmp(&mut b, &mut stack, IntCC::NotEqual, FloatCC::NotEqual)?
                        }
                        BytecodeOp::Lt => cmp(
                            &mut b,
                            &mut stack,
                            IntCC::SignedLessThan,
                            FloatCC::LessThan,
                        )?,
                        BytecodeOp::Lte => cmp(
                            &mut b,
                            &mut stack,
                            IntCC::SignedLessThanOrEqual,
                            FloatCC::LessThanOrEqual,
                        )?,
                        BytecodeOp::Gt => cmp(
                            &mut b,
                            &mut stack,
                            IntCC::SignedGreaterThan,
                            FloatCC::GreaterThan,
                        )?,
                        BytecodeOp::Gte => cmp(
                            &mut b,
                            &mut stack,
                            IntCC::SignedGreaterThanOrEqual,
                            FloatCC::GreaterThanOrEqual,
                        )?,

                        BytecodeOp::And => logic2(&mut b, &mut stack, LogicOp::And)?,
                        BytecodeOp::Or => logic2(&mut b, &mut stack, LogicOp::Or)?,
                        BytecodeOp::Xor => logic2(&mut b, &mut stack, LogicOp::Xor)?,
                        BytecodeOp::Not => {
                            let (a, k) = stack.pop().ok_or("jit: stack underflow")?;
                            let t = is_zero_test(&mut b, a, k);
                            stack.push((bool_to_i64(&mut b, t), Kind::Bool));
                        }

                        BytecodeOp::Increment(name) => rmw_imm(&mut b, &vars, plan, name, 1)?,
                        BytecodeOp::Decrement(name) => rmw_imm(&mut b, &vars, plan, name, -1)?,
                        BytecodeOp::AddLocalConst(name, k) => rmw_imm(&mut b, &vars, plan, name, *k)?,
                        BytecodeOp::MulLocalConst(name, k) => {
                            let slot = plan.slot_of(name).ok_or("jit: unknown local")?;
                            let v = b.use_var(vars[slot]);
                            let nv = b.ins().imul_imm(v, *k);
                            b.def_var(vars[slot], nv);
                        }

                        BytecodeOp::JumpIfLocalCmpConstFalse(name, c, cmpop, target) => {
                            let slot = plan.slot_of(name).ok_or("jit: unknown local")?;
                            let v = b.use_var(vars[slot]);
                            let cc = b.ins().icmp_imm(int_cc(*cmpop), v, *c);
                            b.ins().brif(cc, fallthrough.clone()?, &[], target_block(*target)?, &[]);
                            terminated = true;
                        }
                        BytecodeOp::ForLoopStep(name, limit, cmpop, step, target) => {
                            let slot = plan.slot_of(name).ok_or("jit: unknown local")?;
                            let v = b.use_var(vars[slot]);
                            let nv = b.ins().iadd_imm(v, *step);
                            b.def_var(vars[slot], nv);
                            let cc = b.ins().icmp_imm(int_cc(*cmpop), nv, *limit);
                            b.ins().brif(cc, target_block(*target)?, &[], fallthrough.clone()?, &[]);
                            terminated = true;
                        }

                        BytecodeOp::Jump(target) => {
                            b.ins().jump(target_block(*target)?, &[]);
                            terminated = true;
                        }
                        BytecodeOp::JumpIfFalse(target) => {
                            let (cond, k) = stack.pop().ok_or("jit: stack underflow")?;
                            let is_false = is_zero_test(&mut b, cond, k);
                            b.ins().brif(is_false, target_block(*target)?, &[], fallthrough.clone()?, &[]);
                            terminated = true;
                        }
                        BytecodeOp::JumpIfTrue(target) => {
                            let (cond, k) = stack.pop().ok_or("jit: stack underflow")?;
                            let is_true = is_truthy(&mut b, cond, k);
                            b.ins().brif(is_true, target_block(*target)?, &[], fallthrough.clone()?, &[]);
                            terminated = true;
                        }

                        BytecodeOp::Pop => {
                            let _ = stack.pop();
                        }
                        BytecodeOp::Dup => {
                            let v = *stack.last().ok_or("jit: stack underflow")?;
                            stack.push(v);
                        }

                        BytecodeOp::Return => {
                            let (v, k) = stack.pop().ok_or("jit: stack underflow")?;
                            // A Float result is bit-cast to i64 for the ABI and
                            // re-interpreted as f64 by the engine.
                            let out = if k == Kind::Float {
                                b.ins().bitcast(I64, MemFlags::new(), v)
                            } else {
                                v
                            };
                            b.ins().return_(&[out]);
                            terminated = true;
                        }

                        BytecodeOp::DeclareLocal(_) => {}
                        BytecodeOp::LineInfo(_, _) => {}

                        // Push a marker for a known builtin or UDF ref. The
                        // dummy `Value` is never consumed by IR — `Call`
                        // reads the kind tag (Builtin → resolve overload via
                        // `lookup_overload`; UdfRef → look up the binding in
                        // `plan.udf_call_at` by ip) and emits the actual
                        // `call`. The UdfRef index is unused in codegen, so
                        // any sentinel works.
                        BytecodeOp::LoadGlobal(name) => {
                            let placeholder = b.ins().iconst(I64, 0);
                            if let Some(canon) = builtins::canonical_name(name) {
                                stack.push((placeholder, Kind::Builtin(canon)));
                            } else {
                                stack.push((placeholder, Kind::UdfRef(0)));
                            }
                        }
                        BytecodeOp::Call(n) => {
                            if stack.len() < n + 1 {
                                return Err("jit: stack underflow on Call".into());
                            }
                            let split = stack.len() - n;
                            let raw_args: Vec<(Value, Kind)> = stack.split_off(split);
                            let (_marker_val, marker_kind) =
                                stack.pop().ok_or("jit: missing fn-ref marker")?;
                            match marker_kind {
                                Kind::Builtin(name) => {
                                    let arg_kinds: Vec<Kind> =
                                        raw_args.iter().map(|(_, k)| *k).collect();
                                    let shim_idx = builtins::lookup_overload(name, &arg_kinds)
                                        .ok_or("jit: no shim overload for call")?;
                                    let shim = &SHIMS[shim_idx];
                                    // Convert each arg from its operand kind to the
                                    // shim's ABI kind. `to_i64` saturates floats,
                                    // `to_f64` promotes ints.
                                    let mut cl_args: Vec<Value> =
                                        Vec::with_capacity(raw_args.len() + 1);
                                    for (idx, (v, k)) in raw_args.into_iter().enumerate() {
                                        let abi = shim.args_abi[idx];
                                        let conv = if abi == Kind::Float {
                                            to_f64(&mut b, v, k)
                                        } else {
                                            to_i64(&mut b, v, k)
                                        };
                                        cl_args.push(conv);
                                    }
                                    // v0.99.3 — bailable shims take a trailing
                                    // *mut i64 bail pointer; after the call we
                                    // load *bail and branch to bail_block if
                                    // it's non-zero (mirrors UDF dispatcher).
                                    if shim.bailable {
                                        cl_args.push(b.use_var(bail_var));
                                    }
                                    let call = b.ins().call(shim_refs[shim_idx], &cl_args);
                                    let r = b.inst_results(call)[0];
                                    if shim.bailable {
                                        let bp = b.use_var(bail_var);
                                        let bail_val =
                                            b.ins().load(I64, MemFlags::new(), bp, 0);
                                        let bail_set = b.ins().icmp_imm(
                                            IntCC::NotEqual,
                                            bail_val,
                                            0,
                                        );
                                        let cont = b.create_block();
                                        b.ins().brif(bail_set, bail_block, &[], cont, &[]);
                                        b.switch_to_block(cont);
                                    }
                                    stack.push((r, shim.ret_kind));
                                }
                                Kind::UdfRef(_) => {
                                    // Look up the binding the analyser
                                    // already resolved for this call site.
                                    let binding = plan
                                        .udf_call_at
                                        .get(&ip)
                                        .copied()
                                        .ok_or("jit: UDF call site missing binding")?;
                                    let slot = udf_args_slot
                                        .ok_or("jit: UDF call without stack slot")?;
                                    // Marshal each arg into the slot. Float
                                    // values are bit-cast to i64; Int and
                                    // Boxed (tagged ptr) cross unchanged.
                                    // Matches run_compiled's args encoding.
                                    for (i, (v, k)) in raw_args.iter().enumerate() {
                                        let stored = if *k == Kind::Float {
                                            b.ins().bitcast(
                                                I64,
                                                MemFlags::new(),
                                                *v,
                                            )
                                        } else {
                                            *v
                                        };
                                        b.ins().stack_store(stored, slot, (i * 8) as i32);
                                    }
                                    let args_addr = b.ins().stack_addr(ptr_ty, slot, 0);
                                    let bp = b.use_var(bail_var);
                                    let gid =
                                        b.ins().iconst(I64, binding.global_id as i64);
                                    let sig = b.ins().iconst(I64, binding.sig as i64);
                                    // Pass the caller-speculated ret_kind
                                    // (0/1/2) to the dispatcher so it can
                                    // detect a speculation mismatch against
                                    // the actual cached callee and surface
                                    // `*bail = 2`.
                                    let erk_code: i64 = match binding.ret_kind {
                                        crate::jit::BindingRet::Int => 0,
                                        crate::jit::BindingRet::Float => 1,
                                        crate::jit::BindingRet::Boxed => 2,
                                    };
                                    let erk = b.ins().iconst(I64, erk_code);
                                    let nargs_v = b.ins().iconst(I64, *n as i64);
                                    let call = b.ins().call(
                                        udf_dispatch_ref,
                                        &[gid, sig, erk, args_addr, nargs_v, bp],
                                    );
                                    let raw_result = b.inst_results(call)[0];
                                    // Check the bail flag — the dispatcher
                                    // sets it on cache miss or callee bail.
                                    let bail_val =
                                        b.ins().load(I64, MemFlags::new(), bp, 0);
                                    let bail_set =
                                        b.ins().icmp_imm(IntCC::NotEqual, bail_val, 0);
                                    let cont = b.create_block();
                                    b.ins().brif(bail_set, bail_block, &[], cont, &[]);
                                    b.switch_to_block(cont);
                                    // Re-interpret the i64 result per the
                                    // callee's declared return kind. Float
                                    // returns are an f64 bit pattern packed
                                    // into i64; Boxed returns are a tagged
                                    // pointer (Option-γ) that flows on as a
                                    // Boxed operand and gets reclaimed by
                                    // the engine once the body returns it
                                    // or, if consumed mid-body, by the
                                    // arena drain.
                                    let (result_val, result_kind) = match binding.ret_kind {
                                        crate::jit::BindingRet::Float => (
                                            b.ins().bitcast(F64, MemFlags::new(), raw_result),
                                            Kind::Float,
                                        ),
                                        crate::jit::BindingRet::Boxed => {
                                            (raw_result, Kind::Boxed)
                                        }
                                        crate::jit::BindingRet::Int => {
                                            (raw_result, Kind::Int)
                                        }
                                    };
                                    stack.push((result_val, result_kind));
                                }
                                _ => return Err("jit: Call without a fn-ref marker".into()),
                            }
                        }

                        other => return Err(format!("jit: unsupported op reached codegen: {other:?}")),
                    }
                }

                if !terminated {
                    b.ins().jump(fallthrough.clone()?, &[]);
                }
            }

            b.seal_all_blocks();
            b.finalize();
        }

        let name = format!("cfml_jit_{}", self.func_counter);
        self.func_counter += 1;
        let id = self
            .module
            .declare_function(&name, Linkage::Export, &ctx.func.signature)
            .map_err(|e| e.to_string())?;
        self.module
            .define_function(id, &mut ctx)
            .map_err(|e| e.to_string())?;
        self.module.clear_context(&mut ctx);
        self.module
            .finalize_definitions()
            .map_err(|e| e.to_string())?;
        let code = self.module.get_finalized_function(id);
        // SAFETY: `code` points at freshly-emitted native code for our exact
        // signature; it lives as long as the `JITModule` (owned by the engine).
        Ok(unsafe { std::mem::transmute::<*const u8, CompiledFn>(code) })
    }
}

/// v0.90.0 — promote a stack value to its tagged-Boxed form. A Boxed
/// operand passes through; an `Int` or `Float` operand is shipped to the
/// matching `cfml_jit_box_int` / `cfml_jit_box_float` shim. `Bool` /
/// fn-ref markers should never reach this helper (the analyser rejects).
pub(super) fn ensure_boxed(
    b: &mut FunctionBuilder,
    box_int: cranelift_codegen::ir::FuncRef,
    box_float: cranelift_codegen::ir::FuncRef,
    v: Value,
    k: Kind,
) -> Value {
    match k {
        Kind::Boxed => v,
        Kind::Float => {
            let call = b.ins().call(box_float, &[v]);
            b.inst_results(call)[0]
        }
        Kind::Int => {
            let call = b.ins().call(box_int, &[v]);
            b.inst_results(call)[0]
        }
        _ => {
            // The analyser only ever pushes Int/Float/Boxed onto sites
            // that flow into ensure_boxed (Concat operands). Defensive:
            // emit an iconst-zero so the IR still verifies, but the
            // result will be wrong; this should be unreachable.
            debug_assert!(false, "ensure_boxed: unexpected operand kind");
            b.ins().iconst(I64, 0)
        }
    }
}

/// CFML `CmpOp` → Cranelift signed integer condition (fused loop ops only).
pub(super) fn int_cc(op: CmpOp) -> IntCC {
    match op {
        CmpOp::Lt => IntCC::SignedLessThan,
        CmpOp::Lte => IntCC::SignedLessThanOrEqual,
        CmpOp::Gt => IntCC::SignedGreaterThan,
        CmpOp::Gte => IntCC::SignedGreaterThanOrEqual,
        CmpOp::Eq => IntCC::Equal,
        CmpOp::Neq => IntCC::NotEqual,
    }
}

/// Promote a value to `f64` (int/bool → `fcvt_from_sint`; float passes through).
pub(super) fn to_f64(b: &mut FunctionBuilder, v: Value, k: Kind) -> Value {
    if k == Kind::Float {
        v
    } else {
        b.ins().fcvt_from_sint(F64, v)
    }
}

/// Truncate a value to `i64` (float → saturating `fcvt_to_sint_sat`, matching
/// Rust's `as i64`; int/bool pass through).
pub(super) fn to_i64(b: &mut FunctionBuilder, v: Value, k: Kind) -> Value {
    if k == Kind::Float {
        b.ins().fcvt_to_sint_sat(I64, v)
    } else {
        v
    }
}

/// Emit the integer divide/modulo guard: branch to `bail` when the divisor is
/// `0` or the `INT_MIN / -1` overflow case, returning the continuation block.
pub(super) fn guard_int_div(
    b: &mut FunctionBuilder,
    bail: cranelift_codegen::ir::Block,
    dividend: Value,
    divisor: Value,
) -> cranelift_codegen::ir::Block {
    let is_zero = b.ins().icmp_imm(IntCC::Equal, divisor, 0);
    let div_neg1 = b.ins().icmp_imm(IntCC::Equal, divisor, -1);
    let dvd_min = b.ins().icmp_imm(IntCC::Equal, dividend, i64::MIN);
    let ov = b.ins().band(div_neg1, dvd_min);
    let bad = b.ins().bor(is_zero, ov);
    let cont = b.create_block();
    b.ins().brif(bad, bail, &[], cont, &[]);
    cont
}

/// Materialise a Cranelift boolean (`I8`) into an `i64` `0`/`1`.
pub(super) fn bool_to_i64(b: &mut FunctionBuilder, cond: Value) -> Value {
    let one = b.ins().iconst(I64, 1);
    let zero = b.ins().iconst(I64, 0);
    b.ins().select(cond, one, zero)
}

/// Boolean: `v == 0` (`fcmp`/`icmp` per kind). Used by `Not` and `JumpIfFalse`.
pub(super) fn is_zero_test(b: &mut FunctionBuilder, v: Value, k: Kind) -> Value {
    if k == Kind::Float {
        let z = b.ins().f64const(0.0);
        b.ins().fcmp(FloatCC::Equal, v, z)
    } else {
        b.ins().icmp_imm(IntCC::Equal, v, 0)
    }
}

/// Boolean: `v != 0` (`fcmp`/`icmp` per kind). Truthiness for logical ops.
pub(super) fn is_truthy(b: &mut FunctionBuilder, v: Value, k: Kind) -> Value {
    if k == Kind::Float {
        let z = b.ins().f64const(0.0);
        b.ins().fcmp(FloatCC::NotEqual, v, z)
    } else {
        b.ins().icmp_imm(IntCC::NotEqual, v, 0)
    }
}

pub(super) enum NumOp {
    Add,
    Sub,
    Mul,
}

/// `+ - *`: pop b, pop a. Two ints use wrapping integer ops (bit-exact with the
/// interpreter's `Int(i op j)`); any float operand promotes both and uses the
/// float op (matching the interpreter's int→double promotion on mixed operands).
pub(super) fn num_bin(b: &mut FunctionBuilder, stack: &mut Vec<(Value, Kind)>, op: NumOp) -> Result<(), String> {
    let (rhs, rk) = stack.pop().ok_or("jit: stack underflow")?;
    let (lhs, lk) = stack.pop().ok_or("jit: stack underflow")?;
    if lk == Kind::Float || rk == Kind::Float {
        let a = to_f64(b, lhs, lk);
        let d = to_f64(b, rhs, rk);
        let r = match op {
            NumOp::Add => b.ins().fadd(a, d),
            NumOp::Sub => b.ins().fsub(a, d),
            NumOp::Mul => b.ins().fmul(a, d),
        };
        stack.push((r, Kind::Float));
    } else {
        let r = match op {
            NumOp::Add => b.ins().iadd(lhs, rhs),
            NumOp::Sub => b.ins().isub(lhs, rhs),
            NumOp::Mul => b.ins().imul(lhs, rhs),
        };
        stack.push((r, Kind::Int));
    }
    Ok(())
}

/// v0.99.6 — `+` on polymorphic operands with an inline SMI Int fast path.
///
/// Both operands must already be promoted to tagged i64 (Boxed form); the
/// caller does that via [`ensure_boxed`] when the analyser-declared kind is
/// `Int` or `Float`.
///
/// Emitted IR:
/// ```text
///     entry:
///         a_xor = a XOR TAG_INT
///         b_xor = b XOR TAG_INT
///         both_smi = ((a_xor OR b_xor) AND TAG_MASK) == 0
///         brif both_smi → smi_block, slow_block
///
///     smi_block:
///         ua = a SAR 3            ; arithmetic shift right by 3
///         ub = b SAR 3
///         raw = ua + ub
///         shifted = raw SHL 3
///         recovered = shifted SAR 3
///         fits = recovered == raw   ; SMI-fit overflow check
///         brif fits → smi_ok, smi_overflow
///
///     smi_ok:
///         tagged = shifted OR TAG_INT
///         jump common(tagged)
///
///     smi_overflow:
///         boxed = cfml_jit_box_int(raw)
///         jump common(boxed)
///
///     slow_block:
///         result = cfml_jit_add_boxed(a, b, bail)
///         jump common(result)
///
///     common(result):
///         ; push (result, Kind::Boxed)
/// ```
#[allow(clippy::too_many_arguments)]
pub(super) fn arith_boxed_smi(
    b: &mut FunctionBuilder,
    stack: &mut Vec<(Value, Kind)>,
    op: NumOp,
    box_int: cranelift_codegen::ir::FuncRef,
    box_float: cranelift_codegen::ir::FuncRef,
    slow_shim: cranelift_codegen::ir::FuncRef,
    bail_var: Variable,
    ptr_ty: types::Type,
) -> Result<(), String> {
    let _ = ptr_ty; // bail pointer is i64-sized on every supported target
    let (rhs, rk) = stack.pop().ok_or("jit: stack underflow")?;
    let (lhs, lk) = stack.pop().ok_or("jit: stack underflow")?;
    let a = ensure_boxed(b, box_int, box_float, lhs, lk);
    let bv = ensure_boxed(b, box_int, box_float, rhs, rk);

    // Tag-discriminate.
    let a_xor = b.ins().bxor_imm(a, super::boxed::TAG_INT as i64);
    let b_xor = b.ins().bxor_imm(bv, super::boxed::TAG_INT as i64);
    let or = b.ins().bor(a_xor, b_xor);
    let masked = b.ins().band_imm(or, super::boxed::TAG_MASK as i64);
    let both_smi = b.ins().icmp_imm(IntCC::Equal, masked, 0);

    let smi_block = b.create_block();
    let slow_block = b.create_block();
    let smi_ok = b.create_block();
    let smi_overflow = b.create_block();
    let common = b.create_block();
    b.append_block_param(common, I64);

    b.ins().brif(both_smi, smi_block, &[], slow_block, &[]);

    // SMI block: untag, apply op, retag with SMI-fit check.
    b.switch_to_block(smi_block);
    let ua = b.ins().sshr_imm(a, 3);
    let ub = b.ins().sshr_imm(bv, 3);
    let raw = match op {
        NumOp::Add => b.ins().iadd(ua, ub),
        NumOp::Sub => b.ins().isub(ua, ub),
        NumOp::Mul => b.ins().imul(ua, ub),
    };
    let shifted = b.ins().ishl_imm(raw, 3);
    let recovered = b.ins().sshr_imm(shifted, 3);
    let fits = b.ins().icmp(IntCC::Equal, recovered, raw);
    b.ins().brif(fits, smi_ok, &[], smi_overflow, &[]);

    // SMI ok.
    b.switch_to_block(smi_ok);
    let tagged = b.ins().bor_imm(shifted, super::boxed::TAG_INT as i64);
    let tagged_arg: cranelift_codegen::ir::BlockArg = tagged.into();
    b.ins().jump(common, &[tagged_arg]);

    // SMI overflow.
    b.switch_to_block(smi_overflow);
    let boxed = {
        let call = b.ins().call(box_int, &[raw]);
        b.inst_results(call)[0]
    };
    let boxed_arg: cranelift_codegen::ir::BlockArg = boxed.into();
    b.ins().jump(common, &[boxed_arg]);

    // Slow block.
    b.switch_to_block(slow_block);
    let bp = b.use_var(bail_var);
    let call = b.ins().call(slow_shim, &[a, bv, bp]);
    let slow_r = b.inst_results(call)[0];
    let slow_arg: cranelift_codegen::ir::BlockArg = slow_r.into();
    b.ins().jump(common, &[slow_arg]);

    // Common.
    b.switch_to_block(common);
    let result = b.block_params(common)[0];
    stack.push((result, Kind::Boxed));
    Ok(())
}

/// Comparison: pop b, pop a, push `(a CMP b)` as i64 `0`/`1`. Two ints compare
/// with `icmp`; a float operand promotes both and uses `fcmp` (matching the
/// interpreter, which compares mixed int/double in f64).
pub(super) fn cmp(
    b: &mut FunctionBuilder,
    stack: &mut Vec<(Value, Kind)>,
    icc: IntCC,
    fcc: FloatCC,
) -> Result<(), String> {
    let (rhs, rk) = stack.pop().ok_or("jit: stack underflow")?;
    let (lhs, lk) = stack.pop().ok_or("jit: stack underflow")?;
    let c = if lk == Kind::Float || rk == Kind::Float {
        let a = to_f64(b, lhs, lk);
        let d = to_f64(b, rhs, rk);
        b.ins().fcmp(fcc, a, d)
    } else {
        b.ins().icmp(icc, lhs, rhs)
    };
    let r = bool_to_i64(b, c);
    stack.push((r, Kind::Bool));
    Ok(())
}

pub(super) enum LogicOp {
    And,
    Or,
    Xor,
}

/// Logical op on truthiness: pop b, pop a, push `(a≠0 OP b≠0)` as i64 `0`/`1`.
pub(super) fn logic2(b: &mut FunctionBuilder, stack: &mut Vec<(Value, Kind)>, op: LogicOp) -> Result<(), String> {
    let (rhs, rk) = stack.pop().ok_or("jit: stack underflow")?;
    let (lhs, lk) = stack.pop().ok_or("jit: stack underflow")?;
    let at = is_truthy(b, lhs, lk);
    let bt = is_truthy(b, rhs, rk);
    let r = match op {
        LogicOp::And => b.ins().band(at, bt),
        LogicOp::Or => b.ins().bor(at, bt),
        LogicOp::Xor => b.ins().bxor(at, bt),
    };
    let r = bool_to_i64(b, r);
    stack.push((r, Kind::Bool));
    Ok(())
}

/// Read-modify-write an integer local by a constant addend: `local += k`.
fn rmw_imm(
    b: &mut FunctionBuilder,
    vars: &[Variable],
    plan: &Plan,
    name: &str,
    k: i64,
) -> Result<(), String> {
    let slot = plan.slot_of(name).ok_or("jit: unknown local")?;
    let v = b.use_var(vars[slot]);
    let nv = b.ins().iadd_imm(v, k);
    b.def_var(vars[slot], nv);
    Ok(())
}
