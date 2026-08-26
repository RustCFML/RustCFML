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
pub const HOST_TIER: u32 = abi::tier::VALUES;

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
    let state = state_of(raw)?;
    if h.gen != state.generation {
        return None;
    }
    state.slab.get(h.slot as usize).cloned()
}

/// Borrow a handle's value in place, for the accessors that hand out a pointer
/// into it.
unsafe fn value_ref<'a>(raw: *mut Ctx, h: ValueHandle) -> Option<&'a CfmlValue> {
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
    let mut ctx = HostCtx {
        magic: CTX_MAGIC,
        generation: state.generation,
        state: &mut state as *mut CallState,
        receiver: receiver.clone(),
    };
    let raw = &mut ctx as *mut HostCtx as *mut Ctx;

    // A panic inside the extension unwinds across `C-unwind` into here and
    // becomes a CFML error. Without this an extension bug takes the process
    // down, which is not an acceptable failure mode for a plugin.
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f(raw, &handles)));

    // Drop the ctx before touching `state` again: it holds a raw pointer to it.
    drop(ctx);

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

    fn call_method(&mut self, name: &str, args: Vec<CfmlValue>) -> CfmlResult {
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
/// # Safety
/// `decl` must point at a live `ModuleDecl` produced by `rustcfml_module_decl`.
pub unsafe fn adopt(decl: *const abi::ModuleDecl, source: &str) -> Result<LoadedModule, String> {
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

    let name: Arc<str> = Arc::from(d.name.as_str());
    let mut bifs = Vec::with_capacity(d.bif_count);
    if !d.bifs.is_null() {
        for i in 0..d.bif_count {
            let b = &*d.bifs.add(i);
            bifs.push(ForeignBuiltin {
                entry: b.entry,
                module: leak(&name),
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

    // on_load: once per process. Given a real ctx so config can be delivered as
    // an ordinary struct handle rather than a second marshalling scheme.
    if let Some(on_load) = d.on_load {
        let config = CfmlValue::Struct(CfmlStruct::empty());
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
    })
}

/// A struct built from a `ValueMap`, for `throw`'s `extras`.
#[allow(dead_code)]
fn _unused(_: ValueMap) {}
