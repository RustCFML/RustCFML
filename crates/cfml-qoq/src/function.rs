//! QoQ function registry: built-in (native Rust) and user-registered (CFML UDF)
//! functions usable inside QoQ SQL.

use cfml_common::dynamic::CfmlValue;
use cfml_common::vm::CfmlResult;
use std::collections::HashMap;

/// Tells the engine whether a function is called per-row (scalar) or
/// per-partition (aggregate).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QoQFnKind {
    /// Called once per row with the per-row argument values.
    /// Signature: `fn([arg1, arg2, …]) -> CfmlResult`.
    Scalar,
    /// Called once per partition. Each SQL argument is delivered as a
    /// `CfmlValue::Array` of that argument's value across every row in the
    /// partition. E.g. `SUM(salary)` receives `[Array([100, 200, 300])]`.
    Aggregate,
}

/// A native QoQ function pointer — same shape as a stdlib `BuiltinFunction`.
pub type QoQFn = fn(Vec<CfmlValue>) -> CfmlResult;

/// A SQL function whose implementation cannot be a bare `fn` pointer.
///
/// A dynamically loaded extension's function has to carry "which module, which
/// entry point" and be handed a `ctx`, none of which fits in a `fn` — the same
/// reason foreign BIFs need their own registry rather than living in
/// `builtins`. The host wraps its dispatch in a closure so this crate needs no
/// knowledge of the extension ABI at all.
pub type DynamicQoQFn = std::sync::Arc<dyn Fn(Vec<CfmlValue>) -> CfmlResult + Send + Sync>;

/// Holds the functions available inside QoQ SQL: native scalar/aggregate
/// functions (registered from Rust) and CFML UDFs/closures (registered at
/// runtime via `queryRegisterFunction`).
#[derive(Default)]
pub struct QoQFunctionRegistry {
    /// Native scalar functions, keyed by lowercase name.
    pub scalars: HashMap<String, QoQFn>,
    /// Native aggregate functions, keyed by lowercase name.
    pub aggregates: HashMap<String, QoQFn>,
    /// CFML UDFs/closures, keyed by lowercase name, with their kind. The stored
    /// `CfmlValue` is a `Function`/`Closure` invoked through the VM callback.
    pub customs: HashMap<String, (CfmlValue, QoQFnKind)>,
    /// Functions provided by a loaded native extension, keyed by lowercase
    /// name. Held as closures so this crate stays unaware of the extension ABI.
    pub dynamics: HashMap<String, (DynamicQoQFn, QoQFnKind)>,
}

impl std::fmt::Debug for QoQFunctionRegistry {
    /// Hand-written because a closure has no `Debug`. Counts are what anyone
    /// debugging a registry actually wants anyway.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("QoQFunctionRegistry")
            .field("scalars", &self.scalars.len())
            .field("aggregates", &self.aggregates.len())
            .field("customs", &self.customs.len())
            .field("dynamics", &self.dynamics.len())
            .finish()
    }
}

impl QoQFunctionRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a native scalar or aggregate function.
    pub fn register_native(&mut self, name: &str, func: QoQFn, kind: QoQFnKind) {
        let key = name.to_lowercase();
        match kind {
            QoQFnKind::Scalar => {
                self.scalars.insert(key, func);
            }
            QoQFnKind::Aggregate => {
                self.aggregates.insert(key, func);
            }
        }
    }

    /// Register a function whose implementation is not a bare `fn` — currently
    /// a native extension's.
    pub fn register_dynamic(&mut self, name: &str, func: DynamicQoQFn, kind: QoQFnKind) {
        self.dynamics.insert(name.to_lowercase(), (func, kind));
    }

    /// Look up an extension-provided function of the right kind.
    pub fn get_dynamic(&self, name: &str, kind: QoQFnKind) -> Option<DynamicQoQFn> {
        self.dynamics
            .get(&name.to_lowercase())
            .filter(|(_, k)| *k == kind)
            .map(|(f, _)| f.clone())
    }

    /// Register a CFML UDF/closure under `name`. `kind` defaults to scalar at
    /// the call site if the caller doesn't know better.
    pub fn register_custom(&mut self, name: &str, func: CfmlValue, kind: QoQFnKind) {
        self.customs.insert(name.to_lowercase(), (func, kind));
    }

    /// Look up a native function, returning its kind and pointer.
    pub fn get_native(&self, name: &str) -> Option<(QoQFnKind, QoQFn)> {
        let key = name.to_lowercase();
        if let Some(&f) = self.scalars.get(&key) {
            return Some((QoQFnKind::Scalar, f));
        }
        if let Some(&f) = self.aggregates.get(&key) {
            return Some((QoQFnKind::Aggregate, f));
        }
        None
    }

    /// Look up a custom CFML function (the value + its kind).
    pub fn get_custom(&self, name: &str) -> Option<&(CfmlValue, QoQFnKind)> {
        self.customs.get(&name.to_lowercase())
    }

    /// Is `name` an aggregate (native or custom)?
    pub fn is_aggregate(&self, name: &str) -> bool {
        let key = name.to_lowercase();
        self.aggregates.contains_key(&key)
            || matches!(self.customs.get(&key), Some((_, QoQFnKind::Aggregate)))
            || matches!(self.dynamics.get(&key), Some((_, QoQFnKind::Aggregate)))
    }

    /// Is `name` registered at all (native or custom)?
    pub fn contains(&self, name: &str) -> bool {
        let key = name.to_lowercase();
        self.scalars.contains_key(&key)
            || self.aggregates.contains_key(&key)
            || self.customs.contains_key(&key)
            || self.dynamics.contains_key(&key)
    }
}
