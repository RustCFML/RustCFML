//! Host side of the native-extension ABI: the value slab, the service vtable a
//! module calls back through, and the `CfmlNative` bridge for module-provided
//! classes.
//!
//! Nothing here trusts the module. Every handle is range- and generation-
//! checked, every `ctx` is magic-checked, and every crossing into module code
//! is wrapped in `catch_unwind` so a panicking extension produces a CFML error
//! rather than aborting the process.
//!
//! # Why a slab
//!
//! A module never sees a `CfmlValue`. It sees a [`ValueHandle`] — an index into
//! a host-owned `Vec` plus a generation tag — so the module links neither
//! `cfml-common` nor the host allocator, and nothing crosses ownership. The
//! slab is drawn from a thread-local pool and truncated rather than freed, so a
//! steady-state call allocates nothing; the generation bump on release is what
//! turns "used a handle after its call returned" into a clean error instead of
//! reading a slot that now holds someone else's value.

use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::c_void;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, RwLock, Weak};

use cfml_module_abi as abi;
use cfml_module_abi::{Ctx, HostVtable, NativeClassVtable, StrRef, ValueHandle};

use cfml_common::dynamic::{CfmlArray, CfmlNative, CfmlQuery, CfmlStruct, CfmlValue, ValueMap};
use cfml_common::vm::{CfmlError, CfmlErrorType, CfmlResult};

/// The tier this host implements. Values only, for now; tier 2 appends scope
/// access to the vtable without breaking anything already published.
pub const HOST_TIER: u32 = abi::tier::EXECUTION;

const CTX_MAGIC: u64 = 0x5243464d_4c435458; // "RCFMLCTX"

// ---------------------------------------------------------------------------
// The per-call state
// ---------------------------------------------------------------------------

#[derive(Default)]
struct CallState {
    slab: Vec<CfmlValue>,
    /// Argument handles, kept beside the slab so the per-call `Vec` is reused
    /// rather than allocated and dropped on every crossing.
    handles: Vec<ValueHandle>,
    generation: u32,
    error: Option<CfmlError>,
}

/// The scopes a module may only WRITE while holding that scope's lock.
///
/// These are the live, cross-request ones. `variables` and `request` belong to
/// one request and need no lock; `cgi`/`url`/`form`/`cookie` are request inputs
/// and are read-only to an extension.
const SHARED_SCOPES: &[&str] = &["application", "session", "server"];

/// Scopes an extension may not write at all.
const READ_ONLY_SCOPES: &[&str] = &["cgi", "url", "form", "cookie"];

/// What `*mut Ctx` really points at. The module only ever holds the opaque
/// pointer; every vtable entry casts it back and checks the magic first, so a
/// stored-and-reused ctx is [`abi::status::BAD_CTX`] rather than a wild read.
struct HostCtx {
    magic: u64,
    generation: u32,
    state: *mut CallState,
    /// The object a native-class method was called on, for
    /// [`ValueHandle::SELF`]. `None` inside a plain BIF, which is why
    /// returning SELF from one is an error rather than a null.
    receiver: Option<CfmlValue>,
    /// Tier 2: the VM for exactly the duration of this call.
    ///
    /// Derived from the `&mut self` the dispatch site already holds, and never
    /// used by the host while the module call is in flight — the same discipline
    /// as the call slab's pointer. Null when there is no VM (`on_load`), which
    /// makes every scope entry return [`abi::status::NO_SCOPE`] rather than
    /// dereference nothing.
    vm: *mut crate::CfmlVirtualMachine,
    /// The calling frame's locals, so an unqualified read can honour CFML's
    /// resolution order from the frame that actually made the call.
    locals: *const ValueMap,
    /// `held_locks.len()` on entry. Anything the module pushed past this is
    /// force-released when the call returns: a lock held across requests is a
    /// hang, not a bug report.
    lock_floor: usize,
}

thread_local! {
    /// Reused slabs. Popped for the duration of a call and returned after, so
    /// nesting (a module class method invoked while another module call is on
    /// the stack) gets its own state rather than sharing one.
    static POOL: RefCell<Vec<CallState>> = const { RefCell::new(Vec::new()) };
}

/// Generations are process-global and monotonic, so a handle from one thread
/// can never be mistaken for a live slot on another.
static NEXT_GEN: AtomicU32 = AtomicU32::new(1);

thread_local! {
    /// The VM and calling-frame locals for the foreign call in flight.
    ///
    /// A side channel rather than a parameter because `CfmlNative::call_method`
    /// has no VM argument, and inventing one would churn the trait and every
    /// existing implementor for the sake of extensions. Set by [`VmScope`] at
    /// each dispatch site — where a `&mut CfmlVirtualMachine` is in hand — and
    /// restored on the way out, so nesting is correct and nothing survives the
    /// call.
    static CURRENT_VM: RefCell<(*mut crate::CfmlVirtualMachine, *const ValueMap)> =
        const { RefCell::new((std::ptr::null_mut(), std::ptr::null())) };
}

/// Publishes the VM to the foreign call about to run, and takes it away again.
///
/// The pointer is derived from a `&mut CfmlVirtualMachine` the dispatch site
/// already holds and does not touch while the module runs — the same discipline
/// the call slab's pointer uses.
pub struct VmScope {
    prev: (*mut crate::CfmlVirtualMachine, *const ValueMap),
}

impl VmScope {
    pub fn new(vm: &mut crate::CfmlVirtualMachine, locals: Option<&ValueMap>) -> VmScope {
        let next = (
            vm as *mut crate::CfmlVirtualMachine,
            locals.map_or(std::ptr::null(), |l| l as *const ValueMap),
        );
        let prev = CURRENT_VM.with(|c| {
            let mut cell = c.borrow_mut();
            let prev = *cell;
            *cell = next;
            prev
        });
        VmScope { prev }
    }
}

impl Drop for VmScope {
    fn drop(&mut self) {
        let prev = self.prev;
        CURRENT_VM.with(|c| *c.borrow_mut() = prev);
    }
}

fn current_vm() -> (*mut crate::CfmlVirtualMachine, *const ValueMap) {
    CURRENT_VM.with(|c| *c.borrow())
}

fn acquire() -> CallState {
    let mut state = POOL.with(|p| p.borrow_mut().pop()).unwrap_or_default();
    state.slab.clear();
    state.handles.clear();
    state.error = None;
    // Wrapping is fine: a collision needs 4 billion intervening calls AND a
    // handle held across all of them, which the borrow checker in the wrapper
    // crate already prevents.
    state.generation = NEXT_GEN.fetch_add(1, Ordering::Relaxed).max(1);
    state
}

fn release(mut state: CallState) {
    state.slab.clear();
    state.handles.clear();
    state.error = None;
    POOL.with(|p| {
        let mut pool = p.borrow_mut();
        if pool.len() < 8 {
            pool.push(state);
        }
    });
}

impl CallState {
    fn push(&mut self, v: CfmlValue) -> ValueHandle {
        let slot = self.slab.len() as u32;
        self.slab.push(v);
        ValueHandle { slot, gen: self.generation }
    }
}

// ---------------------------------------------------------------------------
// Handle access
// ---------------------------------------------------------------------------

/// # Safety
/// `raw` must be a ctx the host created for a call still in progress.
unsafe fn ctx_of<'a>(raw: *mut Ctx) -> Option<&'a mut HostCtx> {
    if raw.is_null() {
        return None;
    }
    let c = &mut *(raw as *mut HostCtx);
    if c.magic != CTX_MAGIC {
        return None;
    }
    Some(c)
}

unsafe fn state_of<'a>(raw: *mut Ctx) -> Option<&'a mut CallState> {
    let c = ctx_of(raw)?;
    if c.state.is_null() {
        return None;
    }
    let state = &mut *c.state;
    if state.generation != c.generation {
        return None;
    }
    Some(state)
}

/// Read a handle's value, cloning it out. Cloning a `CfmlValue` is an `Arc`
/// bump for every container type, so this is cheap for exactly the values where
/// it would otherwise be expensive.
unsafe fn value_of(raw: *mut Ctx, h: ValueHandle) -> Option<CfmlValue> {
    // `ctx.this()` is usable as a VALUE, not only as a return: passing it to
    // `invoke_method` is how a class method re-enters itself through the engine,
    // which is the whole point of tier 3. Resolved here so every entry point
    // accepts it without each one remembering to.
    if h.is_self() {
        return ctx_of(raw)?.receiver.clone();
    }
    let state = state_of(raw)?;
    if h.gen != state.generation {
        return None;
    }
    state.slab.get(h.slot as usize).cloned()
}

/// Borrow a handle's value in place, for the accessors that hand out a pointer
/// into it.
unsafe fn value_ref<'a>(raw: *mut Ctx, h: ValueHandle) -> Option<&'a CfmlValue> {
    if h.is_self() {
        return ctx_of(raw)?.receiver.as_ref();
    }
    let state = state_of(raw)?;
    if h.gen != state.generation {
        return None;
    }
    state.slab.get(h.slot as usize)
}

unsafe fn make(raw: *mut Ctx, v: CfmlValue) -> ValueHandle {
    match state_of(raw) {
        Some(state) => state.push(v),
        None => ValueHandle::NULL,
    }
}

/// Park a string in the slab and hand back a pointer into it.
///
/// Every borrowed `StrRef` the host returns must point at something the slab
/// keeps alive for the rest of the call — a pointer into a struct's key map or
/// a query's column list would be a read of shared state with no lock held.
/// The `Arc<String>` buffer does not move when the slab's `Vec` grows, so the
/// pointer stays valid even as the module keeps creating values.
unsafe fn intern(raw: *mut Ctx, s: String) -> Option<StrRef> {
    let state = state_of(raw)?;
    let arc = Arc::new(s);
    let r = StrRef { ptr: arc.as_ptr(), len: arc.len() };
    state.slab.push(CfmlValue::String(arc));
    Some(r)
}

// ---------------------------------------------------------------------------
// The vtable
// ---------------------------------------------------------------------------

macro_rules! out_or {
    ($e:expr, $code:expr) => {
        match $e {
            Some(v) => v,
            None => return $code,
        }
    };
}

unsafe extern "C" fn h_val_type(raw: *mut Ctx, h: ValueHandle) -> u32 {
    match value_ref(raw, h) {
        None => abi::ty::NULL,
        Some(v) => match v {
            CfmlValue::Null => abi::ty::NULL,
            CfmlValue::Bool(_) => abi::ty::BOOL,
            CfmlValue::Int(_) => abi::ty::INT,
            CfmlValue::Double(_) => abi::ty::DOUBLE,
            CfmlValue::TimeSpan(_) => abi::ty::TIMESPAN,
            CfmlValue::String(_) => abi::ty::STRING,
            CfmlValue::Array(_) | CfmlValue::QueryColumn(_, _) => abi::ty::ARRAY,
            CfmlValue::Struct(_) => abi::ty::STRUCT,
            CfmlValue::Binary(_) => abi::ty::BINARY,
            CfmlValue::Query(_) => abi::ty::QUERY,
            CfmlValue::Function(_) | CfmlValue::Closure(_) => abi::ty::FUNCTION,
            CfmlValue::Component(_) => abi::ty::COMPONENT,
            CfmlValue::NativeObject(_) => abi::ty::NATIVE,
            #[cfg(feature = "component-instance")]
            CfmlValue::Instance(_) => abi::ty::COMPONENT,
        },
    }
}

unsafe extern "C" fn h_val_bool(raw: *mut Ctx, h: ValueHandle, out: *mut bool) -> u32 {
    let v = out_or!(value_ref(raw, h), abi::status::BAD_HANDLE);
    match v {
        CfmlValue::Bool(b) => {
            *out = *b;
            abi::status::OK
        }
        _ => abi::status::WRONG_TYPE,
    }
}

unsafe extern "C" fn h_val_i64(raw: *mut Ctx, h: ValueHandle, out: *mut i64) -> u32 {
    let v = out_or!(value_ref(raw, h), abi::status::BAD_HANDLE);
    match v {
        CfmlValue::Int(i) => {
            *out = *i;
            abi::status::OK
        }
        CfmlValue::Double(d) | CfmlValue::TimeSpan(d) => {
            *out = *d as i64;
            abi::status::OK
        }
        CfmlValue::Bool(b) => {
            *out = i64::from(*b);
            abi::status::OK
        }
        // CFML is a string-typed language: "3" IS 3. Refusing to parse here
        // would make every module re-implement the coercion by hand.
        CfmlValue::String(s) => match s.trim().parse::<i64>() {
            Ok(n) => {
                *out = n;
                abi::status::OK
            }
            Err(_) => abi::status::WRONG_TYPE,
        },
        _ => abi::status::WRONG_TYPE,
    }
}

unsafe extern "C" fn h_val_f64(raw: *mut Ctx, h: ValueHandle, out: *mut f64) -> u32 {
    let v = out_or!(value_ref(raw, h), abi::status::BAD_HANDLE);
    match v {
        CfmlValue::Int(i) => {
            *out = *i as f64;
            abi::status::OK
        }
        CfmlValue::Double(d) | CfmlValue::TimeSpan(d) => {
            *out = *d;
            abi::status::OK
        }
        CfmlValue::Bool(b) => {
            *out = if *b { 1.0 } else { 0.0 };
            abi::status::OK
        }
        CfmlValue::String(s) => match s.trim().parse::<f64>() {
            Ok(n) => {
                *out = n;
                abi::status::OK
            }
            Err(_) => abi::status::WRONG_TYPE,
        },
        _ => abi::status::WRONG_TYPE,
    }
}

unsafe extern "C" fn h_val_str(raw: *mut Ctx, h: ValueHandle, out: *mut StrRef) -> u32 {
    let v = out_or!(value_ref(raw, h), abi::status::BAD_HANDLE);
    match v {
        CfmlValue::String(s) => {
            *out = StrRef { ptr: s.as_ptr(), len: s.len() };
            abi::status::OK
        }
        _ => abi::status::WRONG_TYPE,
    }
}

unsafe extern "C" fn h_val_bytes(raw: *mut Ctx, h: ValueHandle, out: *mut StrRef) -> u32 {
    let v = out_or!(value_ref(raw, h), abi::status::BAD_HANDLE);
    match v {
        CfmlValue::Binary(b) => {
            *out = StrRef { ptr: b.as_ptr(), len: b.len() };
            abi::status::OK
        }
        _ => abi::status::WRONG_TYPE,
    }
}

unsafe extern "C" fn h_val_to_string(raw: *mut Ctx, h: ValueHandle) -> ValueHandle {
    let v = out_or!(value_of(raw, h), ValueHandle::NULL);
    make(raw, CfmlValue::string(v.as_string()))
}

unsafe extern "C" fn h_val_is_true(raw: *mut Ctx, h: ValueHandle, out: *mut bool) -> u32 {
    let v = out_or!(value_ref(raw, h), abi::status::BAD_HANDLE);
    *out = v.is_true();
    abi::status::OK
}

unsafe extern "C" fn h_new_null(raw: *mut Ctx) -> ValueHandle {
    make(raw, CfmlValue::Null)
}
unsafe extern "C" fn h_new_bool(raw: *mut Ctx, v: bool) -> ValueHandle {
    make(raw, CfmlValue::Bool(v))
}
unsafe extern "C" fn h_new_int(raw: *mut Ctx, v: i64) -> ValueHandle {
    make(raw, CfmlValue::Int(v))
}
unsafe extern "C" fn h_new_double(raw: *mut Ctx, v: f64) -> ValueHandle {
    make(raw, CfmlValue::Double(v))
}
unsafe extern "C" fn h_new_timespan(raw: *mut Ctx, v: f64) -> ValueHandle {
    make(raw, CfmlValue::TimeSpan(v))
}

/// Copy the module's bytes in. A module's memory is not the host's to keep.
unsafe fn str_in(r: StrRef) -> String {
    if r.ptr.is_null() || r.len == 0 {
        return String::new();
    }
    String::from_utf8_lossy(std::slice::from_raw_parts(r.ptr, r.len)).into_owned()
}

unsafe extern "C" fn h_new_string(raw: *mut Ctx, s: StrRef) -> ValueHandle {
    make(raw, CfmlValue::string(str_in(s)))
}

unsafe extern "C" fn h_new_binary(raw: *mut Ctx, s: StrRef) -> ValueHandle {
    let bytes = if s.ptr.is_null() || s.len == 0 {
        Vec::new()
    } else {
        std::slice::from_raw_parts(s.ptr, s.len).to_vec()
    };
    make(raw, CfmlValue::Binary(bytes))
}

unsafe extern "C" fn h_arr_new(raw: *mut Ctx, cap: usize) -> ValueHandle {
    make(raw, CfmlValue::Array(CfmlArray::new(Vec::with_capacity(cap))))
}

unsafe extern "C" fn h_arr_len(raw: *mut Ctx, h: ValueHandle, out: *mut usize) -> u32 {
    let v = out_or!(value_ref(raw, h), abi::status::BAD_HANDLE);
    match v {
        CfmlValue::Array(a) => {
            *out = a.len();
            abi::status::OK
        }
        CfmlValue::QueryColumn(a, _) => {
            *out = a.len();
            abi::status::OK
        }
        _ => abi::status::WRONG_TYPE,
    }
}

unsafe extern "C" fn h_arr_get(raw: *mut Ctx, h: ValueHandle, i: usize) -> ValueHandle {
    let v = out_or!(value_of(raw, h), ValueHandle::NULL);
    let item = match &v {
        CfmlValue::Array(a) => a.get(i),
        CfmlValue::QueryColumn(a, _) => a.get(i).cloned(),
        _ => None,
    };
    make(raw, item.unwrap_or(CfmlValue::Null))
}

unsafe extern "C" fn h_arr_set(raw: *mut Ctx, h: ValueHandle, i: usize, v: ValueHandle) -> u32 {
    let target = out_or!(value_of(raw, h), abi::status::BAD_HANDLE);
    let value = out_or!(value_of(raw, v), abi::status::BAD_HANDLE);
    match target {
        CfmlValue::Array(a) => {
            a.set_or_grow(i, value);
            abi::status::OK
        }
        _ => abi::status::WRONG_TYPE,
    }
}

unsafe extern "C" fn h_arr_push(raw: *mut Ctx, h: ValueHandle, v: ValueHandle) -> u32 {
    let target = out_or!(value_of(raw, h), abi::status::BAD_HANDLE);
    let value = out_or!(value_of(raw, v), abi::status::BAD_HANDLE);
    match target {
        CfmlValue::Array(a) => {
            a.push(value);
            abi::status::OK
        }
        _ => abi::status::WRONG_TYPE,
    }
}

unsafe extern "C" fn h_struct_new(raw: *mut Ctx) -> ValueHandle {
    make(raw, CfmlValue::Struct(CfmlStruct::empty()))
}

unsafe extern "C" fn h_struct_len(raw: *mut Ctx, h: ValueHandle, out: *mut usize) -> u32 {
    let v = out_or!(value_ref(raw, h), abi::status::BAD_HANDLE);
    match v {
        CfmlValue::Struct(s) => {
            *out = s.len();
            abi::status::OK
        }
        _ => abi::status::WRONG_TYPE,
    }
}

unsafe extern "C" fn h_struct_get(raw: *mut Ctx, h: ValueHandle, k: StrRef) -> ValueHandle {
    let v = out_or!(value_of(raw, h), ValueHandle::NULL);
    let key = str_in(k);
    let found = match &v {
        CfmlValue::Struct(s) => s.get_ci(key.as_str()),
        _ => None,
    };
    make(raw, found.unwrap_or(CfmlValue::Null))
}

unsafe extern "C" fn h_struct_set(
    raw: *mut Ctx,
    h: ValueHandle,
    k: StrRef,
    v: ValueHandle,
) -> u32 {
    let target = out_or!(value_of(raw, h), abi::status::BAD_HANDLE);
    let value = out_or!(value_of(raw, v), abi::status::BAD_HANDLE);
    match target {
        CfmlValue::Struct(s) => {
            s.insert(str_in(k), value);
            abi::status::OK
        }
        _ => abi::status::WRONG_TYPE,
    }
}

unsafe extern "C" fn h_struct_has(
    raw: *mut Ctx,
    h: ValueHandle,
    k: StrRef,
    out: *mut bool,
) -> u32 {
    let v = out_or!(value_ref(raw, h), abi::status::BAD_HANDLE);
    match v {
        CfmlValue::Struct(s) => {
            *out = s.contains_key_ci(str_in(k).as_str());
            abi::status::OK
        }
        _ => abi::status::WRONG_TYPE,
    }
}

unsafe extern "C" fn h_struct_delete(raw: *mut Ctx, h: ValueHandle, k: StrRef) -> u32 {
    let v = out_or!(value_of(raw, h), abi::status::BAD_HANDLE);
    match v {
        CfmlValue::Struct(s) => {
            s.remove_ci(str_in(k).as_str());
            abi::status::OK
        }
        _ => abi::status::WRONG_TYPE,
    }
}

unsafe extern "C" fn h_struct_key_at(
    raw: *mut Ctx,
    h: ValueHandle,
    i: usize,
    out: *mut StrRef,
) -> u32 {
    // Clone the value out first: `intern` takes the slab mutably, and holding a
    // borrow of it across that call would alias.
    let v = out_or!(value_of(raw, h), abi::status::BAD_HANDLE);
    let CfmlValue::Struct(s) = &v else {
        return abi::status::WRONG_TYPE;
    };
    let keys = s.keys();
    let Some(k) = keys.get(i) else {
        return abi::status::NOT_FOUND;
    };
    let key = k.clone();
    drop(v);
    *out = out_or!(intern(raw, key), abi::status::BAD_CTX);
    abi::status::OK
}

// ---- queries ---------------------------------------------------------------

unsafe extern "C" fn h_query_new(raw: *mut Ctx, cols: ValueHandle) -> ValueHandle {
    let v = out_or!(value_of(raw, cols), ValueHandle::NULL);
    let names: Vec<String> = match &v {
        CfmlValue::Array(a) => a.snapshot().iter().map(|c| c.as_string()).collect(),
        _ => Vec::new(),
    };
    make(raw, CfmlValue::Query(CfmlQuery::new(names)))
}

unsafe extern "C" fn h_query_cols(raw: *mut Ctx, h: ValueHandle, out: *mut usize) -> u32 {
    let v = out_or!(value_ref(raw, h), abi::status::BAD_HANDLE);
    match v {
        CfmlValue::Query(q) => {
            *out = q.column_count();
            abi::status::OK
        }
        _ => abi::status::WRONG_TYPE,
    }
}

unsafe extern "C" fn h_query_col_name(
    raw: *mut Ctx,
    h: ValueHandle,
    i: usize,
    out: *mut StrRef,
) -> u32 {
    // Cloned out for the same reason as `h_struct_key_at`.
    let v = out_or!(value_of(raw, h), abi::status::BAD_HANDLE);
    let CfmlValue::Query(q) = &v else {
        return abi::status::WRONG_TYPE;
    };
    let cols = q.columns();
    let Some(name) = cols.get(i) else {
        return abi::status::NOT_FOUND;
    };
    let name = name.clone();
    drop(v);
    *out = out_or!(intern(raw, name), abi::status::BAD_CTX);
    abi::status::OK
}

unsafe extern "C" fn h_query_col_index(
    raw: *mut Ctx,
    h: ValueHandle,
    name: StrRef,
    out: *mut usize,
) -> u32 {
    let v = out_or!(value_ref(raw, h), abi::status::BAD_HANDLE);
    let CfmlValue::Query(q) = v else {
        return abi::status::WRONG_TYPE;
    };
    let wanted = str_in(name);
    match q.with_read(|d| d.column_index_ci(&wanted)) {
        Some(i) => {
            *out = i;
            abi::status::OK
        }
        None => abi::status::NOT_FOUND,
    }
}

unsafe extern "C" fn h_query_rows(raw: *mut Ctx, h: ValueHandle, out: *mut usize) -> u32 {
    let v = out_or!(value_ref(raw, h), abi::status::BAD_HANDLE);
    match v {
        CfmlValue::Query(q) => {
            *out = q.row_count();
            abi::status::OK
        }
        _ => abi::status::WRONG_TYPE,
    }
}

unsafe extern "C" fn h_query_cell(
    raw: *mut Ctx,
    h: ValueHandle,
    row: usize,
    col: usize,
) -> ValueHandle {
    let v = out_or!(value_of(raw, h), ValueHandle::NULL);
    let cell = match &v {
        CfmlValue::Query(q) => q.with_read(|d| d.cell(row, col).cloned()),
        _ => None,
    };
    make(raw, cell.unwrap_or(CfmlValue::Null))
}

unsafe extern "C" fn h_query_set_cell(
    raw: *mut Ctx,
    h: ValueHandle,
    row: usize,
    col: usize,
    val: ValueHandle,
) -> u32 {
    let target = out_or!(value_of(raw, h), abi::status::BAD_HANDLE);
    let value = out_or!(value_of(raw, val), abi::status::BAD_HANDLE);
    let CfmlValue::Query(q) = target else {
        return abi::status::WRONG_TYPE;
    };
    let ok = q.with_write(|d| match d.cell_mut(row, col) {
        Some(slot) => {
            *slot = value;
            true
        }
        None => false,
    });
    if ok {
        abi::status::OK
    } else {
        abi::status::NOT_FOUND
    }
}

unsafe extern "C" fn h_query_add_row(raw: *mut Ctx, h: ValueHandle, row: ValueHandle) -> u32 {
    let target = out_or!(value_of(raw, h), abi::status::BAD_HANDLE);
    let cells = out_or!(value_of(raw, row), abi::status::BAD_HANDLE);
    let CfmlValue::Query(q) = target else {
        return abi::status::WRONG_TYPE;
    };
    let vals: Vec<CfmlValue> = match &cells {
        CfmlValue::Array(a) => a.snapshot(),
        _ => return abi::status::WRONG_TYPE,
    };
    q.with_write(|d| d.push_row_positional(vals));
    abi::status::OK
}

unsafe extern "C" fn h_query_col_values(
    raw: *mut Ctx,
    h: ValueHandle,
    col: usize,
) -> ValueHandle {
    let v = out_or!(value_of(raw, h), ValueHandle::NULL);
    let values = match &v {
        CfmlValue::Query(q) => q.with_read(|d| d.column_data(col).cloned()),
        _ => None,
    };
    make(raw, CfmlValue::Array(CfmlArray::new(values.unwrap_or_default())))
}

// ---- components and natives -------------------------------------------------

unsafe extern "C" fn h_component_name(raw: *mut Ctx, h: ValueHandle, out: *mut StrRef) -> u32 {
    let v = out_or!(value_of(raw, h), abi::status::BAD_HANDLE);
    let name = match &v {
        CfmlValue::Component(c) => c.name.clone(),
        CfmlValue::Struct(s) => match s.get_ci("__component_path") {
            Some(p) => p.as_string(),
            None => return abi::status::WRONG_TYPE,
        },
        _ => return abi::status::WRONG_TYPE,
    };
    *out = out_or!(intern(raw, name), abi::status::BAD_CTX);
    abi::status::OK
}

unsafe extern "C" fn h_native_class_name(raw: *mut Ctx, h: ValueHandle, out: *mut StrRef) -> u32 {
    let v = out_or!(value_of(raw, h), abi::status::BAD_HANDLE);
    let CfmlValue::NativeObject(o) = &v else {
        return abi::status::WRONG_TYPE;
    };
    let name = match o.read() {
        Ok(g) => g.class_name().to_string(),
        Err(_) => return abi::status::WRONG_TYPE,
    };
    *out = out_or!(intern(raw, name), abi::status::BAD_CTX);
    abi::status::OK
}

unsafe extern "C" fn h_new_native(
    raw: *mut Ctx,
    data: *mut c_void,
    vtable: *const NativeClassVtable,
) -> ValueHandle {
    if data.is_null() || vtable.is_null() {
        return ValueHandle::NULL;
    }
    make(raw, ForeignNative::into_value(data, vtable))
}

unsafe extern "C" fn h_throw(
    raw: *mut Ctx,
    error_type: u32,
    custom_type: StrRef,
    message: StrRef,
    extras: ValueHandle,
) {
    let message = str_in(message);
    let custom = str_in(custom_type);
    let extras_val = value_of(raw, extras);
    let Some(state) = state_of(raw) else { return };
    let kind = match error_type {
        abi::error_type::EXPRESSION => CfmlErrorType::Expression,
        // The engine has no dedicated Database/Security/IO variants; these are
        // the custom type names CFML code actually catches on.
        abi::error_type::DATABASE => CfmlErrorType::Custom("database".to_string()),
        abi::error_type::SECURITY => CfmlErrorType::Custom("Security".to_string()),
        abi::error_type::IO => CfmlErrorType::Custom("java.io.IOException".to_string()),
        abi::error_type::CUSTOM => CfmlErrorType::Custom(if custom.is_empty() {
            "extension".to_string()
        } else {
            custom
        }),
        _ => CfmlErrorType::Application,
    };
    // FIRST error wins. A tier-3 entry point that fails stages the engine's own
    // error, and the wrapper then returns `Err` from the module — whose `throw`
    // would otherwise land here and replace a precise CFML error ("no such
    // component", a database error, a lock timeout) with the module's generic
    // "[x] failed". The inner cause is the one worth reporting.
    if state.error.is_some() {
        return;
    }
    let mut err = CfmlError::new(message, kind);
    if let Some(CfmlValue::Struct(s)) = extras_val {
        err.extras = Some(Box::new(s.snapshot()));
    }
    state.error = Some(err);
}

/// The table handed to every extension at load.
pub static HOST_VTABLE: HostVtable = HostVtable {
    size: std::mem::size_of::<HostVtable>(),
    abi_major: abi::ABI_MAJOR,
    tier: HOST_TIER,
    val_type: h_val_type,
    val_bool: h_val_bool,
    val_i64: h_val_i64,
    val_f64: h_val_f64,
    val_str: h_val_str,
    val_bytes: h_val_bytes,
    val_to_string: h_val_to_string,
    val_is_true: h_val_is_true,
    new_null: h_new_null,
    new_bool: h_new_bool,
    new_int: h_new_int,
    new_double: h_new_double,
    new_timespan: h_new_timespan,
    new_string: h_new_string,
    new_binary: h_new_binary,
    arr_new: h_arr_new,
    arr_len: h_arr_len,
    arr_get: h_arr_get,
    arr_set: h_arr_set,
    arr_push: h_arr_push,
    struct_new: h_struct_new,
    struct_len: h_struct_len,
    struct_get: h_struct_get,
    struct_set: h_struct_set,
    struct_has: h_struct_has,
    struct_delete: h_struct_delete,
    struct_key_at: h_struct_key_at,
    query_new: h_query_new,
    query_cols: h_query_cols,
    query_col_name: h_query_col_name,
    query_col_index: h_query_col_index,
    query_rows: h_query_rows,
    query_cell: h_query_cell,
    query_set_cell: h_query_set_cell,
    query_add_row: h_query_add_row,
    query_col_values: h_query_col_values,
    component_name: h_component_name,
    native_class_name: h_native_class_name,
    new_native: h_new_native,
    throw: h_throw,
    // ---- tier 2 ----
    scope_get: h_scope_get,
    scope_set: h_scope_set,
    scope_has: h_scope_has,
    scope_delete: h_scope_delete,
    scope_snapshot: h_scope_snapshot,
    var_get: h_var_get,
    lock: h_lock,
    unlock: h_unlock,
    root: h_root,
    unroot: h_unroot,
    root_get: h_root_get,
    // ---- tier 3 ----
    call_fn: h_call_fn,
    call_value: h_call_value,
    new_component: h_new_component,
    invoke_method: h_invoke_method,
    component_set: h_component_set,
    component_metadata: h_component_metadata,
    write_output: h_write_output,
    include_template: h_include_template,
};

// ---------------------------------------------------------------------------
// Calling into a module
// ---------------------------------------------------------------------------

/// Run one module entry point with a fresh call state.
///
/// `receiver` is the object a native-class method was called on, and is what
/// [`ValueHandle::SELF`] resolves to — the mechanism behind a fluent mutator
/// returning the same object rather than a copy of it.
fn with_call<F>(
    what: &dyn Fn() -> String,
    receiver: Option<CfmlValue>,
    args: Vec<CfmlValue>,
    f: F,
) -> CfmlResult
where
    F: FnOnce(*mut Ctx, &[ValueHandle]) -> ValueHandle + std::panic::UnwindSafe,
{
    let mut state = acquire();
    for v in args {
        let h = state.push(v);
        state.handles.push(h);
    }
    // Handed to the module as a slice; the Vec itself lives on in the pool.
    let handles = std::mem::take(&mut state.handles);
    let (vm, locals) = current_vm();
    // Where the lock stack stood on entry: anything the module pushes above
    // this is force-released below.
    let lock_floor = if vm.is_null() { 0 } else { unsafe { (*vm).held_lock_depth() } };
    let mut ctx = HostCtx {
        magic: CTX_MAGIC,
        generation: state.generation,
        state: &mut state as *mut CallState,
        receiver: receiver.clone(),
        vm,
        locals,
        lock_floor,
    };
    let raw = &mut ctx as *mut HostCtx as *mut Ctx;

    // A panic inside the extension unwinds across `C-unwind` into here and
    // becomes a CFML error. Without this an extension bug takes the process
    // down, which is not an acceptable failure mode for a plugin.
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f(raw, &handles)));

    // Drop the ctx before touching `state` again: it holds a raw pointer to it.
    drop(ctx);

    // Guards are call-scoped. A module that acquires and forgets must not hold a
    // lock into the next request.
    if !vm.is_null() {
        let leaked = unsafe { (*vm).release_locks_above(lock_floor) };
        if leaked > 0 {
            log::warn!(
                "native extension [{}] returned holding {} lock(s); released",
                what(),
                leaked
            );
        }
    }

    // Put the buffer back before the result is read out.
    state.handles = handles;
    let result = match outcome {
        Err(payload) => {
            let detail = payload
                .downcast_ref::<&str>()
                .map(|s| s.to_string())
                .or_else(|| payload.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "unknown panic".to_string());
            Err(CfmlError::runtime(format!(
                "native extension [{}] panicked: {}",
                what(),
                detail
            )))
        }
        Ok(h) => {
            if let Some(err) = state.error.take() {
                Err(err)
            } else if h.is_self() {
                match receiver {
                    Some(v) => Ok(v),
                    None => Err(CfmlError::runtime(format!(
                        "native extension [{}] returned the fluent self-handle from a function \
                         with no receiver — ctx.this() is only valid inside a class method",
                        what()
                    ))),
                }
            } else if h.is_null() {
                Ok(CfmlValue::Null)
            } else if h.gen != state.generation {
                Err(CfmlError::runtime(format!(
                    "native extension [{}] returned a stale value handle",
                    what()
                )))
            } else {
                Ok(state.slab.get(h.slot as usize).cloned().unwrap_or(CfmlValue::Null))
            }
        }
    };
    release(state);
    result
}

/// A BIF provided by a loaded extension.
///
/// The entry point cannot live in `builtins` — that map holds bare
/// `fn(Vec<CfmlValue>) -> CfmlResult` pointers, which cannot carry "which
/// module, which entry point", and cannot be handed a `ctx`.
#[derive(Clone, Copy)]
pub struct ForeignBuiltin {
    pub entry: abi::ModuleFn,
    /// Leaked at load. Extensions are never unloaded (§6.2), so process
    /// lifetime is the honest lifetime — and it makes this type `Copy`, so the
    /// per-call lookup in `call_function` costs no atomics at all rather than
    /// two `Arc` bumps.
    pub module: &'static str,
    pub name: &'static str,
}

// The entry point is module-static code; the Arcs are ordinary shared strings.
unsafe impl Send for ForeignBuiltin {}
unsafe impl Sync for ForeignBuiltin {}

impl ForeignBuiltin {
    pub fn call(&self, args: Vec<CfmlValue>) -> CfmlResult {
        let entry = self.entry;
        // The label is built only on the error paths. Formatting it eagerly
        // cost a heap allocation on every successful call, which on a BIF this
        // small is a measurable fraction of the whole crossing.
        let label = || format!("{}:{}", self.module, self.name);
        with_call(&label, None, args, move |raw, handles| unsafe {
            entry(raw, handles.as_ptr(), handles.len())
        })
    }
}

/// A class provided by a loaded extension: what the host needs to construct one.
#[derive(Clone, Copy)]
pub struct ForeignClass {
    pub ctor: abi::ClassCtorFn,
    pub vtable: *const NativeClassVtable,
}

unsafe impl Send for ForeignClass {}
unsafe impl Sync for ForeignClass {}

impl ForeignClass {
    /// `createObject("rust", "Name", …)` for a module-provided class.
    pub fn construct(&self, args: Vec<CfmlValue>) -> CfmlResult {
        let ctor = self.ctor;
        let vtable = self.vtable;
        with_call(&|| "class constructor".to_string(), None, args, move |raw, handles| unsafe {
            let mut data: *mut c_void = std::ptr::null_mut();
            let code = ctor(raw, handles.as_ptr(), handles.len(), &mut data);
            if code != abi::status::OK || data.is_null() {
                // The module already called throw(); returning NULL lets that
                // error surface rather than being masked by a second one.
                return ValueHandle::NULL;
            }
            make(raw, ForeignNative::into_value(data, vtable))
        })
        .and_then(|v| match v {
            CfmlValue::Null => Err(CfmlError::runtime(
                "native extension class constructor failed".to_string(),
            )),
            other => Ok(other),
        })
    }
}

// ---------------------------------------------------------------------------
// ForeignNative — a module's class, seen by the engine as an ordinary native
// ---------------------------------------------------------------------------

/// Host-side `CfmlNative` over a module-supplied method table (§4.6).
///
/// Because the engine only ever sees a `CfmlNative`, `component
/// extends="rust:Name"`, `super.method()` and `this.X` fall-through all keep
/// working against an extension class with no further plumbing.
pub struct ForeignNative {
    data: *mut c_void,
    vtable: *const NativeClassVtable,
    /// Set at construction so a fluent mutator can hand back the *same* object.
    /// The module has no handle to itself; the host does.
    self_ref: Option<Weak<RwLock<ForeignNative>>>,
}

// Asserted by contract (§4.6): the module is responsible for its own interior
// synchronisation, which is also what will let tier 3 dispatch these without an
// exclusive lock.
unsafe impl Send for ForeignNative {}
unsafe impl Sync for ForeignNative {}

impl Drop for ForeignNative {
    fn drop(&mut self) {
        if !self.data.is_null() {
            unsafe { ((*self.vtable).drop_fn)(self.data) };
            self.data = std::ptr::null_mut();
        }
    }
}

impl std::fmt::Debug for ForeignNative {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ForeignNative").field("class", &self.class_name()).finish()
    }
}

impl ForeignNative {
    /// Wrap a module instance into a shared handle, stamping the weak
    /// self-reference on the way (mirrors how the built-in `Spreadsheet` does
    /// its fluent chaining).
    pub fn into_value(data: *mut c_void, vtable: *const NativeClassVtable) -> CfmlValue {
        let arc: Arc<RwLock<ForeignNative>> = Arc::new_cyclic(|weak| {
            RwLock::new(ForeignNative { data, vtable, self_ref: Some(weak.clone()) })
        });
        CfmlValue::NativeObject(arc)
    }

    /// Forward a method call to the module, with no engine lock held.
    fn dispatch(&self, name: &str, args: Vec<CfmlValue>) -> CfmlResult {
        let vtable = self.vtable;
        let data = self.data;
        let class = self.class_name();
        let label = || format!("{}.{}", class, name);
        let method = name.to_string();
        with_call(&label, self.this(), args, move |raw, handles| unsafe {
            ((*vtable).call_method)(
                raw,
                data,
                StrRef::new(&method),
                handles.as_ptr(),
                handles.len(),
            )
        })
    }

    fn this(&self) -> Option<CfmlValue> {
        self.self_ref
            .as_ref()
            .and_then(|w| w.upgrade())
            .map(|arc| CfmlValue::NativeObject(arc as Arc<RwLock<dyn CfmlNative>>))
    }
}

impl CfmlNative for ForeignNative {
    fn class_name(&self) -> &str {
        // The module returns a pointer into its own static string data, which
        // lives as long as the process — extensions are never unloaded.
        unsafe {
            let r = ((*self.vtable).class_name)(self.data);
            if r.ptr.is_null() {
                "RustExtension"
            } else {
                r.as_str()
            }
        }
    }

    /// An extension manages its own synchronisation by contract (§4.6), and the
    /// wrapper's `&self` method signature enforces it — so dispatch need not
    /// hold the exclusive lock, which is what lets an extension method call back
    /// into CFML that re-enters this same object.
    fn needs_exclusive(&self) -> bool {
        false
    }

    fn call_method_shared(&self, name: &str, args: Vec<CfmlValue>) -> CfmlResult {
        self.dispatch(name, args)
    }

    fn call_method(&mut self, name: &str, args: Vec<CfmlValue>) -> CfmlResult {
        // Reached only through a path that took the exclusive lock anyway
        // (`cfinvoke`, a proxy SAM invoke). Forwards to the same place.
        self.dispatch(name, args)
    }

    fn method_params(&self, method: &str) -> Option<&'static [&'static str]> {
        let mut out = StrRef::EMPTY;
        let code =
            unsafe { ((*self.vtable).method_params)(self.data, StrRef::new(method), &mut out) };
        if code != abi::status::OK || out.ptr.is_null() {
            return None;
        }
        let list = unsafe { out.as_str() };
        Some(intern_params(self.class_name(), method, list))
    }

    fn get_property(&self, name: &str) -> Option<CfmlValue> {
        let vtable = self.vtable;
        let data = self.data;
        let prop = name.to_string();
        // An `&mut bool` cannot cross the `catch_unwind` boundary; an atomic can.
        let declined = std::sync::atomic::AtomicBool::new(false);
        let class = self.class_name();
        let result = with_call(
            &|| format!("{}.{}", class, name),
            None,
            Vec::new(),
            |raw, _| unsafe {
                let mut out = ValueHandle::NULL;
                let code = ((*vtable).get_property)(raw, data, StrRef::new(&prop), &mut out);
                if code != abi::status::OK {
                    declined.store(true, Ordering::Relaxed);
                    return ValueHandle::NULL;
                }
                out
            },
        );
        if declined.load(Ordering::Relaxed) {
            return None;
        }
        result.ok()
    }

    fn set_property(&mut self, name: &str, value: CfmlValue) -> Option<Result<(), CfmlError>> {
        let vtable = self.vtable;
        let data = self.data;
        let prop = name.to_string();
        let declined = std::sync::atomic::AtomicBool::new(false);
        let class = self.class_name();
        let result = with_call(
            &|| format!("{}.{}", class, name),
            None,
            vec![value],
            |raw, handles| unsafe {
                let code = ((*vtable).set_property)(
                    raw,
                    data,
                    StrRef::new(&prop),
                    handles.first().copied().unwrap_or(ValueHandle::NULL),
                );
                if code == abi::status::NOT_FOUND {
                    declined.store(true, Ordering::Relaxed);
                }
                ValueHandle::NULL
            },
        );
        if declined.load(Ordering::Relaxed) {
            return None;
        }
        Some(result.map(|_| ()))
    }
}

/// Turn a module's comma-separated parameter list into the `&'static` slice
/// `CfmlNative::method_params` wants.
///
/// Interned once per (class, method) and leaked. Extensions are never unloaded,
/// so process lifetime is the honest lifetime here — and the alternative,
/// widening the trait's return type, would churn every existing implementor for
/// the benefit of a path that is called once per named call site.
fn intern_params(class: &str, method: &str, list: &str) -> &'static [&'static str] {
    static CACHE: Mutex<Option<HashMap<(String, String), &'static [&'static str]>>> =
        Mutex::new(None);
    let key = (class.to_string(), method.to_ascii_lowercase());
    let mut guard = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    let map = guard.get_or_insert_with(HashMap::new);
    if let Some(hit) = map.get(&key) {
        return hit;
    }
    let names: Vec<&'static str> = list
        .split(',')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| &*Box::leak(s.to_string().into_boxed_str()))
        .collect();
    let leaked: &'static [&'static str] = Box::leak(names.into_boxed_slice());
    map.insert(key, leaked);
    leaked
}

// ---------------------------------------------------------------------------
// A loaded module, ready to register into a VM
// ---------------------------------------------------------------------------

/// Everything a loaded extension contributes, extracted from its `ModuleDecl`
/// once at load so the per-VM `register` does nothing but insert into maps.
pub struct LoadedModule {
    pub name: Arc<str>,
    pub version: String,
    pub bifs: Vec<ForeignBuiltin>,
    pub classes: Vec<(String, ForeignClass)>,
    /// SQL functions usable inside `queryExecute(…, {dbtype:"query"})`.
    /// `true` in the second slot means aggregate.
    pub qoq_fns: Vec<(String, ForeignBuiltin, bool)>,
    /// A directory of CFML the extension ships, mounted as `/<name>/`. Set by
    /// the loader after extraction; the ABI knows nothing about it.
    pub cfml_dir: Option<std::path::PathBuf>,
}

// The pointers inside are module-static.
unsafe impl Send for LoadedModule {}
unsafe impl Sync for LoadedModule {}

/// Intern a name for the process. Extensions are never unloaded, so this is
/// the correct lifetime rather than a leak in the pejorative sense.
fn leak(s: &str) -> &'static str {
    static NAMES: Mutex<Option<HashMap<String, &'static str>>> = Mutex::new(None);
    let mut guard = NAMES.lock().unwrap_or_else(|e| e.into_inner());
    let map = guard.get_or_insert_with(HashMap::new);
    if let Some(hit) = map.get(s) {
        return hit;
    }
    let leaked: &'static str = Box::leak(s.to_string().into_boxed_str());
    map.insert(s.to_string(), leaked);
    leaked
}

/// Read a `ModuleDecl` and check it against this host.
///
/// `config` is the extension's `.cfconfig.json` settings block, handed to its
/// `on_load` as an ordinary CFML struct.
///
/// # Safety
/// `decl` must point at a live `ModuleDecl` produced by `rustcfml_module_decl`.
pub unsafe fn adopt(
    decl: *const abi::ModuleDecl,
    source: &str,
    config: CfmlValue,
) -> Result<LoadedModule, String> {
    if decl.is_null() {
        return Err(format!("{}: rustcfml_module_decl() returned null", source));
    }
    let d = &*decl;
    if d.abi_major != abi::ABI_MAJOR {
        return Err(format!(
            "{}: built for extension ABI {} but this engine speaks {}",
            source,
            d.abi_major,
            abi::ABI_MAJOR
        ));
    }
    let target = d.target.as_str();
    if target != abi::TARGET {
        return Err(format!(
            "{}: built for target [{}] but this engine is [{}]",
            source, target, abi::TARGET
        ));
    }
    if d.tier > HOST_TIER {
        return Err(format!(
            "{}: requires extension capability tier {} but this engine implements tier {}",
            source, d.tier, HOST_TIER
        ));
    }

    let name_str = d.name.as_str().to_string();
    let name: Arc<str> = Arc::from(name_str.as_str());
    let mut bifs = Vec::with_capacity(d.bif_count);
    if !d.bifs.is_null() {
        for i in 0..d.bif_count {
            let b = &*d.bifs.add(i);
            bifs.push(ForeignBuiltin {
                entry: b.entry,
                module: leak(&name_str),
                name: leak(b.name.as_str()),
            });
        }
    }
    let mut classes = Vec::with_capacity(d.class_count);
    if !d.classes.is_null() {
        for i in 0..d.class_count {
            let c = &*d.classes.add(i);
            classes.push((
                c.name.as_str().to_string(),
                ForeignClass { ctor: c.ctor, vtable: c.vtable },
            ));
        }
    }

    let mut qoq_fns = Vec::with_capacity(d.qoq_fn_count);
    if !d.qoq_fns.is_null() {
        for i in 0..d.qoq_fn_count {
            let q = &*d.qoq_fns.add(i);
            let name = q.name.as_str().to_string();
            qoq_fns.push((
                name.clone(),
                ForeignBuiltin {
                    entry: q.entry,
                    module: leak(&name_str),
                    name: leak(&name),
                },
                q.kind == 1,
            ));
        }
    }

    // on_load: once per process. Given a real ctx so config can be delivered as
    // an ordinary struct handle rather than a second marshalling scheme.
    if let Some(on_load) = d.on_load {
        let refused = std::sync::atomic::AtomicBool::new(false);
        let r = with_call(&|| "on_load".to_string(), None, vec![config], |raw, handles| {
            let code = unsafe {
                on_load(
                    &HOST_VTABLE as *const HostVtable,
                    raw,
                    handles.first().copied().unwrap_or(ValueHandle::NULL),
                )
            };
            if code != abi::status::OK {
                refused.store(true, Ordering::Relaxed);
            }
            ValueHandle::NULL
        });
        // A module may fail its load either by throwing (which surfaces as an
        // Err here) or by returning non-zero without one.
        if let Err(e) = r {
            return Err(format!("{}: on_load failed: {}", source, e.message));
        }
        if refused.load(Ordering::Relaxed) {
            return Err(format!("{}: on_load refused to initialise the extension", source));
        }
    }

    Ok(LoadedModule {
        name,
        version: d.version.as_str().to_string(),
        bifs,
        classes,
        qoq_fns,
        cfml_dir: None,
    })
}

/// A struct built from a `ValueMap`, for `throw`'s `extras`.
#[allow(dead_code)]
fn _unused(_: ValueMap) {}

// ---------------------------------------------------------------------------
// Tier 2 — the scope facade
// ---------------------------------------------------------------------------

/// The VM behind a ctx, or `None` in a context that has none (`on_load`).
///
/// # Safety
/// The returned reference is valid only while the module call that owns `raw`
/// is in flight — which is exactly when the module can call this.
unsafe fn vm_of<'a>(raw: *mut Ctx) -> Option<&'a mut crate::CfmlVirtualMachine> {
    let c = ctx_of(raw)?;
    if c.vm.is_null() {
        return None;
    }
    Some(&mut *c.vm)
}

unsafe fn locals_of<'a>(raw: *mut Ctx) -> &'a ValueMap {
    // A leaked empty map, so callers always have something to borrow. One
    // allocation for the process, and it is never mutated.
    static EMPTY: std::sync::OnceLock<ValueMap> = std::sync::OnceLock::new();
    match ctx_of(raw) {
        Some(c) if !c.locals.is_null() => &*c.locals,
        _ => EMPTY.get_or_init(ValueMap::default),
    }
}

fn normalise_scope(name: &str) -> String {
    name.trim().to_ascii_lowercase()
}

/// The scope's container, as a struct we can read and write through.
///
/// `session` is deliberately absent: it is indirected through
/// `set_session_variable` / `get_session_scope` because a session may be backed
/// by an external store, so it is handled separately at each call site.
unsafe fn scope_struct(
    vm: &mut crate::CfmlVirtualMachine,
    scope: &str,
) -> Option<CfmlStruct> {
    match scope {
        "request" => Some(vm.request_scope.clone()),
        "application" => vm.application_scope.clone(),
        "server" => Some(vm.live_server_scope()),
        "variables" => None, // flat `globals`, handled at the call site
        _ => None,
    }
}

/// Take the scope's read lock for the duration of `f`.
///
/// A read of a shared scope has to be consistent with an in-flight exclusive
/// CFML `<cflock>`, and the author should not have to think about it — so every
/// read takes the lock, and a read of a per-request scope takes nothing because
/// nothing else can be writing it.
unsafe fn with_scope_read<R>(
    vm: &mut crate::CfmlVirtualMachine,
    scope: &str,
    f: impl FnOnce(&mut crate::CfmlVirtualMachine) -> R,
) -> R {
    if !SHARED_SCOPES.contains(&scope) {
        return f(vm);
    }
    let key = vm.scope_lock_key_for(scope);
    // Already held by this call (or by an enclosing <cflock>)? Reentrant, so do
    // not try to re-acquire — the underlying RwLock is not reentrant.
    if vm.holds_lock(&key) {
        return f(vm);
    }
    let opts = crate::LockOpts::for_scope(key, scope, false);
    // A read lock that cannot be had is not worth failing the read over: fall
    // through after the same timeout <cflock> would use. Correctness-wise this
    // is the pre-tier-2 behaviour, and it cannot deadlock a reader.
    let acquired = vm.acquire_named_lock(&opts).unwrap_or(false);
    let out = f(vm);
    if acquired {
        vm.release_top_lock();
    }
    out
}

unsafe extern "C" fn h_scope_get(raw: *mut Ctx, scope: StrRef, key: StrRef) -> ValueHandle {
    let scope_name = normalise_scope(&str_in(scope));
    let k = str_in(key);
    let found = {
        let Some(vm) = vm_of(raw) else { return ValueHandle::NULL };
        with_scope_read(vm, &scope_name, |vm| match scope_name.as_str() {
            "session" => match vm.get_session_scope() {
                CfmlValue::Struct(s) => s.get_ci(k.as_str()),
                _ => None,
            },
            "variables" => vm.globals.get(k.as_str()).cloned().or_else(|| {
                vm.globals
                    .iter()
                    .find(|(gk, _)| gk.eq_ignore_ascii_case(&k))
                    .map(|(_, v)| v.clone())
            }),
            other => scope_struct(vm, other).and_then(|s| s.get_ci(k.as_str())),
        })
    };
    make(raw, found.unwrap_or(CfmlValue::Null))
}

unsafe extern "C" fn h_scope_has(
    raw: *mut Ctx,
    scope: StrRef,
    key: StrRef,
    out: *mut bool,
) -> u32 {
    let h = h_scope_get(raw, scope, key);
    *out = !matches!(value_ref(raw, h), None | Some(CfmlValue::Null));
    abi::status::OK
}

/// Is a write to `scope` allowed right now?
unsafe fn writable(raw: *mut Ctx, scope: &str) -> u32 {
    if READ_ONLY_SCOPES.contains(&scope) {
        return abi::status::NO_SCOPE;
    }
    if !SHARED_SCOPES.contains(&scope) {
        return abi::status::OK;
    }
    let Some(vm) = vm_of(raw) else { return abi::status::NO_SCOPE };
    let key = vm.scope_lock_key_for(scope);
    if vm.holds_lock(&key) {
        abi::status::OK
    } else {
        abi::status::UNLOCKED
    }
}

unsafe extern "C" fn h_scope_set(
    raw: *mut Ctx,
    scope: StrRef,
    key: StrRef,
    value: ValueHandle,
) -> u32 {
    let scope_name = normalise_scope(&str_in(scope));
    let code = writable(raw, &scope_name);
    if code != abi::status::OK {
        return code;
    }
    let k = str_in(key);
    let v = out_or!(value_of(raw, value), abi::status::BAD_HANDLE);
    let Some(vm) = vm_of(raw) else { return abi::status::NO_SCOPE };
    match scope_name.as_str() {
        "session" => match vm.set_session_variable(&k, v) {
            Ok(_) => abi::status::OK,
            Err(_) => abi::status::NO_SCOPE,
        },
        "variables" => {
            vm.globals.insert(k, v);
            abi::status::OK
        }
        other => match scope_struct(vm, other) {
            Some(s) => {
                s.insert(k, v);
                abi::status::OK
            }
            None => abi::status::NO_SCOPE,
        },
    }
}

unsafe extern "C" fn h_scope_delete(raw: *mut Ctx, scope: StrRef, key: StrRef) -> u32 {
    let scope_name = normalise_scope(&str_in(scope));
    let code = writable(raw, &scope_name);
    if code != abi::status::OK {
        return code;
    }
    let k = str_in(key);
    let Some(vm) = vm_of(raw) else { return abi::status::NO_SCOPE };
    match scope_name.as_str() {
        "variables" => {
            vm.globals.shift_remove(k.as_str());
            abi::status::OK
        }
        other => match scope_struct(vm, other) {
            Some(s) => {
                s.remove_ci(&k);
                abi::status::OK
            }
            None => abi::status::NO_SCOPE,
        },
    }
}

unsafe extern "C" fn h_scope_snapshot(raw: *mut Ctx, scope: StrRef) -> ValueHandle {
    let scope_name = normalise_scope(&str_in(scope));
    let snap = {
        let Some(vm) = vm_of(raw) else { return ValueHandle::NULL };
        with_scope_read(vm, &scope_name, |vm| match scope_name.as_str() {
            "session" => match vm.get_session_scope() {
                CfmlValue::Struct(s) => Some(s.snapshot()),
                _ => None,
            },
            "variables" => Some(vm.globals.clone()),
            other => scope_struct(vm, other).map(|s| s.snapshot()),
        })
    };
    match snap {
        // A COPY, not the live store: walking a live shared scope key by key
        // while another request writes it is exactly the race this avoids.
        Some(map) => make(raw, CfmlValue::strukt(map)),
        None => ValueHandle::NULL,
    }
}

unsafe extern "C" fn h_var_get(raw: *mut Ctx, key: StrRef) -> ValueHandle {
    let k = str_in(key);
    let locals = locals_of(raw);
    let found = {
        let Some(vm) = vm_of(raw) else { return ValueHandle::NULL };
        // The engine's own resolver, so an unqualified read from an extension
        // and from CFML answer identically by construction.
        vm.resolve_path_root_public(&k, locals)
    };
    make(raw, found.unwrap_or(CfmlValue::Null))
}

// ---- locks -----------------------------------------------------------------

unsafe extern "C" fn h_lock(
    raw: *mut Ctx,
    scope: StrRef,
    name: StrRef,
    exclusive: bool,
    timeout_ms: u64,
    out: *mut u64,
) -> u32 {
    let scope_name = normalise_scope(&str_in(scope));
    let lock_name = str_in(name);
    let Some(vm) = vm_of(raw) else { return abi::status::NO_SCOPE };
    let (key, label) = if !lock_name.is_empty() {
        (lock_name.clone(), format!("lock with name [{}]", lock_name))
    } else if !scope_name.is_empty() {
        (
            vm.scope_lock_key_for(&scope_name),
            format!("[{}] scope lock", scope_name),
        )
    } else {
        return abi::status::NO_SCOPE;
    };
    let mut opts = crate::LockOpts::for_scope(key, &label, exclusive);
    opts.timeout_ms = timeout_ms;
    match vm.acquire_named_lock(&opts) {
        Ok(true) => {
            // The token is the held_locks depth, so release is exact even when
            // several locks are taken in one call.
            *out = vm.held_lock_depth() as u64;
            abi::status::OK
        }
        Ok(false) => abi::status::TIMEOUT,
        Err(e) => {
            // Surface the engine's own `lock`-typed error, LockOperation and
            // wording included, rather than inventing a second dialect.
            if let Some(state) = state_of(raw) {
                state.error = Some(e);
            }
            abi::status::TIMEOUT
        }
    }
}

unsafe extern "C" fn h_unlock(raw: *mut Ctx, token: u64) -> u32 {
    let floor = match ctx_of(raw) {
        Some(c) => c.lock_floor,
        None => return abi::status::BAD_CTX,
    };
    let Some(vm) = vm_of(raw) else { return abi::status::NO_SCOPE };
    // Only locks this call took may be released, and only down to the depth the
    // token names — an extension must never be able to drop a lock an enclosing
    // `<cflock>` is holding.
    let target = (token as usize).max(floor + 1);
    while vm.held_lock_depth() >= target {
        vm.release_top_lock();
    }
    abi::status::OK
}

// ---- rooted values ---------------------------------------------------------

/// Values an extension has asked to keep beyond a call.
///
/// **Cycle-collector participation comes for free**, and that is worth stating
/// because it looked like the hard part: the collector decides liveness by
/// refcount (`external = strong_count − 1 − internal_in`), not from a root
/// list. A `CfmlValue` parked here holds its `Arc`, this table is not in the
/// survivor set, so the value reads as externally owned and is protected. There
/// is a test for exactly that, because "it should follow" is not evidence.
static ROOTS: Mutex<Option<(u64, HashMap<u64, CfmlValue>)>> = Mutex::new(None);

unsafe extern "C" fn h_root(raw: *mut Ctx, h: ValueHandle, out: *mut u64) -> u32 {
    let v = out_or!(value_of(raw, h), abi::status::BAD_HANDLE);
    let mut guard = ROOTS.lock().unwrap_or_else(|e| e.into_inner());
    let table = guard.get_or_insert_with(|| (0, HashMap::new()));
    table.0 += 1;
    let id = table.0;
    table.1.insert(id, v);
    *out = id;
    abi::status::OK
}

unsafe extern "C" fn h_unroot(id: u64) -> u32 {
    let mut guard = ROOTS.lock().unwrap_or_else(|e| e.into_inner());
    match guard.as_mut().and_then(|t| t.1.remove(&id)) {
        Some(_) => abi::status::OK,
        None => abi::status::NOT_FOUND,
    }
}

unsafe extern "C" fn h_root_get(raw: *mut Ctx, id: u64) -> ValueHandle {
    let found = {
        let guard = ROOTS.lock().unwrap_or_else(|e| e.into_inner());
        guard.as_ref().and_then(|t| t.1.get(&id).cloned())
    };
    match found {
        Some(v) => make(raw, v),
        None => ValueHandle::NULL,
    }
}

/// Mappings contributed by loaded extensions, as `(prefix, directory)`.
///
/// Process-global, not per-VM, because several engine paths REPLACE
/// `CfmlVirtualMachine::mappings` wholesale (`Application.cfc`'s
/// `this.mappings`, a thread seed, `application action="update"`). Keeping the
/// extension set here and re-applying it after those means an extension's CFCs
/// cannot silently vanish when an application declares its own mappings — which
/// is exactly how the shipped-CFML payload failed the first time.
static EXTENSION_MAPPINGS: Mutex<Option<Vec<(String, String)>>> = Mutex::new(None);

/// Record a mapping an extension provides. Idempotent.
pub fn register_extension_mapping(prefix: String, dir: String) {
    let mut guard = EXTENSION_MAPPINGS.lock().unwrap_or_else(|e| e.into_inner());
    let list = guard.get_or_insert_with(Vec::new);
    if !list.iter().any(|(p, _)| *p == prefix) {
        list.push((prefix, dir));
    }
}

/// Every extension-provided mapping, for the engine to re-apply.
pub fn extension_mappings() -> Vec<(String, String)> {
    EXTENSION_MAPPINGS
        .lock()
        .ok()
        .and_then(|g| g.clone())
        .unwrap_or_default()
}

/// How many values an extension is currently keeping alive. For tests and
/// diagnostics.
pub fn rooted_count() -> usize {
    ROOTS
        .lock()
        .ok()
        .and_then(|g| g.as_ref().map(|t| t.1.len()))
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// Tier 3 — CFML execution
// ---------------------------------------------------------------------------
//
// Everything here re-enters the engine while a module call is in flight. That is
// only possible because object dispatch stopped holding the exclusive guard for
// self-synchronising natives (`CfmlNative::needs_exclusive`): an extension
// method calling CFML that calls back into the same object would otherwise
// block on the lock its own caller holds.

/// Pull the argument handles into owned values, for a re-entrant call.
///
/// Cloning out first matters: the callee runs arbitrary CFML, which may create
/// values and grow the slab, and a borrow into it would not survive that.
unsafe fn args_out(raw: *mut Ctx, args: *const ValueHandle, argc: usize) -> Vec<CfmlValue> {
    if argc == 0 || args.is_null() {
        return Vec::new();
    }
    std::slice::from_raw_parts(args, argc)
        .iter()
        .map(|h| value_of(raw, *h).unwrap_or(CfmlValue::Null))
        .collect()
}

/// Stage an error raised by re-entrant CFML so the module's return becomes it.
unsafe fn stage_err(raw: *mut Ctx, e: CfmlError) -> ValueHandle {
    if let Some(state) = state_of(raw) {
        state.error = Some(e);
    }
    ValueHandle::NULL
}

unsafe extern "C-unwind" fn h_call_fn(
    raw: *mut Ctx,
    name: StrRef,
    args: *const ValueHandle,
    argc: usize,
) -> ValueHandle {
    let fname = str_in(name);
    let vals = args_out(raw, args, argc);
    let locals = locals_of(raw).clone();
    let Some(vm) = vm_of(raw) else { return ValueHandle::NULL };
    let Some(callee) = vm.resolve_callable(&fname, &locals) else {
        return stage_err(
            raw,
            CfmlError::runtime(format!("Function [{}] not found", fname)),
        );
    };
    match vm.call_value_public(&callee, vals, &locals) {
        Ok(v) => make(raw, v),
        Err(e) => stage_err(raw, e),
    }
}

unsafe extern "C-unwind" fn h_call_value(
    raw: *mut Ctx,
    callee: ValueHandle,
    args: *const ValueHandle,
    argc: usize,
) -> ValueHandle {
    let f = out_or!(value_of(raw, callee), ValueHandle::NULL);
    if !matches!(f, CfmlValue::Function(_) | CfmlValue::Closure(_)) {
        return stage_err(
            raw,
            CfmlError::runtime(format!("cannot call a value of type {}", f.type_name())),
        );
    }
    let vals = args_out(raw, args, argc);
    let locals = locals_of(raw).clone();
    let Some(vm) = vm_of(raw) else { return ValueHandle::NULL };
    match vm.call_value_public(&f, vals, &locals) {
        Ok(v) => make(raw, v),
        Err(e) => stage_err(raw, e),
    }
}

unsafe extern "C-unwind" fn h_new_component(
    raw: *mut Ctx,
    path: StrRef,
    args: *const ValueHandle,
    argc: usize,
) -> ValueHandle {
    let p = str_in(path);
    let vals = args_out(raw, args, argc);
    let locals = locals_of(raw).clone();
    let Some(vm) = vm_of(raw) else { return ValueHandle::NULL };
    match vm.new_component_public(&p, vals, &locals) {
        Ok(v) => make(raw, v),
        Err(e) => stage_err(raw, e),
    }
}

unsafe extern "C-unwind" fn h_invoke_method(
    raw: *mut Ctx,
    object: ValueHandle,
    name: StrRef,
    args: *const ValueHandle,
    argc: usize,
) -> ValueHandle {
    let obj = out_or!(value_of(raw, object), ValueHandle::NULL);
    let method = str_in(name);
    let vals = args_out(raw, args, argc);
    let locals = locals_of(raw).clone();
    let Some(vm) = vm_of(raw) else { return ValueHandle::NULL };
    match vm.invoke_method_public(&obj, &method, vals, &locals) {
        Ok(v) => make(raw, v),
        Err(e) => stage_err(raw, e),
    }
}

unsafe extern "C-unwind" fn h_component_set(
    raw: *mut Ctx,
    object: ValueHandle,
    name: StrRef,
    value: ValueHandle,
) -> u32 {
    let obj = out_or!(value_of(raw, object), abi::status::BAD_HANDLE);
    let v = out_or!(value_of(raw, value), abi::status::BAD_HANDLE);
    let key = str_in(name);
    // Writes the component's **`variables`** scope, not `this` — because that is
    // what injecting a dependency means, and what the component's own methods
    // read. Setting the public member instead compiles, runs, and leaves
    // `variables.injected` untouched, so the injection silently does nothing.
    //
    // A component arrives either as the marker struct or, with the flyweight
    // instance model on (the default), as an `Instance`. `createObject` returns
    // the latter, so handling only the struct form fails every real injection.
    match &obj {
        #[cfg(feature = "component-instance")]
        CfmlValue::Instance(inst) => {
            let vars = inst.read().private_map_handle();
            vars.insert(key, v);
            abi::status::OK
        }
        CfmlValue::Struct(s) => {
            match s.get_ci(&*cfml_common::key::well_known::VARIABLES) {
                Some(CfmlValue::Struct(vars)) => vars.insert(key, v),
                // A marker with no `variables` scope assembled yet (an
                // in-construction `this`): fall back to the struct itself.
                _ => s.insert(key, v),
            };
            abi::status::OK
        }
        _ => abi::status::WRONG_TYPE,
    }
}

unsafe extern "C-unwind" fn h_component_metadata(raw: *mut Ctx, path: StrRef) -> ValueHandle {
    let p = str_in(path);
    let locals = locals_of(raw).clone();
    let Some(vm) = vm_of(raw) else { return ValueHandle::NULL };
    match vm.component_metadata_public(&p, &locals) {
        Ok(v) => make(raw, v),
        Err(e) => stage_err(raw, e),
    }
}

unsafe extern "C-unwind" fn h_write_output(raw: *mut Ctx, text: StrRef) -> u32 {
    let t = str_in(text);
    let Some(vm) = vm_of(raw) else { return abi::status::NO_SCOPE };
    vm.write_output_public(&t);
    abi::status::OK
}

unsafe extern "C-unwind" fn h_include_template(raw: *mut Ctx, path: StrRef) -> u32 {
    let p = str_in(path);
    let Some(vm) = vm_of(raw) else { return abi::status::NO_SCOPE };
    match vm.include_for_extension(&p) {
        Ok(()) => abi::status::OK,
        Err(e) => {
            stage_err(raw, e);
            1
        }
    }
}
