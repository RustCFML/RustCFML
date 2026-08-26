//! Write a RustCFML extension in safe Rust.
//!
//! The raw [`cfml_module_abi`] surface is a C ABI full of raw pointers and
//! status codes. Nobody should write that by hand, so this crate wraps it: you
//! write ordinary functions taking `&Ctx` and `&[Value]` and returning
//! `Result<Value>`, list them in [`module!`], and the macro generates the
//! `extern "C"` layer.
//!
//! ```ignore
//! use rustcfml_module::{module, Ctx, Result, Value};
//!
//! fn greet(ctx: &Ctx, args: &[Value]) -> Result<Value> {
//!     let name = args.first().map(|v| v.to_string()).unwrap_or_else(|| "World".into());
//!     Ok(ctx.string(format!("Hello, {name}")))
//! }
//!
//! module! {
//!     name: "greeter",
//!     version: "0.1.0",
//!     bifs: { "rustGreet" => greet },
//! }
//! ```
//!
//! # Two things that are deliberate, not accidental
//!
//! **`Value` is a borrowed handle, not an owned value.** You pay one host call
//! per field you actually touch, and a 10,000-row query passed as an argument
//! is never copied. A `Value` cannot outlive the call it came from — the borrow
//! checker enforces what the ABI's generation tag would otherwise only catch at
//! runtime.
//!
//! **Class methods take `&self`, not `&mut self`.** Interior mutability is
//! required from day one, because it is what lets the host dispatch a module's
//! methods without holding an exclusive lock once CFML re-entry arrives. Use a
//! `Mutex` or an atomic; the host asserts `Send + Sync` by contract.

use core::ffi::c_void;
use core::sync::atomic::{AtomicPtr, Ordering};

pub use cfml_module_abi as abi;
use abi::{HostVtable, StrRef, ValueHandle};

// ---------------------------------------------------------------------------
// The host vtable, installed once at load
// ---------------------------------------------------------------------------

static HOST: AtomicPtr<HostVtable> = AtomicPtr::new(core::ptr::null_mut());

/// Install the host's service table. Called by the generated `on_load`.
///
/// # Safety
/// `vtable` must live for the process (the host's does; extensions are never
/// unloaded).
pub unsafe fn install_host(vtable: *const HostVtable) {
    HOST.store(vtable as *mut HostVtable, Ordering::Release);
}

fn host() -> &'static HostVtable {
    let p = HOST.load(Ordering::Acquire);
    assert!(!p.is_null(), "rustcfml-module: host vtable not installed — was on_load skipped?");
    unsafe { &*p }
}

/// The tier the loaded host implements. Compare against [`abi::tier`] before
/// using anything beyond values.
pub fn host_tier() -> u32 {
    host().tier
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// The CFML error a failed call raises.
#[derive(Debug, Clone)]
pub struct Error {
    pub kind: u32,
    pub custom_type: String,
    pub message: String,
}

impl Error {
    pub fn new(message: impl Into<String>) -> Error {
        Error {
            kind: abi::error_type::APPLICATION,
            custom_type: String::new(),
            message: message.into(),
        }
    }

    /// An expression error — the category CFML uses for a bad argument.
    pub fn expression(message: impl Into<String>) -> Error {
        Error { kind: abi::error_type::EXPRESSION, ..Error::new(message) }
    }

    /// A custom-typed error, catchable by `<cfcatch type="my.type">`.
    pub fn custom(custom_type: impl Into<String>, message: impl Into<String>) -> Error {
        Error {
            kind: abi::error_type::CUSTOM,
            custom_type: custom_type.into(),
            message: message.into(),
        }
    }
}

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for Error {}

pub type Result<T> = core::result::Result<T, Error>;

fn status(code: u32, what: &str) -> Result<()> {
    if code == abi::status::OK {
        return Ok(());
    }
    let reason = match code {
        abi::status::BAD_HANDLE => "the value handle is stale or invalid",
        abi::status::WRONG_TYPE => "the value is of the wrong type",
        abi::status::NOT_FOUND => "no such key or index",
        abi::status::BAD_CTX => "the ctx is not valid for this call",
        abi::status::UNSUPPORTED => "this host does not provide that capability",
        _ => "unknown host error",
    };
    Err(Error::expression(format!("{what}: {reason}")))
}

// ---------------------------------------------------------------------------
// Ctx
// ---------------------------------------------------------------------------

/// The per-call context. Every entry point receives one, and it is the only
/// way to make a value or raise an error.
///
/// It is `!Send`, `!Sync` and carries no lifetime you can escape: storing one
/// and using it on a later request is the mistake the ABI's generation tag
/// exists to catch, and this type makes it hard to reach that point.
pub struct Ctx {
    raw: *mut abi::Ctx,
    _not_send: core::marker::PhantomData<*const ()>,
}

impl Ctx {
    /// # Safety
    /// `raw` must be the pointer the host passed for the call in progress.
    pub unsafe fn from_raw(raw: *mut abi::Ctx) -> Ctx {
        Ctx { raw, _not_send: core::marker::PhantomData }
    }

    fn wrap(&self, h: ValueHandle) -> Value<'_> {
        Value { h, ctx: self }
    }

    pub fn null(&self) -> Value<'_> {
        self.wrap(unsafe { (host().new_null)(self.raw) })
    }

    pub fn bool(&self, v: bool) -> Value<'_> {
        self.wrap(unsafe { (host().new_bool)(self.raw, v) })
    }

    pub fn int(&self, v: i64) -> Value<'_> {
        self.wrap(unsafe { (host().new_int)(self.raw, v) })
    }

    pub fn double(&self, v: f64) -> Value<'_> {
        self.wrap(unsafe { (host().new_double)(self.raw, v) })
    }

    /// A CFML timespan — numerically a count of fractional days.
    pub fn timespan(&self, days: f64) -> Value<'_> {
        self.wrap(unsafe { (host().new_timespan)(self.raw, days) })
    }

    pub fn string(&self, v: impl AsRef<str>) -> Value<'_> {
        let s = v.as_ref();
        self.wrap(unsafe { (host().new_string)(self.raw, StrRef::new(s)) })
    }

    pub fn binary(&self, v: &[u8]) -> Value<'_> {
        let r = StrRef { ptr: v.as_ptr(), len: v.len() };
        self.wrap(unsafe { (host().new_binary)(self.raw, r) })
    }

    pub fn array(&self) -> Value<'_> {
        self.wrap(unsafe { (host().arr_new)(self.raw, 0) })
    }

    pub fn array_with_capacity(&self, n: usize) -> Value<'_> {
        self.wrap(unsafe { (host().arr_new)(self.raw, n) })
    }

    pub fn strukt(&self) -> Value<'_> {
        self.wrap(unsafe { (host().struct_new)(self.raw) })
    }

    /// A query with the given column names and no rows.
    pub fn query(&self, columns: &[&str]) -> Result<Value<'_>> {
        let cols = self.array_with_capacity(columns.len());
        for c in columns {
            cols.push(self.string(c))?;
        }
        Ok(self.wrap(unsafe { (host().query_new)(self.raw, cols.h) }))
    }

    /// "Return the receiver" — the fluent self-handle. Only valid as the return
    /// value of a native class method.
    pub fn this(&self) -> Value<'_> {
        self.wrap(ValueHandle::SELF)
    }

    /// Hand a module-owned instance to the host as a CFML native object, so a
    /// BIF can return one of the module's own classes.
    pub fn new_object<C: NativeClass>(&self, instance: C) -> Value<'_> {
        let data = Box::into_raw(Box::new(instance)) as *mut c_void;
        self.wrap(unsafe { (host().new_native)(self.raw, data, class_vtable::<C>()) })
    }
}

// ---------------------------------------------------------------------------
// Value
// ---------------------------------------------------------------------------

/// A borrowed handle onto a host-owned CFML value.
///
/// Reading costs one indirect call per field you touch; nothing is copied
/// until you ask for it. The lifetime ties it to its call, so it cannot be
/// stored across requests.
#[derive(Copy, Clone)]
pub struct Value<'a> {
    h: ValueHandle,
    ctx: &'a Ctx,
}

impl<'a> Value<'a> {
    pub fn handle(self) -> ValueHandle {
        self.h
    }

    pub fn kind(self) -> u32 {
        unsafe { (host().val_type)(self.ctx.raw, self.h) }
    }

    pub fn is_null(self) -> bool {
        self.kind() == abi::ty::NULL
    }

    /// CFML truthiness — `true`, `"yes"`, `1`, a non-empty query, …
    pub fn is_true(self) -> bool {
        let mut out = false;
        unsafe { (host().val_is_true)(self.ctx.raw, self.h, &mut out) };
        out
    }

    /// The string contents, without copying — only for an actual string.
    pub fn as_str(self) -> Result<&'a str> {
        let mut r = StrRef::EMPTY;
        status(unsafe { (host().val_str)(self.ctx.raw, self.h, &mut r) }, "as_str")?;
        Ok(unsafe { r.as_str() })
    }

    /// The bytes of a Binary value, without copying.
    pub fn as_bytes(self) -> Result<&'a [u8]> {
        let mut r = StrRef::EMPTY;
        status(unsafe { (host().val_bytes)(self.ctx.raw, self.h, &mut r) }, "as_bytes")?;
        if r.ptr.is_null() {
            return Ok(&[]);
        }
        Ok(unsafe { core::slice::from_raw_parts(r.ptr, r.len) })
    }

    pub fn as_i64(self) -> Result<i64> {
        let mut out = 0i64;
        status(unsafe { (host().val_i64)(self.ctx.raw, self.h, &mut out) }, "as_i64")?;
        Ok(out)
    }

    pub fn as_f64(self) -> Result<f64> {
        let mut out = 0f64;
        status(unsafe { (host().val_f64)(self.ctx.raw, self.h, &mut out) }, "as_f64")?;
        Ok(out)
    }

    pub fn as_bool(self) -> Result<bool> {
        let mut out = false;
        status(unsafe { (host().val_bool)(self.ctx.raw, self.h, &mut out) }, "as_bool")?;
        Ok(out)
    }

    // ---- arrays ----------------------------------------------------------

    pub fn len(self) -> Result<usize> {
        let mut out = 0usize;
        status(unsafe { (host().arr_len)(self.ctx.raw, self.h, &mut out) }, "len")?;
        Ok(out)
    }

    pub fn is_empty(self) -> Result<bool> {
        Ok(self.len()? == 0)
    }

    /// Zero-based, unlike CFML itself. The ABI is not the place for 1-based
    /// arithmetic; convert at your own API's edge if it matters.
    pub fn get(self, index: usize) -> Value<'a> {
        self.ctx.wrap(unsafe { (host().arr_get)(self.ctx.raw, self.h, index) })
    }

    pub fn set(self, index: usize, v: Value<'_>) -> Result<()> {
        status(unsafe { (host().arr_set)(self.ctx.raw, self.h, index, v.h) }, "set")
    }

    pub fn push(self, v: Value<'_>) -> Result<()> {
        status(unsafe { (host().arr_push)(self.ctx.raw, self.h, v.h) }, "push")
    }

    /// Every element, as a Vec of handles. One crossing per element — use it
    /// when you are going to touch them all anyway.
    pub fn to_vec(self) -> Result<Vec<Value<'a>>> {
        let n = self.len()?;
        Ok((0..n).map(|i| self.get(i)).collect())
    }

    // ---- structs ---------------------------------------------------------

    /// Case-insensitive, like every CFML key lookup.
    pub fn key(self, name: &str) -> Value<'a> {
        self.ctx.wrap(unsafe { (host().struct_get)(self.ctx.raw, self.h, StrRef::new(name)) })
    }

    pub fn put(self, name: &str, v: Value<'_>) -> Result<()> {
        status(
            unsafe { (host().struct_set)(self.ctx.raw, self.h, StrRef::new(name), v.h) },
            "put",
        )
    }

    pub fn has_key(self, name: &str) -> bool {
        let mut out = false;
        unsafe { (host().struct_has)(self.ctx.raw, self.h, StrRef::new(name), &mut out) };
        out
    }

    pub fn remove(self, name: &str) -> Result<()> {
        status(
            unsafe { (host().struct_delete)(self.ctx.raw, self.h, StrRef::new(name)) },
            "remove",
        )
    }

    pub fn key_count(self) -> Result<usize> {
        let mut out = 0usize;
        status(unsafe { (host().struct_len)(self.ctx.raw, self.h, &mut out) }, "key_count")?;
        Ok(out)
    }

    /// Keys in insertion order.
    pub fn keys(self) -> Result<Vec<&'a str>> {
        let n = self.key_count()?;
        let mut out = Vec::with_capacity(n);
        for i in 0..n {
            let mut r = StrRef::EMPTY;
            status(
                unsafe { (host().struct_key_at)(self.ctx.raw, self.h, i, &mut r) },
                "keys",
            )?;
            out.push(unsafe { r.as_str() });
        }
        Ok(out)
    }

    // ---- queries ---------------------------------------------------------

    pub fn query_columns(self) -> Result<Vec<&'a str>> {
        let mut n = 0usize;
        status(unsafe { (host().query_cols)(self.ctx.raw, self.h, &mut n) }, "query_columns")?;
        let mut out = Vec::with_capacity(n);
        for i in 0..n {
            let mut r = StrRef::EMPTY;
            status(
                unsafe { (host().query_col_name)(self.ctx.raw, self.h, i, &mut r) },
                "query_columns",
            )?;
            out.push(unsafe { r.as_str() });
        }
        Ok(out)
    }

    pub fn query_row_count(self) -> Result<usize> {
        let mut out = 0usize;
        status(unsafe { (host().query_rows)(self.ctx.raw, self.h, &mut out) }, "query_row_count")?;
        Ok(out)
    }

    /// Zero-based row and column.
    pub fn query_cell(self, row: usize, col: usize) -> Value<'a> {
        self.ctx.wrap(unsafe { (host().query_cell)(self.ctx.raw, self.h, row, col) })
    }

    pub fn query_column_index(self, name: &str) -> Result<usize> {
        let mut out = 0usize;
        status(
            unsafe { (host().query_col_index)(self.ctx.raw, self.h, StrRef::new(name), &mut out) },
            "query_column_index",
        )?;
        Ok(out)
    }

    /// A whole column as an array — **one** crossing rather than one per row.
    /// This is the bulk read; reach for it before looping `query_cell`.
    pub fn query_column(self, col: usize) -> Value<'a> {
        self.ctx.wrap(unsafe { (host().query_col_values)(self.ctx.raw, self.h, col) })
    }

    pub fn query_set_cell(self, row: usize, col: usize, v: Value<'_>) -> Result<()> {
        status(
            unsafe { (host().query_set_cell)(self.ctx.raw, self.h, row, col, v.h) },
            "query_set_cell",
        )
    }

    /// Append a row from an array of cell values, in column order.
    pub fn query_add_row(self, row: Value<'_>) -> Result<()> {
        status(unsafe { (host().query_add_row)(self.ctx.raw, self.h, row.h) }, "query_add_row")
    }

    // ---- components and natives ------------------------------------------

    pub fn component_name(self) -> Result<&'a str> {
        let mut r = StrRef::EMPTY;
        status(
            unsafe { (host().component_name)(self.ctx.raw, self.h, &mut r) },
            "component_name",
        )?;
        Ok(unsafe { r.as_str() })
    }

    pub fn native_class_name(self) -> Result<&'a str> {
        let mut r = StrRef::EMPTY;
        status(
            unsafe { (host().native_class_name)(self.ctx.raw, self.h, &mut r) },
            "native_class_name",
        )?;
        Ok(unsafe { r.as_str() })
    }
}

impl core::fmt::Display for Value<'_> {
    /// CFML's own stringification, for any value.
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let h = unsafe { (host().val_to_string)(self.ctx.raw, self.h) };
        let mut r = StrRef::EMPTY;
        let code = unsafe { (host().val_str)(self.ctx.raw, h, &mut r) };
        if code == abi::status::OK {
            f.write_str(unsafe { r.as_str() })
        } else {
            Ok(())
        }
    }
}

// ---------------------------------------------------------------------------
// Native classes
// ---------------------------------------------------------------------------

/// A Rust type exposed to CFML as a class.
///
/// Note `&self` on every method. Interior mutability is required, not
/// suggested: the host asserts `Send + Sync` for you, and dispatching without
/// an exclusive lock is what makes CFML re-entry possible later without a
/// breaking change to this trait.
pub trait NativeClass: Send + Sync + Sized + 'static {
    /// The name CFML sees, e.g. in `getMetadata()` and `writeDump`.
    const CLASS_NAME: &'static str;

    /// `createObject( "rust", CLASS_NAME, … )`.
    fn new(ctx: &Ctx, args: &[Value]) -> Result<Self>;

    fn call<'a>(&self, ctx: &'a Ctx, method: &str, args: &[Value<'a>]) -> Result<Value<'a>>;

    /// Comma-separated parameter names for `method`, in positional order, so a
    /// NAMED call binds correctly. Returning `None` makes the host **refuse**
    /// named arguments for that method rather than binding them by position —
    /// which is a silent wrong answer, so the refusal is deliberate.
    fn method_params(_method: &str) -> Option<&'static str> {
        None
    }

    /// `this.X` read fall-through for `component extends="rust:…"`. `None`
    /// declines and lets the CFC's own struct answer.
    fn get_property<'a>(&self, _ctx: &'a Ctx, _name: &str) -> Option<Result<Value<'a>>> {
        None
    }

    /// `this.X` write fall-through. `None` declines.
    fn set_property(&self, _ctx: &Ctx, _name: &str, _value: Value) -> Option<Result<()>> {
        None
    }
}

/// Turn a `Result` into what the ABI wants: a handle, or a thrown error and
/// [`ValueHandle::NULL`].
fn finish(ctx: &Ctx, r: Result<Value>) -> ValueHandle {
    match r {
        Ok(v) => v.h,
        Err(e) => {
            unsafe {
                (host().throw)(
                    ctx.raw,
                    e.kind,
                    StrRef::new(&e.custom_type),
                    StrRef::new(&e.message),
                    ValueHandle::NULL,
                )
            };
            ValueHandle::NULL
        }
    }
}

/// Run module code with panics turned into CFML errors.
///
/// The host catches too, as a backstop, but catching here produces a much
/// better message: we still know which function panicked.
fn guard<F: FnOnce() -> ValueHandle + std::panic::UnwindSafe>(
    ctx: &Ctx,
    what: &str,
    f: F,
) -> ValueHandle {
    match std::panic::catch_unwind(f) {
        Ok(h) => h,
        Err(payload) => {
            let detail = payload
                .downcast_ref::<&str>()
                .map(|s| s.to_string())
                .or_else(|| payload.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "unknown panic".to_string());
            finish(ctx, Err(Error::new(format!("{what} panicked: {detail}"))))
        }
    }
}

#[doc(hidden)]
pub mod internal {
    //! Machinery the [`module!`] macro emits. Not a stable surface.

    use super::*;

    /// One declared BIF, as a type so the trampoline can be a generic
    /// `extern "C"` function. `macro_rules!` cannot build an identifier out of
    /// a string literal, and this sidesteps needing to: the macro declares an
    /// anonymous unit struct per entry inside the `static` initialiser.
    pub trait Bif {
        const NAME: &'static str;
        fn call<'a>(ctx: &'a Ctx, args: &[Value<'a>]) -> Result<Value<'a>>;
    }

    /// How many arguments are borrowed without touching the heap.
    ///
    /// Almost every CFML call is under this, and the `Vec` it replaces was a
    /// malloc/free pair on every single crossing — material on a BIF whose whole
    /// job takes a few hundred nanoseconds.
    const INLINE_ARGS: usize = 8;

    pub fn with_args<'a, R>(
        ctx: &'a Ctx,
        args: *const ValueHandle,
        argc: usize,
        f: impl FnOnce(&[Value<'a>]) -> R,
    ) -> R {
        if argc == 0 {
            return f(&[]);
        }
        let handles = unsafe { core::slice::from_raw_parts(args, argc) };
        if argc <= INLINE_ARGS {
            // Build on the stack. `Value` is Copy and two words wide, so a
            // fixed array costs nothing and skips the allocation entirely.
            let first = Value { h: handles[0], ctx };
            let mut buf = [first; INLINE_ARGS];
            for (i, h) in handles.iter().enumerate() {
                buf[i] = Value { h: *h, ctx };
            }
            return f(&buf[..argc]);
        }
        let spilled: Vec<Value> = handles.iter().map(|h| Value { h: *h, ctx }).collect();
        f(&spilled)
    }

    /// Wrap a raw handle the host handed us.
    pub fn value_from<'a>(ctx: &'a Ctx, h: ValueHandle) -> Value<'a> {
        Value { h, ctx }
    }

    /// # Safety
    /// Called by the host with a live ctx and `argc` valid handles.
    pub unsafe extern "C-unwind" fn bif_shim<B: Bif>(
        raw: *mut abi::Ctx,
        args: *const ValueHandle,
        argc: usize,
    ) -> ValueHandle {
        let ctx = Ctx::from_raw(raw);
        with_args(&ctx, args, argc, |values| {
            guard(
                &ctx,
                B::NAME,
                std::panic::AssertUnwindSafe(|| finish(&ctx, B::call(&ctx, values))),
            )
        })
    }

    /// # Safety
    /// Called by the host with a live ctx and `argc` valid handles.
    pub unsafe extern "C-unwind" fn class_ctor<C: NativeClass>(
        raw: *mut abi::Ctx,
        args: *const ValueHandle,
        argc: usize,
        out: *mut *mut c_void,
    ) -> u32 {
        let ctx = Ctx::from_raw(raw);
        let built = with_args(&ctx, args, argc, |values| {
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| C::new(&ctx, values)))
        });
        match built {
            Ok(Ok(instance)) => {
                *out = Box::into_raw(Box::new(instance)) as *mut c_void;
                abi::status::OK
            }
            Ok(Err(e)) => {
                throw(raw, &e);
                1
            }
            Err(_) => {
                throw(raw, &Error::new(format!("{}: constructor panicked", C::CLASS_NAME)));
                1
            }
        }
    }

    fn throw(raw: *mut abi::Ctx, e: &Error) {
        unsafe {
            (host().throw)(
                raw,
                e.kind,
                StrRef::new(&e.custom_type),
                StrRef::new(&e.message),
                ValueHandle::NULL,
            )
        };
    }
}

// ---- the generated class vtable -------------------------------------------

unsafe extern "C" fn vt_class_name<C: NativeClass>(_data: *mut c_void) -> StrRef {
    StrRef::new(C::CLASS_NAME)
}

unsafe extern "C-unwind" fn vt_call_method<C: NativeClass>(
    raw: *mut abi::Ctx,
    data: *mut c_void,
    name: StrRef,
    args: *const ValueHandle,
    argc: usize,
) -> ValueHandle {
    let ctx = Ctx::from_raw(raw);
    let this = &*(data as *const C);
    let method = name.as_str();
    internal::with_args(&ctx, args, argc, |values| {
        guard(
            &ctx,
            C::CLASS_NAME,
            std::panic::AssertUnwindSafe(|| finish(&ctx, this.call(&ctx, method, values))),
        )
    })
}

unsafe extern "C" fn vt_method_params<C: NativeClass>(
    _data: *mut c_void,
    name: StrRef,
    out: *mut StrRef,
) -> u32 {
    match C::method_params(name.as_str()) {
        Some(list) => {
            *out = StrRef::new(list);
            abi::status::OK
        }
        None => {
            *out = StrRef::EMPTY;
            abi::status::NOT_FOUND
        }
    }
}

unsafe extern "C-unwind" fn vt_get_property<C: NativeClass>(
    raw: *mut abi::Ctx,
    data: *mut c_void,
    name: StrRef,
    out: *mut ValueHandle,
) -> u32 {
    let ctx = Ctx::from_raw(raw);
    let this = &*(data as *const C);
    match this.get_property(&ctx, name.as_str()) {
        None => abi::status::NOT_FOUND,
        Some(r) => {
            *out = finish(&ctx, r);
            abi::status::OK
        }
    }
}

unsafe extern "C-unwind" fn vt_set_property<C: NativeClass>(
    raw: *mut abi::Ctx,
    data: *mut c_void,
    name: StrRef,
    value: ValueHandle,
) -> u32 {
    let ctx = Ctx::from_raw(raw);
    let this = &*(data as *const C);
    let v = Value { h: value, ctx: &ctx };
    match this.set_property(&ctx, name.as_str(), v) {
        None => abi::status::NOT_FOUND,
        Some(Ok(())) => abi::status::OK,
        Some(Err(e)) => {
            (host().throw)(
                raw,
                e.kind,
                StrRef::new(&e.custom_type),
                StrRef::new(&e.message),
                ValueHandle::NULL,
            );
            1
        }
    }
}

unsafe extern "C" fn vt_drop<C: NativeClass>(data: *mut c_void) {
    if !data.is_null() {
        drop(Box::from_raw(data as *mut C));
    }
}

/// Holder for the per-class method table.
///
/// An associated `const` can be generic where a `static` cannot, and a table of
/// nothing but function pointers and a `usize` is const-promotable — so each
/// class gets one `'static` vtable with no allocation, no lock and no leak.
pub struct Vt<C>(core::marker::PhantomData<C>);

impl<C: NativeClass> Vt<C> {
    pub const TABLE: abi::NativeClassVtable = abi::NativeClassVtable {
        size: core::mem::size_of::<abi::NativeClassVtable>(),
        class_name: vt_class_name::<C>,
        call_method: vt_call_method::<C>,
        method_params: vt_method_params::<C>,
        get_property: vt_get_property::<C>,
        set_property: vt_set_property::<C>,
        drop_fn: vt_drop::<C>,
    };
}

/// The `#[repr(C)]` method table for `C`, with process lifetime.
pub const fn class_vtable<C: NativeClass>() -> *const abi::NativeClassVtable {
    &Vt::<C>::TABLE
}


// ---------------------------------------------------------------------------
// Tier 2 — scopes, locks, and values that outlive a call
// ---------------------------------------------------------------------------

/// Fail clearly when the loaded host is older than the capability being used,
/// rather than calling through a vtable slot it never filled in.
fn require(entry_offset: usize, what: &str) -> Result<()> {
    require_tier(entry_offset, what, abi::tier::SCOPES)
}

fn require_tier(entry_offset: usize, what: &str, tier: u32) -> Result<()> {
    let host = host();
    if host.tier < tier || !host.has(entry_offset) {
        return Err(Error::new(format!(
            "{what} needs an engine that provides extension capability tier {}; \
             this one provides tier {}",
            tier, host.tier
        )));
    }
    Ok(())
}

/// One CFML scope, reached by name.
///
/// Reads take that scope's read lock, so a value can never be read half-written
/// by a concurrent CFML `<cflock>`. **Writing a shared scope (`application`,
/// `session`, `server`) requires a lock you are already holding** — see
/// [`Ctx::lock`]. That is stricter than CFML itself, on purpose: an extension
/// can write a live shared scope from a thread the application never thinks
/// about.
pub struct Scope<'a> {
    ctx: &'a Ctx,
    /// Owned, so a returned [`Value`] borrows only the ctx. Borrowing the name
    /// instead tied every value read out of a scope to the lifetime of the
    /// string that named it, which made the obvious one-liner
    /// (`ctx.scope(&name).get(&key)`) fail to compile.
    name: String,
}

impl<'a> Scope<'a> {
    pub fn get(&self, key: &str) -> Result<Value<'a>> {
        require(core::mem::offset_of!(HostVtable, scope_get), "reading a scope")?;
        let h = unsafe {
            (host().scope_get)(self.ctx.raw, StrRef::new(&self.name), StrRef::new(key))
        };
        Ok(Value { h, ctx: self.ctx })
    }

    pub fn set(&self, key: &str, value: Value<'_>) -> Result<()> {
        require(core::mem::offset_of!(HostVtable, scope_set), "writing a scope")?;
        let code = unsafe {
            (host().scope_set)(
                self.ctx.raw,
                StrRef::new(&self.name),
                StrRef::new(key),
                value.h,
            )
        };
        if code == abi::status::UNLOCKED {
            return Err(Error::new(format!(
                "writing [{}] requires holding its lock — take ctx.lock(\"{}\", …) first",
                self.name, self.name
            )));
        }
        status(code, "scope set")
    }

    pub fn has(&self, key: &str) -> Result<bool> {
        require(core::mem::offset_of!(HostVtable, scope_has), "reading a scope")?;
        let mut out = false;
        let code = unsafe {
            (host().scope_has)(
                self.ctx.raw,
                StrRef::new(&self.name),
                StrRef::new(key),
                &mut out,
            )
        };
        status(code, "scope has")?;
        Ok(out)
    }

    pub fn remove(&self, key: &str) -> Result<()> {
        require(core::mem::offset_of!(HostVtable, scope_delete), "writing a scope")?;
        let code = unsafe {
            (host().scope_delete)(self.ctx.raw, StrRef::new(&self.name), StrRef::new(key))
        };
        status(code, "scope delete")
    }

    /// The whole scope as a **snapshot** struct — a copy. Walking a live shared
    /// scope key by key while another request writes it would race, so there is
    /// deliberately no live iterator.
    pub fn snapshot(&self) -> Result<Value<'a>> {
        require(core::mem::offset_of!(HostVtable, scope_snapshot), "reading a scope")?;
        let h = unsafe { (host().scope_snapshot)(self.ctx.raw, StrRef::new(&self.name)) };
        Ok(Value { h, ctx: self.ctx })
    }
}

/// A lock held for the rest of this call, or until dropped.
///
/// The host force-releases anything still held when the call returns, so a
/// forgotten guard cannot become a hang in the next request — but drop it when
/// you are done anyway, so CFML code waiting on the same scope is not blocked
/// for the rest of your function.
pub struct LockGuard<'a> {
    ctx: &'a Ctx,
    token: u64,
    released: bool,
}

impl LockGuard<'_> {
    /// Release now rather than at call end.
    pub fn release(mut self) {
        self.release_inner();
    }

    fn release_inner(&mut self) {
        if self.released {
            return;
        }
        self.released = true;
        unsafe { (host().unlock)(self.ctx.raw, self.token) };
    }
}

impl Drop for LockGuard<'_> {
    fn drop(&mut self) {
        self.release_inner();
    }
}

/// A value kept alive beyond the call that produced it — a cross-request cache.
///
/// Unroots on drop. Rooted values are visible to the cycle collector, so a
/// cache cannot be collected out from under you; the flip side is that a
/// `Rooted` you never drop is a leak for the life of the process.
pub struct Rooted {
    id: u64,
}

impl Rooted {
    /// Bring the value back as a handle in the current call.
    pub fn get<'a>(&self, ctx: &'a Ctx) -> Value<'a> {
        let h = unsafe { (host().root_get)(ctx.raw, self.id) };
        Value { h, ctx }
    }

    pub fn id(&self) -> u64 {
        self.id
    }
}

impl Drop for Rooted {
    fn drop(&mut self) {
        // No ctx here, and none needed: a root outlives every call, which is
        // why `unroot` is the one host entry that takes no context.
        unsafe { (host().unroot)(self.id) };
    }
}

impl Ctx {
    /// A CFML scope by name: `"application"`, `"session"`, `"server"`,
    /// `"request"`, `"variables"`, `"cgi"`, `"url"`, `"form"`, `"cookie"`.
    pub fn scope(&self, name: &str) -> Scope<'_> {
        Scope { ctx: self, name: name.to_string() }
    }

    /// An unqualified read, honouring CFML's own resolution order. Identical to
    /// what an unprefixed read in CFML source would see, because it goes through
    /// the engine's own resolver.
    pub fn var(&self, key: &str) -> Result<Value<'_>> {
        require(core::mem::offset_of!(HostVtable, var_get), "reading a variable")?;
        let h = unsafe { (host().var_get)(self.raw, StrRef::new(key)) };
        Ok(Value { h, ctx: self })
    }

    /// Take a scope lock — the same lock `<cflock scope="…">` takes, so CFML
    /// code and this extension mutually exclude.
    ///
    /// `timeout` of zero means wait forever, matching CFML.
    pub fn lock(&self, scope: &str, exclusive: bool, timeout_ms: u64) -> Result<LockGuard<'_>> {
        self.lock_inner(scope, "", exclusive, timeout_ms)
    }

    /// Take a named lock, equivalent to `<cflock name="…">`.
    pub fn lock_named(
        &self,
        name: &str,
        exclusive: bool,
        timeout_ms: u64,
    ) -> Result<LockGuard<'_>> {
        self.lock_inner("", name, exclusive, timeout_ms)
    }

    fn lock_inner(
        &self,
        scope: &str,
        name: &str,
        exclusive: bool,
        timeout_ms: u64,
    ) -> Result<LockGuard<'_>> {
        require(core::mem::offset_of!(HostVtable, lock), "locking")?;
        let mut token = 0u64;
        let code = unsafe {
            (host().lock)(
                self.raw,
                StrRef::new(scope),
                StrRef::new(name),
                exclusive,
                timeout_ms,
                &mut token,
            )
        };
        if code == abi::status::TIMEOUT {
            // The host has already staged the engine's own `lock`-typed error,
            // LockOperation and Lucee wording included, so returning a second
            // message of our own would mask it.
            return Err(Error::new(format!(
                "timed out acquiring the {} lock",
                if name.is_empty() { scope } else { name }
            )));
        }
        status(code, "lock")?;
        Ok(LockGuard { ctx: self, token, released: false })
    }

    /// Keep `value` alive beyond this call.
    pub fn root(&self, value: Value<'_>) -> Result<Rooted> {
        require(core::mem::offset_of!(HostVtable, root), "rooting a value")?;
        let mut id = 0u64;
        status(unsafe { (host().root)(self.raw, value.h, &mut id) }, "root")?;
        Ok(Rooted { id })
    }
}


// ---------------------------------------------------------------------------
// Tier 3 — running CFML
// ---------------------------------------------------------------------------

fn args_of(values: &[Value<'_>]) -> Vec<ValueHandle> {
    values.iter().map(|v| v.h).collect()
}

impl Ctx {
    /// Call a CFML function by name — a BIF, a UDF, anything in scope.
    ///
    /// ```ignore
    /// let upper = ctx.call( "ucase", &[ ctx.string( "hi" ) ] )?;
    /// ```
    pub fn call<'a>(&'a self, name: &str, args: &[Value<'a>]) -> Result<Value<'a>> {
        require_tier(
            core::mem::offset_of!(HostVtable, call_fn),
            "calling a CFML function",
            abi::tier::EXECUTION,
        )?;
        let handles = args_of(args);
        let h = unsafe {
            (host().call_fn)(self.raw, StrRef::new(name), handles.as_ptr(), handles.len())
        };
        self.checked(h, name)
    }

    /// `createObject( "component", path )`, constructor arguments included.
    pub fn new_component<'a>(&'a self, path: &str, args: &[Value<'a>]) -> Result<Value<'a>> {
        require_tier(
            core::mem::offset_of!(HostVtable, new_component),
            "instantiating a component",
            abi::tier::EXECUTION,
        )?;
        let handles = args_of(args);
        let h = unsafe {
            (host().new_component)(self.raw, StrRef::new(path), handles.as_ptr(), handles.len())
        };
        self.checked(h, path)
    }

    /// `getComponentMetadata( path )` — the annotations dependency injection
    /// is driven by.
    pub fn component_metadata(&self, path: &str) -> Result<Value<'_>> {
        require_tier(
            core::mem::offset_of!(HostVtable, component_metadata),
            "reading component metadata",
            abi::tier::EXECUTION,
        )?;
        let h = unsafe { (host().component_metadata)(self.raw, StrRef::new(path)) };
        self.checked(h, path)
    }

    /// Append to page output, honouring any capture in effect
    /// (`cfsavecontent`, `cfsilent`, a thread's buffer).
    pub fn write_output(&self, text: &str) -> Result<()> {
        require_tier(
            core::mem::offset_of!(HostVtable, write_output),
            "writing page output",
            abi::tier::EXECUTION,
        )?;
        status(unsafe { (host().write_output)(self.raw, StrRef::new(text)) }, "write_output")
    }

    /// Run a template for its output.
    ///
    /// Unlike `<cfinclude>` this has no calling frame to merge variables back
    /// into, so the template runs with a fresh scope and what it defines is
    /// discarded. Its output goes to the buffer.
    pub fn include(&self, path: &str) -> Result<()> {
        require_tier(
            core::mem::offset_of!(HostVtable, include_template),
            "including a template",
            abi::tier::EXECUTION,
        )?;
        let code = unsafe { (host().include_template)(self.raw, StrRef::new(path)) };
        if code != abi::status::OK {
            // The host staged the real CFML error; a second message here would
            // mask it.
            return Err(Error::new(format!("include [{}] failed", path)));
        }
        Ok(())
    }

    /// A NULL return means the host staged a CFML error for us; surfacing our
    /// own message instead would hide it, so this only fills in when there is
    /// nothing better.
    fn checked<'a>(&'a self, h: ValueHandle, what: &str) -> Result<Value<'a>> {
        if h.is_null() {
            return Err(Error::new(format!("[{}] failed", what)));
        }
        Ok(Value { h, ctx: self })
    }
}

impl<'a> Value<'a> {
    /// Call this value as a function — a UDF or closure handed to you as an
    /// argument. The mechanism behind `thing.onEvent( function(e){ … } )`.
    pub fn call_as_fn(self, args: &[Value<'a>]) -> Result<Value<'a>> {
        require_tier(
            core::mem::offset_of!(HostVtable, call_value),
            "calling a function value",
            abi::tier::EXECUTION,
        )?;
        let handles = args_of(args);
        let h = unsafe {
            (host().call_value)(self.ctx.raw, self.h, handles.as_ptr(), handles.len())
        };
        self.ctx.checked(h, "function value")
    }

    /// Invoke a method on a component or native object.
    pub fn invoke(self, method: &str, args: &[Value<'a>]) -> Result<Value<'a>> {
        require_tier(
            core::mem::offset_of!(HostVtable, invoke_method),
            "invoking a method",
            abi::tier::EXECUTION,
        )?;
        let handles = args_of(args);
        let h = unsafe {
            (host().invoke_method)(
                self.ctx.raw,
                self.h,
                StrRef::new(method),
                handles.as_ptr(),
                handles.len(),
            )
        };
        self.ctx.checked(h, method)
    }

    /// Set a property on a component — dependency injection.
    pub fn set_property(self, name: &str, value: Value<'_>) -> Result<()> {
        require_tier(
            core::mem::offset_of!(HostVtable, component_set),
            "injecting a component property",
            abi::tier::EXECUTION,
        )?;
        status(
            unsafe {
                (host().component_set)(self.ctx.raw, self.h, StrRef::new(name), value.h)
            },
            "component_set",
        )
    }
}

// ---------------------------------------------------------------------------
// module!
// ---------------------------------------------------------------------------

/// Declare everything the extension provides and generate the `extern "C"`
/// layer the host loads.
///
/// ```ignore
/// module! {
///     name: "typst",
///     version: "0.1.0",
///     bifs: { "typstCompile" => typst_compile, "Document" => document },
///     classes: { Document },
/// }
/// ```
///
/// `bifs` maps the CFML name to a `fn(&Ctx, &[Value]) -> Result<Value>`;
/// `classes` lists types implementing [`NativeClass`], whose CFML name comes
/// from `CLASS_NAME`. `on_load` names an optional
/// `fn() -> Result<()>` run once per process, after the host vtable is
/// installed — the place for thread pools and caches, never per-call work.
#[macro_export]
macro_rules! module {
    (
        name: $name:literal,
        version: $version:literal
        $(, tier: $tier:expr )?
        $(, bifs: { $( $bif_name:literal => $bif_fn:path ),* $(,)? } )?
        $(, classes: { $( $class:ty ),* $(,)? } )?
        $(, on_load: $on_load:path )?
        $(,)?
    ) => {
        #[doc(hidden)]
        static __RUSTCFML_BIFS: &[$crate::abi::BifDecl] = &[
            $( $(
                $crate::abi::BifDecl {
                    name: $crate::abi::StrRef::new($bif_name),
                    entry: {
                        // One unit type per entry, declared inside the
                        // initialiser: `macro_rules!` cannot mint an identifier
                        // from a string literal, and it does not need to.
                        struct Shim;
                        impl $crate::internal::Bif for Shim {
                            const NAME: &'static str = $bif_name;
                            fn call<'a>(
                                ctx: &'a $crate::Ctx,
                                args: &[$crate::Value<'a>],
                            ) -> $crate::Result<$crate::Value<'a>> {
                                $bif_fn(ctx, args)
                            }
                        }
                        $crate::internal::bif_shim::<Shim> as $crate::abi::ModuleFn
                    },
                }
            ),* )?
        ];

        #[doc(hidden)]
        static __RUSTCFML_CLASSES: &[$crate::abi::ClassDecl] = &[
            $( $(
                $crate::abi::ClassDecl {
                    name: $crate::abi::StrRef::new(
                        <$class as $crate::NativeClass>::CLASS_NAME,
                    ),
                    ctor: $crate::internal::class_ctor::<$class>
                        as $crate::abi::ClassCtorFn,
                    vtable: $crate::class_vtable::<$class>(),
                }
            ),* )?
        ];

        #[doc(hidden)]
        static __RUSTCFML_QOQ: &[$crate::abi::QoqFnDecl] = &[];

        #[doc(hidden)]
        unsafe extern "C-unwind" fn __rustcfml_on_load(
            vtable: *const $crate::abi::HostVtable,
            ctx: *mut $crate::abi::Ctx,
            config: $crate::abi::ValueHandle,
        ) -> u32 {
            $crate::install_host(vtable);
            $(
                let ctx = $crate::Ctx::from_raw(ctx);
                let settings = $crate::internal::value_from(&ctx, config);
                if $on_load(&ctx, settings).is_err() {
                    return 1;
                }
            )?
            0
        }

        #[doc(hidden)]
        static __RUSTCFML_DECL: $crate::abi::ModuleDecl = $crate::abi::ModuleDecl {
            size: core::mem::size_of::<$crate::abi::ModuleDecl>(),
            abi_major: $crate::abi::ABI_MAJOR,
            // The tier this extension REQUIRES. Declaring it lets an older
            // engine refuse the load up front with a legible message instead of
            // failing at the first call.
            tier: {
                #[allow(unused_mut, unused_assignments)]
                let mut t = $crate::abi::tier::VALUES;
                $( t = $tier; )?
                t
            },
            name: $crate::abi::StrRef::new($name),
            version: $crate::abi::StrRef::new($version),
            target: $crate::abi::StrRef::new($crate::abi::TARGET),
            on_load: Some(__rustcfml_on_load),
            bifs: __RUSTCFML_BIFS.as_ptr(),
            bif_count: __RUSTCFML_BIFS.len(),
            classes: __RUSTCFML_CLASSES.as_ptr(),
            class_count: __RUSTCFML_CLASSES.len(),
            qoq_fns: __RUSTCFML_QOQ.as_ptr(),
            qoq_fn_count: 0,
        };

        /// The symbol the loader resolves.
        #[no_mangle]
        pub extern "C" fn rustcfml_module_decl() -> *const $crate::abi::ModuleDecl {
            &__RUSTCFML_DECL
        }
    };
}
