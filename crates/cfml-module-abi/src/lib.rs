//! The stable C ABI between RustCFML and a dynamically loaded native extension.
//!
//! Nothing in here is a Rust type whose layout could change: every struct is
//! `#[repr(C)]`, every value crossing the boundary is an opaque handle into a
//! host-owned slab, and every function is a plain `extern "C"` pointer. That is
//! the whole point — an extension built with a different rustc than the host
//! keeps working, because no Rust layout is part of the contract. See
//! `planning/NATIVE_EXTENSIONS_PLAN.md` §3 for why this beat the Rust-ABI
//! alternative.
//!
//! # The three rules an author has to know
//!
//! 1. **Every entry point takes `ctx` first**, even in tier 1 where it can do
//!    almost nothing. Adding scope access and CFML re-entry later then appends
//!    vtable entries instead of breaking every published extension.
//! 2. **The host owns every value.** A [`ValueHandle`] is an index plus a
//!    generation tag, valid only for the duration of the call that produced it.
//!    Using a stale one is a clean error, never a dereference.
//! 3. **The vtable is size-versioned and append-only.** Check
//!    [`HostVtable::has`] before touching anything past tier 1.

#![deny(improper_ctypes_definitions)]

use core::ffi::c_void;

/// Bumped only for a genuinely breaking change. Given the vtable grows by
/// appending, this should approximately never move.
pub const ABI_MAJOR: u32 = 1;

/// The triple this copy of the crate was compiled for — half the compatibility
/// token (§4.8). Baked in by `build.rs`.
pub const TARGET: &str = env!("CFML_ABI_TARGET");

/// Capability tiers (§4.3). A module declares the tier it *requires*; a host
/// declares the tier it *implements* and refuses anything higher.
pub mod tier {
    /// Pure functions over CFML values, plus native classes holding their own
    /// Rust state. No CFML executes, so no re-entrancy concerns.
    pub const VALUES: u32 = 1;
    /// Adds the scope facade: read/write `application`, `session`, … and take
    /// the same locks `<cflock>` uses. Still executes no CFML.
    pub const SCOPES: u32 = 2;
    /// Adds CFML execution — calling UDFs and closures, instantiating
    /// components, invoking methods, page output, and `include`.
    pub const EXECUTION: u32 = 3;
    /// Adds CFML exceptions as data, and named arguments. An error raised by
    /// CFML the extension called can be taken out of the call — caught,
    /// inspected, and either handled or raised again — the way a Lucee
    /// extension catches the `PageException` from `udf.call( pc, … )`. And a
    /// function or method can be called with named arguments, the ABI form of
    /// `argumentCollection` (Lucee's `udf.callWithNamedValues`).
    pub const EXCEPTIONS: u32 = 4;
}

/// Status returned by fallible vtable entries. Zero is success.
pub mod status {
    pub const OK: u32 = 0;
    /// The handle is stale, out of range, or from a different call.
    pub const BAD_HANDLE: u32 = 1;
    /// The handle is live but holds the wrong kind of value.
    pub const WRONG_TYPE: u32 = 2;
    /// An index or key that does not exist.
    pub const NOT_FOUND: u32 = 3;
    /// The `ctx` is not valid for this call (stored and reused, most likely).
    pub const BAD_CTX: u32 = 4;
    /// The host does not implement this entry.
    pub const UNSUPPORTED: u32 = 5;
    /// A write to a shared scope was attempted without holding its lock.
    pub const UNLOCKED: u32 = 6;
    /// A lock could not be acquired within its timeout.
    pub const TIMEOUT: u32 = 7;
    /// No scope of that name, or no VM in this context.
    pub const NO_SCOPE: u32 = 8;
    /// The pending error is not an exception but a request ending — an
    /// `abort`, a `cflocation` redirect or a request timeout. Like
    /// `catch( any )` in CFML, an extension cannot take it: it is raised when
    /// the call returns, whatever the extension does.
    pub const NOT_CATCHABLE: u32 = 9;
}

/// Type discriminants reported by [`HostVtable::val_type`].
///
/// These mirror `CfmlValue`'s variants, collapsing the ones an extension
/// cannot tell apart anyway (a query column reads as an array).
pub mod ty {
    pub const NULL: u32 = 0;
    pub const BOOL: u32 = 1;
    pub const INT: u32 = 2;
    pub const DOUBLE: u32 = 3;
    pub const TIMESPAN: u32 = 4;
    pub const STRING: u32 = 5;
    pub const ARRAY: u32 = 6;
    pub const STRUCT: u32 = 7;
    pub const BINARY: u32 = 8;
    pub const QUERY: u32 = 9;
    pub const FUNCTION: u32 = 10;
    pub const COMPONENT: u32 = 11;
    pub const NATIVE: u32 = 12;
}

/// CFML error categories, for [`HostVtable::throw`]. `CUSTOM` uses the
/// `custom_type` string.
pub mod error_type {
    pub const APPLICATION: u32 = 0;
    pub const EXPRESSION: u32 = 1;
    pub const DATABASE: u32 = 2;
    pub const SECURITY: u32 = 3;
    pub const IO: u32 = 4;
    pub const CUSTOM: u32 = 5;
}

/// A borrowed string. Valid for as long as whatever produced it says it is —
/// for values read out of the slab, that is the duration of the current call.
#[repr(C)]
#[derive(Copy, Clone)]
pub struct StrRef {
    pub ptr: *const u8,
    pub len: usize,
}

impl StrRef {
    pub const EMPTY: StrRef = StrRef { ptr: core::ptr::null(), len: 0 };

    /// Borrow a Rust string. The caller must keep the source alive.
    pub const fn new(s: &str) -> StrRef {
        StrRef { ptr: s.as_ptr(), len: s.len() }
    }

    /// # Safety
    /// `ptr`/`len` must describe live, valid UTF-8 for the returned lifetime.
    pub unsafe fn as_str<'a>(self) -> &'a str {
        if self.ptr.is_null() || self.len == 0 {
            return "";
        }
        core::str::from_utf8_unchecked(core::slice::from_raw_parts(self.ptr, self.len))
    }
}

/// An opaque, host-owned value.
///
/// `slot` indexes the host's per-call slab; `gen` tags the call it belongs to.
/// A handle used after its call has returned is reported as
/// [`status::BAD_HANDLE`] rather than dereferenced — which is why the slab is
/// truncated rather than freed, and why the generation is checked on every
/// access.
#[repr(C)]
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub struct ValueHandle {
    pub slot: u32,
    pub gen: u32,
}

impl ValueHandle {
    /// The "no value" handle. Returned by a failed accessor and by an entry
    /// point that threw; never a valid slot.
    pub const NULL: ValueHandle = ValueHandle { slot: u32::MAX, gen: 0 };

    /// "Return the receiver." Only meaningful as the return of a native class
    /// method, where it makes a fluent mutator hand back the *same* object
    /// rather than a copy — the module has no handle to itself, and the host
    /// does. Returned from anywhere else it is an error, not a null.
    pub const SELF: ValueHandle = ValueHandle { slot: u32::MAX - 1, gen: 0 };

    pub const fn is_null(self) -> bool {
        self.slot == u32::MAX
    }

    pub const fn is_self(self) -> bool {
        self.slot == u32::MAX - 1
    }
}

/// Opaque per-call VM context. Valid ONLY for the duration of the call that
/// received it; it carries its own generation tag so a stored-and-reused `ctx`
/// is [`status::BAD_CTX`] rather than undefined behaviour.
#[repr(C)]
pub struct Ctx {
    _private: [u8; 0],
}

/// The signature every module entry point has.
///
/// `C-unwind` so a panic in the module unwinds into the host's `catch_unwind`
/// shim and becomes a CFML error, instead of aborting the process.
pub type ModuleFn = unsafe extern "C-unwind" fn(
    ctx: *mut Ctx,
    args: *const ValueHandle,
    argc: usize,
) -> ValueHandle;

/// Constructor for a module-provided native class. Writes the instance pointer
/// through `out` and returns [`status::OK`]; on failure it should call
/// `throw` and return non-zero.
pub type ClassCtorFn = unsafe extern "C-unwind" fn(
    ctx: *mut Ctx,
    args: *const ValueHandle,
    argc: usize,
    out: *mut *mut c_void,
) -> u32;

/// The method table for a module-provided native class (§4.6).
///
/// The host implements `CfmlNative` over this, so `component extends="rust:X"`,
/// `super.method()` and `this.X` fall-through all keep working — they only ever
/// see an ordinary native object.
///
/// **`Send + Sync` is asserted by contract**: the module is responsible for its
/// own interior synchronisation. That requirement is what lets the host skip
/// the exclusive lock on dispatch when re-entrancy arrives at tier 3.
#[repr(C)]
pub struct NativeClassVtable {
    pub size: usize,
    /// Logical class name, for `getMetadata` and dumps.
    pub class_name: unsafe extern "C" fn(data: *mut c_void) -> StrRef,
    pub call_method: unsafe extern "C-unwind" fn(
        ctx: *mut Ctx,
        data: *mut c_void,
        name: StrRef,
        args: *const ValueHandle,
        argc: usize,
    ) -> ValueHandle,
    /// Comma-separated parameter names for `name`, in positional order, so the
    /// host can bind a NAMED call. Return [`StrRef::EMPTY`] with a non-zero
    /// return to mean "not declared", which makes the host refuse named
    /// arguments rather than silently binding them positionally.
    pub method_params:
        unsafe extern "C" fn(data: *mut c_void, name: StrRef, out: *mut StrRef) -> u32,
    /// `this.X` read fall-through. Return [`ValueHandle::NULL`] with non-zero
    /// to decline and let the CFC's own struct answer.
    pub get_property:
        unsafe extern "C-unwind" fn(ctx: *mut Ctx, data: *mut c_void, name: StrRef, out: *mut ValueHandle) -> u32,
    /// `this.X` write fall-through. Non-zero declines.
    pub set_property: unsafe extern "C-unwind" fn(
        ctx: *mut Ctx,
        data: *mut c_void,
        name: StrRef,
        value: ValueHandle,
    ) -> u32,
    /// Release the instance. Called when the host drops the object.
    pub drop_fn: unsafe extern "C" fn(data: *mut c_void),
}

/// The method table for a module-provided **database driver** (§4.7).
///
/// The host implements `cfml_stdlib::DynamicDbDriver` over this, so a datasource
/// backed by an extension reaches `queryExecute`, `<cfquery>`, `cftransaction`
/// and `cfdbinfo` through exactly the paths a bundled driver uses.
///
/// Only `execute` is required. Every other entry is `Option`: a driver that
/// leaves `begin` null reports "transactions not supported" the way the engine
/// already does for a dynamic datasource, and one that leaves `dbinfo` null
/// falls through to the built-in metadata handling. A driver therefore starts
/// useful and grows, rather than having to implement all of it to load.
///
/// **`Send + Sync` is asserted by contract**, as for [`NativeClassVtable`]: the
/// module owns its own synchronisation. A datasource is shared by every request.
#[repr(C)]
pub struct DbDriverVtable {
    pub size: usize,
    /// Run one statement. `params` is an Array handle of already-normalised
    /// values (the host flattens `cfqueryparam` structs before calling), and
    /// `return_type` is `"query"`, `"array"` or `"struct:<columnKey>"`.
    pub execute: unsafe extern "C-unwind" fn(
        ctx: *mut Ctx,
        data: *mut c_void,
        sql: StrRef,
        params: ValueHandle,
        return_type: StrRef,
    ) -> ValueHandle,
    /// Open a transaction, writing an opaque module-owned handle through `out`.
    /// Null means this driver has no transactions.
    pub begin: Option<
        unsafe extern "C-unwind" fn(
            ctx: *mut Ctx,
            data: *mut c_void,
            isolation: StrRef,
            out: *mut *mut c_void,
        ) -> u32,
    >,
    /// Run a statement inside the transaction `txn`. Required when `begin` is set.
    pub txn_execute: Option<
        unsafe extern "C-unwind" fn(
            ctx: *mut Ctx,
            txn: *mut c_void,
            sql: StrRef,
            params: ValueHandle,
            return_type: StrRef,
        ) -> ValueHandle,
    >,
    /// Commit and release `txn`. Required when `begin` is set.
    pub txn_commit: Option<unsafe extern "C-unwind" fn(ctx: *mut Ctx, txn: *mut c_void) -> u32>,
    /// Roll back and release `txn`. Required when `begin` is set.
    pub txn_rollback: Option<unsafe extern "C-unwind" fn(ctx: *mut Ctx, txn: *mut c_void) -> u32>,
    /// Savepoint verbs. `op` is one of [`savepoint_op`]. Null means the driver
    /// has no savepoints, which makes a NESTED `transaction { }` an error
    /// instead of silently flattening into the outer one.
    pub txn_savepoint: Option<
        unsafe extern "C-unwind" fn(
            ctx: *mut Ctx,
            txn: *mut c_void,
            op: u32,
            name: StrRef,
        ) -> u32,
    >,
    /// `cfdbinfo`. `attrs` is a Struct handle of the tag's attributes. Return
    /// [`ValueHandle::NULL`] with [`status::OK`] to decline a particular
    /// `info_type` and let the engine answer.
    pub dbinfo: Option<
        unsafe extern "C-unwind" fn(
            ctx: *mut Ctx,
            data: *mut c_void,
            info_type: StrRef,
            attrs: ValueHandle,
        ) -> ValueHandle,
    >,
    /// Release the driver instance. Called when the host drops it.
    pub drop_fn: unsafe extern "C" fn(data: *mut c_void),
}

/// `op` values for [`DbDriverVtable::txn_savepoint`].
pub mod savepoint_op {
    pub const SET: u32 = 0;
    pub const RELEASE: u32 = 1;
    pub const ROLLBACK_TO: u32 = 2;
}

/// One database driver a module provides.
///
/// `name` is the driver KIND, as a datasource's `driver` key names it. The
/// manifest declares it too, so two extensions claiming the same kind are
/// caught before either is loaded. `ctor` is called once per datasource
/// configured with that kind, receiving its settings.
#[repr(C)]
pub struct DbDriverDecl {
    pub name: StrRef,
    pub ctor: ClassCtorFn,
    pub vtable: *const DbDriverVtable,
}

/// One BIF a module provides.
#[repr(C)]
pub struct BifDecl {
    pub name: StrRef,
    pub entry: ModuleFn,
}

/// One native class a module provides.
#[repr(C)]
pub struct ClassDecl {
    pub name: StrRef,
    pub ctor: ClassCtorFn,
    pub vtable: *const NativeClassVtable,
}

/// One QoQ SQL function a module provides.
///
/// **Declared but not yet accepted.** A host that cannot register these refuses
/// the whole extension rather than loading it with the functions missing, so a
/// query can never silently lose one.
#[repr(C)]
pub struct QoqFnDecl {
    pub name: StrRef,
    pub entry: ModuleFn,
    /// 0 = scalar, 1 = aggregate.
    pub kind: u32,
}

/// What `rustcfml_module_decl()` returns: everything the host needs to decide
/// whether it can load this extension, and what it provides if it can.
#[repr(C)]
pub struct ModuleDecl {
    pub size: usize,
    pub abi_major: u32,
    /// The tier this module REQUIRES. A host implementing less refuses it.
    pub tier: u32,
    pub name: StrRef,
    pub version: StrRef,
    /// The triple the module was built for. Compared against the host's.
    pub target: StrRef,
    /// Once per process, after the vtable is handed over: thread pools, caches,
    /// anything expensive. `config` is the extension's `.cfconfig.json` block
    /// as a struct handle (or NULL). Non-zero return refuses the load.
    pub on_load: Option<
        unsafe extern "C-unwind" fn(vtable: *const HostVtable, ctx: *mut Ctx, config: ValueHandle) -> u32,
    >,
    pub bifs: *const BifDecl,
    pub bif_count: usize,
    pub classes: *const ClassDecl,
    pub class_count: usize,
    pub qoq_fns: *const QoqFnDecl,
    pub qoq_fn_count: usize,
    /// Database drivers. APPENDED — a host reading this must check `size`
    /// first, because an extension compiled before these existed emits a
    /// shorter struct and these two fields are then past the end of it.
    pub drivers: *const DbDriverDecl,
    pub driver_count: usize,
}

// A ModuleDecl is built from statics in the module and only ever read by the
// host; the raw pointers are to immutable module-lifetime data.
unsafe impl Sync for ModuleDecl {}
unsafe impl Sync for BifDecl {}
unsafe impl Sync for ClassDecl {}
unsafe impl Sync for QoqFnDecl {}
unsafe impl Sync for NativeClassVtable {}
unsafe impl Sync for DbDriverDecl {}
unsafe impl Sync for DbDriverVtable {}

/// The symbol the loader looks for.
pub const DECL_SYMBOL: &[u8] = b"rustcfml_module_decl";

/// The host services an extension can call, as a size-versioned, append-only
/// table (§4.2).
///
/// `size` is `size_of::<HostVtable>()` **as the host sees it**. A module built
/// against a newer ABI checks [`HostVtable::has`] before calling anything past
/// the tier-1 block, so an old host degrades with a legible error instead of
/// jumping through uninitialised memory.
#[repr(C)]
pub struct HostVtable {
    pub size: usize,
    pub abi_major: u32,
    /// The highest tier this host implements.
    pub tier: u32,

    // ---- tier 1: reading values ------------------------------------------
    pub val_type: unsafe extern "C" fn(*mut Ctx, ValueHandle) -> u32,
    pub val_bool: unsafe extern "C" fn(*mut Ctx, ValueHandle, *mut bool) -> u32,
    pub val_i64: unsafe extern "C" fn(*mut Ctx, ValueHandle, *mut i64) -> u32,
    pub val_f64: unsafe extern "C" fn(*mut Ctx, ValueHandle, *mut f64) -> u32,
    /// Borrowed UTF-8, valid until the call returns. Only for a String value;
    /// use `val_to_string` to coerce anything else.
    pub val_str: unsafe extern "C" fn(*mut Ctx, ValueHandle, *mut StrRef) -> u32,
    pub val_bytes: unsafe extern "C" fn(*mut Ctx, ValueHandle, *mut StrRef) -> u32,
    /// CFML's own stringification, for any value.
    pub val_to_string: unsafe extern "C" fn(*mut Ctx, ValueHandle) -> ValueHandle,
    /// CFML truthiness (`is_true`), for any value.
    pub val_is_true: unsafe extern "C" fn(*mut Ctx, ValueHandle, *mut bool) -> u32,

    // ---- tier 1: creating values -----------------------------------------
    pub new_null: unsafe extern "C" fn(*mut Ctx) -> ValueHandle,
    pub new_bool: unsafe extern "C" fn(*mut Ctx, bool) -> ValueHandle,
    pub new_int: unsafe extern "C" fn(*mut Ctx, i64) -> ValueHandle,
    pub new_double: unsafe extern "C" fn(*mut Ctx, f64) -> ValueHandle,
    pub new_timespan: unsafe extern "C" fn(*mut Ctx, f64) -> ValueHandle,
    pub new_string: unsafe extern "C" fn(*mut Ctx, StrRef) -> ValueHandle,
    pub new_binary: unsafe extern "C" fn(*mut Ctx, StrRef) -> ValueHandle,

    // ---- tier 1: arrays ---------------------------------------------------
    pub arr_new: unsafe extern "C" fn(*mut Ctx, usize) -> ValueHandle,
    pub arr_len: unsafe extern "C" fn(*mut Ctx, ValueHandle, *mut usize) -> u32,
    /// Zero-based, unlike CFML's own indexing — an ABI is not the place for
    /// 1-based arithmetic.
    pub arr_get: unsafe extern "C" fn(*mut Ctx, ValueHandle, usize) -> ValueHandle,
    pub arr_set: unsafe extern "C" fn(*mut Ctx, ValueHandle, usize, ValueHandle) -> u32,
    pub arr_push: unsafe extern "C" fn(*mut Ctx, ValueHandle, ValueHandle) -> u32,

    // ---- tier 1: structs --------------------------------------------------
    pub struct_new: unsafe extern "C" fn(*mut Ctx) -> ValueHandle,
    pub struct_len: unsafe extern "C" fn(*mut Ctx, ValueHandle, *mut usize) -> u32,
    /// Case-insensitive, like every CFML key lookup.
    pub struct_get: unsafe extern "C" fn(*mut Ctx, ValueHandle, StrRef) -> ValueHandle,
    pub struct_set: unsafe extern "C" fn(*mut Ctx, ValueHandle, StrRef, ValueHandle) -> u32,
    pub struct_has: unsafe extern "C" fn(*mut Ctx, ValueHandle, StrRef, *mut bool) -> u32,
    pub struct_delete: unsafe extern "C" fn(*mut Ctx, ValueHandle, StrRef) -> u32,
    /// Key at insertion-order position `idx`, so a module can walk a struct
    /// without the host materialising a key array.
    pub struct_key_at: unsafe extern "C" fn(*mut Ctx, ValueHandle, usize, *mut StrRef) -> u32,

    // ---- tier 1: queries --------------------------------------------------
    //
    // Modelled as a full accessor set rather than a borrowed columnar view.
    // Column-major storage behind an `Arc` cannot be handed out as a raw slice
    // without pinning the host's representation into the ABI, which is the one
    // thing this design refuses to do. `query_col_values` covers the bulk case
    // by materialising one column into an array, which is a single crossing
    // for the whole column rather than one per cell.
    pub query_new: unsafe extern "C" fn(*mut Ctx, ValueHandle) -> ValueHandle,
    pub query_cols: unsafe extern "C" fn(*mut Ctx, ValueHandle, *mut usize) -> u32,
    pub query_col_name: unsafe extern "C" fn(*mut Ctx, ValueHandle, usize, *mut StrRef) -> u32,
    pub query_col_index: unsafe extern "C" fn(*mut Ctx, ValueHandle, StrRef, *mut usize) -> u32,
    pub query_rows: unsafe extern "C" fn(*mut Ctx, ValueHandle, *mut usize) -> u32,
    pub query_cell: unsafe extern "C" fn(*mut Ctx, ValueHandle, usize, usize) -> ValueHandle,
    pub query_set_cell: unsafe extern "C" fn(*mut Ctx, ValueHandle, usize, usize, ValueHandle) -> u32,
    /// Append a row from an array of cell values, in column order.
    pub query_add_row: unsafe extern "C" fn(*mut Ctx, ValueHandle, ValueHandle) -> u32,
    /// A whole column as an array — one crossing instead of `rows` crossings.
    pub query_col_values: unsafe extern "C" fn(*mut Ctx, ValueHandle, usize) -> ValueHandle,

    // ---- tier 1: components, functions, natives ---------------------------
    //
    // CALLING these needs tier 3, but accepting one as an argument, holding it
    // and handing it back is useful now — and it means the tier-3 upgrade adds
    // verbs, not nouns.
    pub component_name: unsafe extern "C" fn(*mut Ctx, ValueHandle, *mut StrRef) -> u32,
    pub native_class_name: unsafe extern "C" fn(*mut Ctx, ValueHandle, *mut StrRef) -> u32,

    /// Wrap a module-owned instance into a CFML native object, so a BIF can
    /// return one of the module's own classes (`Document()` → a Document).
    /// The host takes ownership and calls `vtable.drop_fn` when the value dies.
    pub new_native: unsafe extern "C" fn(
        *mut Ctx,
        data: *mut c_void,
        vtable: *const NativeClassVtable,
    ) -> ValueHandle,

    // ---- tier 1: errors ---------------------------------------------------
    /// Raise a CFML error. The module then returns [`ValueHandle::NULL`]. The
    /// host builds the `CfmlError` and fills in the stack trace as it always
    /// does; `extras` (a struct handle, or NULL) lands on `cfcatch`.
    pub throw: unsafe extern "C" fn(
        *mut Ctx,
        error_type: u32,
        custom_type: StrRef,
        message: StrRef,
        extras: ValueHandle,
    ),

    // ---- tier 2: the scope facade ----------------------------------------
    //
    // APPENDED, never reordered: an extension built against tier 1 has a
    // smaller `size` and simply never reaches these, and an extension built
    // against tier 2 checks `HostVtable::has` before calling them. That is the
    // whole reason `ctx` was in every signature from day one.
    //
    // Scope names are the CFML ones: "variables", "request", "session",
    // "application", "server", "cgi", "url", "form", "cookie".

    /// Read one key from a named scope. Takes that scope's read lock for the
    /// duration, so the value is never torn by a concurrent CFML `<cflock>`.
    pub scope_get: unsafe extern "C" fn(*mut Ctx, scope: StrRef, key: StrRef) -> ValueHandle,
    /// Write one key into a named scope.
    ///
    /// Writing a **shared** scope (`application`, `session`, `server`) requires
    /// a lock this call is already holding, acquired via [`HostVtable::lock`];
    /// without one this returns [`status::UNLOCKED`] rather than succeeding.
    /// Stricter than CFML itself, deliberately — a native module can write a
    /// live shared scope from a thread the application never thinks about.
    pub scope_set: unsafe extern "C" fn(*mut Ctx, scope: StrRef, key: StrRef, ValueHandle) -> u32,
    pub scope_has: unsafe extern "C" fn(*mut Ctx, scope: StrRef, key: StrRef, *mut bool) -> u32,
    pub scope_delete: unsafe extern "C" fn(*mut Ctx, scope: StrRef, key: StrRef) -> u32,
    /// The whole scope as a **snapshot** struct — a copy, safe to walk without
    /// holding anything. Iterating a live shared scope key by key would race.
    pub scope_snapshot: unsafe extern "C" fn(*mut Ctx, scope: StrRef) -> ValueHandle,
    /// An unqualified read, honouring CFML's own resolution order
    /// (local → arguments → variables → request → … → application → server).
    pub var_get: unsafe extern "C" fn(*mut Ctx, key: StrRef) -> ValueHandle,

    /// Acquire a lock, in the **same registry `<cflock>` uses**, so a CFML
    /// `<cflock scope="application">` and a native write mutually exclude.
    ///
    /// `scope` names a scope lock; pass an empty `scope` and a non-empty `name`
    /// for a named lock. `timeout_ms == 0` means wait forever (Lucee
    /// semantics). A timeout raises a `lock`-typed CFML error carrying
    /// `LockOperation = "Timeout"`.
    ///
    /// The token is written through `out`. Guards are **call-scoped**: anything
    /// still held when the call returns is force-released, because a module
    /// holding a lock across requests is a hang, not a bug report.
    pub lock: unsafe extern "C" fn(
        *mut Ctx,
        scope: StrRef,
        name: StrRef,
        exclusive: bool,
        timeout_ms: u64,
        out: *mut u64,
    ) -> u32,
    /// Release a lock early. Optional — the host releases at call end anyway.
    pub unlock: unsafe extern "C" fn(*mut Ctx, token: u64) -> u32,

    /// Keep a value alive beyond this call, for a cache that spans requests.
    ///
    /// The host owns the root; the id is valid until [`HostVtable::unroot`].
    /// Rooted values are visible to the cycle collector, so a cache cannot be
    /// collected out from under a module.
    pub root: unsafe extern "C" fn(*mut Ctx, ValueHandle, out: *mut u64) -> u32,
    /// Release a root. Deliberately takes no `ctx`: a root outlives every call,
    /// so its RAII drop has none to offer.
    pub unroot: unsafe extern "C" fn(id: u64) -> u32,
    /// Bring a rooted value back as a handle in the current call.
    pub root_get: unsafe extern "C" fn(*mut Ctx, id: u64) -> ValueHandle,

    // ---- tier 3: CFML execution -------------------------------------------
    //
    // Everything here re-enters the engine, which is why the object-dispatch
    // lock had to become optional first: an extension method that calls CFML
    // which calls back into the same object would otherwise deadlock on the
    // guard its own caller is holding.

    /// Call a function by name — a BIF, a UDF, or anything in scope.
    pub call_fn: unsafe extern "C-unwind" fn(
        *mut Ctx,
        name: StrRef,
        args: *const ValueHandle,
        argc: usize,
    ) -> ValueHandle,
    /// Call a function VALUE: a UDF or closure handed to you as an argument.
    /// This is what makes `page.onRequest( function(req){ … } )` possible.
    pub call_value: unsafe extern "C-unwind" fn(
        *mut Ctx,
        callee: ValueHandle,
        args: *const ValueHandle,
        argc: usize,
    ) -> ValueHandle,
    /// `createObject( "component", path )`, constructor arguments included.
    pub new_component: unsafe extern "C-unwind" fn(
        *mut Ctx,
        path: StrRef,
        args: *const ValueHandle,
        argc: usize,
    ) -> ValueHandle,
    /// Invoke a method on a component or native object.
    pub invoke_method: unsafe extern "C-unwind" fn(
        *mut Ctx,
        object: ValueHandle,
        name: StrRef,
        args: *const ValueHandle,
        argc: usize,
    ) -> ValueHandle,
    /// Set a property on a component — annotation-driven dependency injection
    /// is the reason this exists separately from `struct_set`.
    pub component_set: unsafe extern "C-unwind" fn(
        *mut Ctx,
        object: ValueHandle,
        name: StrRef,
        value: ValueHandle,
    ) -> u32,
    /// `getComponentMetadata( path )`. Metadata matters more than it looks:
    /// annotation-driven DI is metadata-driven.
    pub component_metadata: unsafe extern "C-unwind" fn(*mut Ctx, path: StrRef) -> ValueHandle,
    /// Append to the page output buffer, honouring whatever capture is in
    /// effect (`cfsavecontent`, `cfsilent`, a thread's buffer).
    pub write_output: unsafe extern "C-unwind" fn(*mut Ctx, text: StrRef) -> u32,
    /// `<cfinclude>` a template.
    pub include_template: unsafe extern "C-unwind" fn(*mut Ctx, path: StrRef) -> u32,

    // ---- tier 4: CFML exceptions as data, named arguments ------------------
    //
    // When CFML that a tier-3 entry ran throws, the entry returns NULL (or a
    // failure status) and the host STAGES the CFML error in this call; it is
    // raised when the call returns. Until tier 4 nothing could un-stage it, so
    // an extension that handled the failure still had it raised. `error_take`
    // is the catch: it removes the staged error and hands it over as data.
    // An extension that never calls it behaves exactly as before.

    /// Take the error staged by the last failed re-entrant call (`call_fn`,
    /// `call_value`, `new_component`, `invoke_method`, `component_metadata`,
    /// `include_template`, `lock`, and the named forms below), so it is NOT
    /// raised when this call returns.
    ///
    /// Writes the exception through `info` as the struct a CFML
    /// `catch ( any e )` would see — `type`, `message`, `detail`,
    /// `extendedInfo`, `errorCode`, `tagContext`, `stackTrace` and any custom
    /// members — and through `token` an id [`HostVtable::error_restore`]
    /// accepts. [`status::NOT_FOUND`] when nothing is staged;
    /// [`status::NOT_CATCHABLE`] for an abort, redirect or request timeout,
    /// which stays staged.
    pub error_take: unsafe extern "C" fn(*mut Ctx, info: *mut ValueHandle, token: *mut u64) -> u32,
    /// Stage a taken error again, exactly as it was — type, message, extras
    /// and stack — so propagating a caught exception loses nothing. First
    /// error wins, as for `throw`: a no-op if an error is already staged.
    /// [`status::NOT_FOUND`] for a token from another call.
    pub error_restore: unsafe extern "C" fn(*mut Ctx, token: u64) -> u32,
    /// Call a function value with NAMED arguments — the ABI form of
    /// `fn( argumentCollection = args )`. `args` is a struct whose keys bind to
    /// parameter names (a positive-integer key is positional, as in CFML; a
    /// key that matches no parameter lands in `arguments`), or an array of
    /// positional arguments.
    pub call_value_named: unsafe extern "C-unwind" fn(
        *mut Ctx,
        callee: ValueHandle,
        args: ValueHandle,
    ) -> ValueHandle,
    /// Invoke a method with NAMED arguments — `invoke( object, name, args )`,
    /// with `args` as for [`HostVtable::call_value_named`].
    pub invoke_method_named: unsafe extern "C-unwind" fn(
        *mut Ctx,
        object: ValueHandle,
        name: StrRef,
        args: ValueHandle,
    ) -> ValueHandle,
}

impl HostVtable {
    /// Whether the host is new enough to have the entry at `byte_offset`.
    ///
    /// Use with `core::mem::offset_of!(HostVtable, some_entry)` before calling
    /// anything the module's ABI knows about but an older host may not.
    pub fn has(&self, byte_offset: usize) -> bool {
        self.size >= byte_offset + core::mem::size_of::<usize>()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The ABI crate must carry no mutable global state: it is linked into both
    /// the host and every extension, and a `static mut` would exist twice with
    /// each side seeing its own copy.
    #[test]
    fn no_mutable_statics_in_this_crate() {
        let src = include_str!("lib.rs");
        // Only the crate proper — the test module below necessarily talks about
        // the very thing it forbids, and the needle is assembled at run time so
        // this assertion cannot match itself.
        let code_only = src.split("#[cfg(test)]").next().unwrap_or(src);
        let needle = format!("{} {} ", "static", "mut");
        for (n, line) in code_only.lines().enumerate() {
            let code = line.split("//").next().unwrap_or("");
            assert!(
                !code.contains(&needle),
                "line {}: the ABI crate must have no mutable statics — it is linked into both \
                 the host and every extension, and each side would get its own copy",
                n + 1
            );
        }
    }

    /// The vtable only ever grows at the end: an extension built against an
    /// older table must find every entry it knows where it always was.
    #[test]
    fn tier_4_entries_were_appended_after_the_tier_3_table() {
        use core::mem::{offset_of, size_of};
        assert_eq!(
            offset_of!(HostVtable, error_take),
            offset_of!(HostVtable, include_template) + size_of::<usize>(),
            "a tier-4 entry was inserted before the end of the tier-3 table"
        );
        assert_eq!(
            offset_of!(HostVtable, invoke_method_named) + size_of::<usize>(),
            size_of::<HostVtable>()
        );
    }

    #[test]
    fn handles_are_two_u32s() {
        assert_eq!(core::mem::size_of::<ValueHandle>(), 8);
        assert!(ValueHandle::NULL.is_null());
        assert!(!ValueHandle { slot: 0, gen: 1 }.is_null());
    }

    #[test]
    fn target_triple_is_real() {
        assert_ne!(TARGET, "unknown");
        assert!(TARGET.contains('-'), "not a triple: {TARGET}");
    }
}
