//! Property and index access handlers: `GetProperty`/`TryGetProperty`,
//! `GetIndex`, `SetProperty`, `GetKeys`. These are among the hottest ops in real
//! CFML — every `x.y`, `a[i]` and `for (k in struct)` — and the four arms
//! together were ~694 lines of the dispatch match.
//!
//! Bodies moved verbatim (roadmap P3 slice 3); see `super` for the rules and
//! `super::effect` for how `continue` / `return Err` translate. One deliberate
//! change: `GetProperty` and `TryGetProperty` shared an arm and discriminated
//! with `matches!(op, BytecodeOp::GetProperty(_))`. That line is gone and the
//! result arrives as the `throw_on_miss` parameter, computed at the call site,
//! so the handler needs no access to the raw op.

use crate::{is_component_struct, CfmlVirtualMachine};
use std::sync::Arc;
use cfml_common::dynamic::{CfmlValue, ValueMap};
use cfml_common::name::Name;
use cfml_common::vm::CfmlError;

#[inline]
pub(crate) fn op_get_property(
    vm: &mut CfmlVirtualMachine,
    stack: &mut Vec<CfmlValue>,
    ip: &mut usize,
    name: &Name,
    throw_on_miss: bool,
) -> Result<(), CfmlError> {
    // GetProperty throws "Variable '<name>' is undefined" on a
    // genuine struct/component member miss (Lucee/ACF parity);
    // TryGetProperty reads the miss as Null (tolerant contexts).
    if let Some(obj) = stack.pop() {
        match &obj {
            CfmlValue::Struct(s) => {
                // v0.599 — ONE probe. `name` is an interned bytecode operand
                // carrying its precomputed key hash, and the map itself folds
                // case, so the old exact → UPPER → lower → full-scan ladder
                // (four probes plus a `to_uppercase()` allocation per miss —
                // 6.3% of all keyed lookups on a warm Preside render) collapses
                // into it. Same answer: every rung differed only in the casing
                // it tried.
                let val = s.get(name).or_else(|| {
                    // Fall back to __variables for component properties.
                    if let Some(CfmlValue::Struct(vars)) =
                        s.get(&*cfml_common::key::well_known::VARIABLES)
                    {
                        vars.get(name)
                    } else {
                        None
                    }
                });
                let val = match val {
                    Some(v) => {
                        // A component method extracted as a VALUE
                        // (e.g. `var f = this.method;` or
                        // `filter=this._filterServices`) must carry
                        // its owning component's scope so that when
                        // it is later invoked elsewhere — bare, or
                        // via another struct member like
                        // `arguments.filter(x)` — its unscoped
                        // references still resolve against the
                        // component's `variables`/`this` (ColdBox
                        // Binder.mapDirectory passes a bound method
                        // as its `filter`). Mirrors the bare-name
                        // bind (Bug #9). Only bind genuine, unbound
                        // component methods on a component receiver.
                        if let CfmlValue::Function(ref f) = v {
                            if f.captured_scope.is_none()
                                && (s.contains_key(&*cfml_common::key::well_known::VARIABLES)
                                    || s.contains_key("__name"))
                            {
                                let mut bound: ValueMap = ValueMap::default();
                                bound.insert("this".to_string(), obj.clone());
                                if let Some(vars) = s.get(&*cfml_common::key::well_known::VARIABLES) {
                                    bound.insert("__variables".to_string(), vars.clone());
                                    bound.insert("variables".to_string(), vars.clone());
                                }
                                if let Some(sup) = s.get(&*cfml_common::key::well_known::SUPER_NATIVE) {
                                    bound.insert("super".to_string(), sup.clone());
                                }
                                let mut bound_fn = (**f).clone();
                                bound_fn.captured_scope =
                                    Some(cfml_common::cycle_gc::tracked_scope(bound));
                                CfmlValue::Function(Arc::new(bound_fn))
                            } else {
                                v
                            }
                        } else {
                            v
                        }
                    }
                    None => {
                        // Fall through to a Rust-backed parent if attached.
                        if let Some(CfmlValue::NativeObject(parent)) =
                            s.get(&*cfml_common::key::well_known::SUPER_NATIVE)
                        {
                            if let Ok(guard) = parent.read() {
                                guard.get_property(name).unwrap_or(CfmlValue::Null)
                            } else {
                                CfmlValue::Null
                            }
                        } else if s.contains_key(
                            cfml_common::dynamic::EMPTY_DEFAULT_SCOPE_MARKER,
                        ) {
                            // Magic scope (cgi): unset key reads as "".
                            CfmlValue::string(String::new())
                        } else if throw_on_miss
                            && !CfmlVirtualMachine::is_declared_arg_param(&obj, name)
                        {
                            // Genuine member miss on a struct/component:
                            // throw a catchable "Variable is undefined".
                            // A declared-but-unpassed `arguments` param
                            // is exempt — it reads as Null (Lucee/ACF).
                            let cip =
                                vm.raise_undefined_member(name, stack)?;
                            *ip = cip;
                            return Ok(());
                        } else {
                            CfmlValue::Null
                        }
                    }
                };
                stack.push(val);
            }
            CfmlValue::Array(arr) => {
                // Array member functions
                match name.lower() {
                    "len" | "length" => {
                        stack.push(CfmlValue::Int(arr.len() as i64));
                    }
                    // An XML named-child group reads its members off the FIRST
                    // element — `x.Root.Kid.xmlText` (GH #343). Anything that is
                    // not a node group keeps returning Null.
                    _ => match crate::xml_group_first(arr) {
                        Some(first) => {
                            let v = CfmlVirtualMachine::lookup_property(&first, name.as_str());
                            stack.push(v);
                        }
                        None => stack.push(CfmlValue::Null),
                    },
                }
            }
            CfmlValue::String(s) => {
                // String member functions
                match name.lower() {
                    "len" | "length" => {
                        // CFML len() is a character count (see fn_len).
                        stack.push(CfmlValue::Int(s.chars().count() as i64));
                    }
                    _ => stack.push(CfmlValue::Null),
                }
            }
            CfmlValue::Query(q) => {
                match name.lower() {
                    "recordcount" => {
                        stack.push(CfmlValue::Int(q.row_count() as i64));
                    }
                    "columnlist" => {
                        // Uppercase column names, matching Lucee/ACF columnList.
                        stack.push(CfmlValue::string(q.column_list()));
                    }
                    "currentrow" => {
                        stack.push(CfmlValue::Int(q.current_row() as i64));
                    }
                    _ => {
                        // Column access: q.columnName returns a QueryColumn
                        // proxy — acts as Array for indexing/iteration/length,
                        // but stringifies to the query's current row (Lucee parity).
                        if let Some(col_data) = q.column_values_ci(name) {
                            stack.push(CfmlValue::QueryColumn(col_data, q.current_row().saturating_sub(1)));
                        } else {
                            stack.push(CfmlValue::Null);
                        }
                    }
                }
            }
            // A Rust-backed object exposes properties through the
            // `CfmlNative::get_property` hook (e.g. the live
            // `socket.data` struct). Property access (`obj.x`, no
            // call) never reaches `call_member_function`, so this
            // is the only place the trait getter is consulted —
            // without it `socket.data` read as Null and the
            // documented live-state write silently vanished.
            CfmlValue::NativeObject(o) => {
                let val = o
                    .read()
                    .ok()
                    .and_then(|g| g.get_property(&name))
                    .unwrap_or(CfmlValue::Null);
                stack.push(val);
            }
            // Phase C.3 — Slice 3: flyweight instance member read.
            // Public then private DATA (method-table aware); a method
            // extracted as a VALUE is bound to this instance's scope so
            // a later bare/foreign call resolves correctly (mirrors the
            // Struct arm). Throwing on a genuine miss matches Lucee.
            #[cfg(feature = "component-instance")]
            CfmlValue::Instance(inst) => {
                let val = match inst.read().get_member(name) {
                    Some(v) => {
                        if let CfmlValue::Function(ref f) = v {
                            if f.captured_scope.is_none() {
                                let g = inst.read();
                                let vars = CfmlValue::Struct(
                                    g.private_map_handle(),
                                );
                                let mut bound: ValueMap = ValueMap::default();
                                bound.insert("this".to_string(), obj.clone());
                                bound.insert(
                                    "__variables".to_string(),
                                    vars.clone(),
                                );
                                bound.insert("variables".to_string(), vars);
                                let mut bound_fn = (**f).clone();
                                bound_fn.captured_scope = Some(
                                    cfml_common::cycle_gc::tracked_scope(bound),
                                );
                                CfmlValue::Function(Arc::new(bound_fn))
                            } else {
                                v
                            }
                        } else {
                            v
                        }
                    }
                    None => {
                        if throw_on_miss {
                            let cip =
                                vm.raise_undefined_member(name, stack)?;
                            *ip = cip;
                            return Ok(());
                        } else {
                            CfmlValue::Null
                        }
                    }
                };
                stack.push(val);
            }
            _ => {
                stack.push(obj.get(&name).unwrap_or(CfmlValue::Null));
            }
        }
    } else {
        stack.push(CfmlValue::Null);
    }
    Ok(())
}

#[inline]
pub(crate) fn op_get_index(
    vm: &mut CfmlVirtualMachine,
    stack: &mut Vec<CfmlValue>,
    ip: &mut usize,
) -> Result<(), CfmlError> {
    let index = stack.pop().unwrap_or(CfmlValue::Null);
    let collection = stack.pop().unwrap_or(CfmlValue::Null);
    let one_based_to_zero = |index: &CfmlValue| -> usize {
        let idx = match index {
            CfmlValue::Int(i) => *i as usize,
            CfmlValue::Double(d) => *d as usize,
            CfmlValue::String(s) => s.parse::<usize>().unwrap_or(0),
            _ => 0,
        };
        if idx > 0 { idx - 1 } else { 0 }
    };
    match &collection {
        CfmlValue::Array(arr) => {
            let idx = one_based_to_zero(&index);
            stack.push(arr.get(idx).unwrap_or(CfmlValue::Null));
        }
        CfmlValue::QueryColumn(arr, _) => {
            let idx = one_based_to_zero(&index);
            stack.push(arr.get(idx).cloned().unwrap_or(CfmlValue::Null));
        }
        // Lucee/ACF/BoxLang parity: q["colName"] returns the column
        // proxy (same as q.colName via GetProperty); q[N] returns
        // row N as a struct. Frameworks need the bracket form for
        // dynamic column names (e.g. Wheels' ORM column processing).
        CfmlValue::Query(q) => {
            let row_at_oneless = |n: i64| -> CfmlValue {
                if n >= 1 {
                    q.get_row((n - 1) as usize)
                        .map(|m| CfmlValue::Struct(cfml_common::dynamic::CfmlStruct::new(m)))
                        .unwrap_or(CfmlValue::Null)
                } else {
                    CfmlValue::Null
                }
            };
            match &index {
                CfmlValue::String(name) => {
                    if let Some(col_data) = q.column_values_ci(name.as_str()) {
                        stack.push(CfmlValue::QueryColumn(col_data, q.current_row().saturating_sub(1)));
                    } else if name.eq_ignore_ascii_case("currentrow") {
                        stack.push(CfmlValue::Int(q.current_row() as i64));
                    } else if let Ok(n) = name.trim().parse::<i64>() {
                        stack.push(row_at_oneless(n));
                    } else {
                        stack.push(CfmlValue::Null);
                    }
                }
                CfmlValue::Int(n) => stack.push(row_at_oneless(*n)),
                CfmlValue::Double(d) => stack.push(row_at_oneless(*d as i64)),
                _ => stack.push(CfmlValue::Null),
            }
        }
        CfmlValue::Struct(s) => {
            // §3.5: the key is only ever READ here (lookup, case
            // folds, numeric parse), so borrow it when `index` is
            // already a string instead of deep-copying the contents.
            // Hottest `as_string` site on a warm Preside page
            // (3,764 calls/request).
            let key = index.as_str_cow();
            let direct = s
                .get(&key)
                .or_else(|| s.get(&key.to_uppercase()))
                .or_else(|| s.get(&key.to_lowercase()))
                .or_else(|| {
                    // Pure key compare + one value clone, so read
                    // UNDER the lock. `iter()` here is `snapshot()`:
                    // it cloned the whole struct to find one key, and
                    // on a warm Preside page this single site copied
                    // ~half of all snapshotted entries (mean map ~222
                    // entries, ~67 calls/request) — the largest
                    // allocation owner in the VM. `GetProperty`'s
                    // sibling CI scan was fixed this way in v0.566.0;
                    // the bracket-access path was missed.
                    let key_lower = key.to_lowercase();
                    s.with_map(|m| {
                        m.iter()
                            .find(|(k, _)| k.eq_ignore_ascii_case(&key_lower))
                            .map(|(_, v)| v.clone())
                    })
                });
            // Arguments-scope positional fallback: when the
            // index is numeric N (1-based) and there's no
            // direct key match, resolve via the declared
            // param name at position N-1. A value bound to
            // a declared param lives under its name, not
            // under the numeric alias.
            let val = if direct.is_none()
                && s.contains_key("__arguments_scope")
            {
                if let Ok(n) = key.parse::<i64>() {
                    if n >= 1 {
                        let idx = (n - 1) as usize;
                        // First try the declared-param name at
                        // position N-1 (named call to a fn with
                        // declared params: value lives under the
                        // param name).
                        let by_param = if let Some(CfmlValue::Array(params)) =
                            s.get("__arguments_params")
                        {
                            params
                                .get(idx)
                                .map(|p| p.as_string())
                                .and_then(|name| s.get(&name))
                        } else {
                            None
                        };
                        // Fall through to the N-th non-marker entry's
                        // value in insertion order. Lucee/ACF: the
                        // arguments scope is array-addressable for
                        // named calls too — `arguments[1]` reads the
                        // first bound arg even when the callee declares
                        // no params (the Wheels $set() shape).
                        by_param.unwrap_or_else(|| {
                            // Same pure read as above — positional
                            // scan, one value cloned out. Was another
                            // whole-struct `snapshot()` per lookup.
                            s.with_map(|m| {
                                m.iter()
                                    .filter(|(k, _)| {
                                        k.as_str() != "__arguments_scope"
                                            && k.as_str() != "__arguments_params"
                                    })
                                    .nth(idx)
                                    .map(|(_, v)| v.clone())
                                    .unwrap_or(CfmlValue::Null)
                            })
                        })
                    } else {
                        CfmlValue::Null
                    }
                } else {
                    CfmlValue::Null
                }
            } else if direct.is_none()
                && s.contains_key(
                    cfml_common::dynamic::EMPTY_DEFAULT_SCOPE_MARKER,
                )
            {
                // Magic scope (cgi): unset key reads as "".
                CfmlValue::string(String::new())
            } else {
                direct.unwrap_or(CfmlValue::Null)
            };
            // `this[ name ]` extracts a component method as a bare
            // VALUE — it is NOT bound to `s` here. Binding is a
            // call-site decision: an immediate `obj[name]()` goes
            // through CallComputedMethod (binds the receiver), and
            // an extracted-then-invoked method binds to whatever
            // component it is called through (member dispatch) or
            // the caller's component (a bare/plain-struct call) —
            // matching Lucee mixin semantics. Eager-binding here
            // froze the SOURCE scope and survived re-homing the
            // method onto another component, breaking every Wheels
            // boot (#220).
            stack.push(val);
        }
        // Phase C.3 — Slice 3: `instance["key"]` reads public then
        // private DATA (tolerant — Null on miss, like struct index).
        #[cfg(feature = "component-instance")]
        CfmlValue::Instance(inst) => {
            let key = index.as_str_cow();
            stack.push(
                inst.read().get_member(&key).unwrap_or(CfmlValue::Null),
            );
        }
        // Lucee/ACF/BoxLang: `str[n]` is 1-based CHARACTER access
        // (equivalent to Mid(str, n, 1)). An out-of-range, zero or
        // non-numeric subscript throws — matching Lucee's
        // "there is no property with name [n] found in [string]".
        // (Preside EmailService validates `sendArgs.to[1]` where
        // `to` is a single-recipient string.)
        CfmlValue::String(s) => {
            let n = match &index {
                CfmlValue::Int(i) => Some(*i),
                CfmlValue::Double(d) => Some(*d as i64),
                CfmlValue::String(is) => is.trim().parse::<i64>().ok(),
                _ => None,
            };
            let ch = match n {
                Some(n) if n >= 1 => s.chars().nth((n - 1) as usize),
                _ => None,
            };
            match ch {
                Some(ch) => stack.push(CfmlValue::string(ch.to_string())),
                None => {
                    // Out-of-range / zero / non-numeric subscript —
                    // catchable, matching Lucee's message + type.
                    // Two spaces before "found" mirrors Lucee's
                    // exact message verbatim.
                    let msg = format!(
                        "there is no property with name [{}]  found in [string]",
                        index.as_string()
                    );
                    match vm.raise_catchable(stack, &msg, "expression") {
                        Ok(catch_ip) => {
                            *ip = catch_ip;
                            return Ok(());
                        }
                        Err(e) => return Err(e),
                    }
                }
            }
        }
        _ => stack.push(CfmlValue::Null),
    }
    Ok(())
}

#[inline]
pub(crate) fn op_set_property(
    stack: &mut Vec<CfmlValue>,
    name: &Name,
) -> Result<(), CfmlError> {
    if let Some(value) = stack.pop() {
        if let Some(mut obj) = stack.pop() {
            // Auto-vivification: setting a property on a base that
            // does not yet exist (`q.lineData = v` reached via a
            // nested-write-back where `q` is undefined) creates a
            // struct, matching SetIndex's Null arm and Lucee/ACF/
            // BoxLang — otherwise the write silently vanished into
            // a Null receiver, leaving the root Null.
            if matches!(obj, CfmlValue::Null) {
                obj = CfmlValue::strukt(ValueMap::default());
            }
            // Phase C.3 — Slice 3: `instance.x = v` writes the public
            // DATA map in place (shared Arc → persists on the instance).
            // A CFC with a `rust:` native parent routes writes the
            // native side recognises to `set_property` first (Rust
            // state stays first-class); a None return defers to the
            // CFC public map — mirrors the marker Struct arm below.
            #[cfg(feature = "component-instance")]
            if let CfmlValue::Instance(ref inst) = obj {
                let handled = {
                    let g = inst.read();
                    if let Some(CfmlValue::NativeObject(parent)) = &g.native_parent {
                        let mut guard = parent.write().map_err(|_| {
                            CfmlError::runtime(
                                "NativeObject lock poisoned".to_string(),
                            )
                        })?;
                        guard.set_property(name, value.clone())
                    } else {
                        None
                    }
                };
                match handled {
                    Some(result) => result?,
                    None => {
                        inst.read().set_public_member(name.to_string(), value);
                    }
                }
                stack.push(obj);
                return Ok(());
            }
            // CFC with a Rust-backed parent: route writes the
            // native side recognises before touching the CFC
            // struct, so Rust state stays first-class. The
            // parent returns None to defer to the CFC.
            if let CfmlValue::Struct(ref s) = obj {
                if let Some(CfmlValue::NativeObject(parent)) =
                    s.get(&*cfml_common::key::well_known::SUPER_NATIVE)
                {
                    let handled = {
                        let mut guard = parent.write().map_err(|_| {
                            CfmlError::runtime(
                                "NativeObject lock poisoned".to_string(),
                            )
                        })?;
                        guard.set_property(name, value.clone())
                    };
                    if let Some(result) = handled {
                        result?;
                        stack.push(obj);
                        return Ok(());
                    }
                }
            }
            // If setting on a CFC struct with declared properties,
            // also update __variables for properties declared via
            // `property name="x"` so they're accessible unscoped in methods.
            if let Some(s) = obj.as_cfml_struct() {
                if s.contains_key(&*cfml_common::key::well_known::VARIABLES) && s.contains_key(&*cfml_common::key::well_known::PROPERTIES) {
                    let name_lower = name.lower();
                    let is_declared = if let Some(CfmlValue::Array(props)) =
                        s.get(&*cfml_common::key::well_known::PROPERTIES)
                    {
                        props.iter().any(|p| {
                            if let CfmlValue::Struct(ps) = p {
                                ps.iter().any(|(k, v)| {
                                    k.to_lowercase() == "name"
                                        && v.as_string().to_lowercase()
                                            == name_lower
                                })
                            } else {
                                false
                            }
                        })
                    } else {
                        false
                    };
                    if is_declared {
                        if let Some(CfmlValue::Struct(vars)) =
                            s.get(&*cfml_common::key::well_known::VARIABLES)
                        {
                            vars.insert(name, value.clone());
                        }
                    }
                }
            }
            obj.set(name.to_string(), value);
            stack.push(obj);
        }
    }
    Ok(())
}

#[inline]
pub(crate) fn op_get_keys(
    stack: &mut Vec<CfmlValue>,
) {
    // For for-in: convert struct to array of keys, leave arrays unchanged
    if let Some(val) = stack.pop() {
        match val {
            CfmlValue::Struct(s) => {
                // Hide the private arguments-scope markers
                // from for-in iteration. Real numeric keys
                // (overflow positional args) still surface,
                // matching Lucee.
                let is_args = s.contains_key("__arguments_scope");
                // A struct carrying CFC instance markers is a
                // component instance (this engine materialises
                // CFCs as marker-bearing structs). Lucee/ACF
                // for-in over a component iterates its `this`
                // scope: public data members AND public methods
                // (UDFs) — never engine-internal `__` keys, the
                // `this` scope itself, or PRIVATE/PACKAGE methods.
                // (WireBox virtual inheritance relies on the
                // public-method enumeration to mix in a base
                // class's methods — `toVirtualInheritance`.)
                let is_cfc = is_component_struct(&s);
                // Accessor-private property names (set via implicit
                // ctor / generated setX) — hidden from for-in to
                // match Lucee's private `variables` storage.
                let accessor_private = if is_cfc {
                    match s.get(cfml_common::dynamic::ACCESSOR_PRIVATE_MARKER) {
                        Some(CfmlValue::Struct(m)) => Some(m.clone()),
                        _ => None,
                    }
                } else {
                    None
                };
                // A Java-collection shim (LinkedHashMap etc.) is a
                // transparent map — hide its `__java_*` markers.
                let is_java_shim = s.contains_key("__java_shim");
                // A java.util.TreeMap iterates its keys in SORTED
                // (natural) order, unlike an insertion-ordered
                // HashMap/struct. The shim stores entries in
                // insertion order, so for-in must sort. MockBox's
                // normalizeArguments() relies on this to build an
                // argument-order-independent hash (`for(k in
                // treeMap)`); without it the hash depends on
                // call-site arg order, mocks fail to match, and
                // `var x = mock()` assigns null -> downstream
                // "Variable undefined".
                let is_treemap = s
                    .get("__java_class")
                    .map(|v| v.as_string().eq_ignore_ascii_case("java.util.treemap"))
                    .unwrap_or(false);
                let mut keys: Vec<CfmlValue> = s
                    .all_keys()
                    .into_iter()
                    .filter(|k| {
                        !is_args
                            || (k != "__arguments_scope"
                                && k != "__arguments_params")
                    })
                    .filter(|k| !is_java_shim || !k.starts_with("__"))
                    .filter(|k| {
                        k != cfml_common::dynamic::EMPTY_DEFAULT_SCOPE_MARKER
                    })
                    .filter(|k| {
                        if !is_cfc {
                            return true;
                        }
                        // Hide only the EXACT engine-reserved keys,
                        // not every `__`-prefixed key — user/framework
                        // `__`/`___` data members (FW/1 AOP `___orig`)
                        // are real public members Lucee for-in surfaces
                        // (C.4 blanket-filter deletion on marker path).
                        if cfml_common::component::is_reserved_component_key(k)
                            || k.eq_ignore_ascii_case("this")
                        {
                            return false;
                        }
                        // Methods: keep only public/remote ones.
                        // A named UDF carries its access modifier;
                        // private/package methods stay hidden.
                        match s.get(k) {
                            Some(CfmlValue::Function(f)) => matches!(
                                f.access,
                                cfml_common::dynamic::CfmlAccess::Public
                                    | cfml_common::dynamic::CfmlAccess::Remote
                            ),
                            // Non-function data member: visible
                            // UNLESS it is an accessor-private
                            // property (Lucee stores it in the
                            // private `variables` scope). A genuine
                            // public `this.x = …` is never marked.
                            _ => !accessor_private
                                .as_ref()
                                .is_some_and(|m| m.contains_key_ci(k)),
                        }
                    })
                    // Lucee ENUMERATES a null-valued key in struct
                    // for-in (e.g. `for(k in {x=nullValue()})`
                    // yields `x`), even though structKeyExists
                    // reports it absent. The loop body reads the
                    // value defensively (`?:` / structKeyExists), and
                    // RustCFML's null access is lenient ("" rather
                    // than a throw). Query rows store NULL columns as
                    // "" (a real value), so Preside's
                    // `for(field in record){ var v = record[field] }`
                    // is unaffected. (cfflow WorkflowStateSubstitution
                    // needs the null `$something` token enumerated.)
                    .map(CfmlValue::string)
                    .collect();
                if is_treemap {
                    keys.sort_by(|a, b| a.as_string().cmp(&b.as_string()));
                }
                stack.push(CfmlValue::array(keys));
            }
            CfmlValue::String(s) => {
                // Lucee parity: for-in over a string iterates it
                // as a comma-delimited LIST, not characters.
                // Comma is the only delimiter, items are not
                // trimmed, and empty items are KEPT ("a,,b" is
                // 3 items) — unlike ListToArray's default. An
                // empty string never enters the loop.
                let items: Vec<CfmlValue> = if s.is_empty() {
                    Vec::new()
                } else {
                    s.split(',')
                        .map(|item| CfmlValue::string(item.to_string()))
                        .collect()
                };
                stack.push(CfmlValue::array(items));
            }
            CfmlValue::Query(q) => {
                // Iterating over a query: convert to array of row structs
                let rows: Vec<CfmlValue> = q
                    .rows()
                    .into_iter()
                    .map(CfmlValue::strukt)
                    .collect();
                stack.push(CfmlValue::array(rows));
            }
            // Lucee@7 parity: `for (v in q.col)` yields a single
            // element — the stringified current row — because
            // Lucee treats QueryColumn as a string in iter context.
            CfmlValue::QueryColumn(a, row) => {
                let cell = a.get(row).or_else(|| a.first()).cloned().unwrap_or(CfmlValue::Null);
                stack.push(CfmlValue::array(vec![cell]));
            }
            // Phase C.3 — Slice 4: `for (k in instance)` iterates the
            // public scope (data + public methods) read straight from
            // the data maps — no `__` filter.
            #[cfg(feature = "component-instance")]
            CfmlValue::Instance(ref inst) => {
                let keys: Vec<CfmlValue> =
                    cfml_common::component::CompRef::for_instance(inst)
                        .instance_public_keys()
                        .into_iter()
                        .map(CfmlValue::string)
                        .collect();
                stack.push(CfmlValue::array(keys));
            }
            other => stack.push(other), // arrays pass through
        }
    }
}
