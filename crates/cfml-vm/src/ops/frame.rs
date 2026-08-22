//! Frame-state handlers that need exactly one piece of the frame — the `locals`
//! map or the slot vector — plus, for the two path-writing ops, the frame's
//! `effective_local_mode_modern` flag. Extracting these first keeps the parameter
//! lists honest: a `FrameCtx` struct only earns its place once an op genuinely
//! touches most of the frame, which is true of the call/store ops but not these.
//!
//! Bodies moved verbatim (roadmap P3 slice 4); see `super` for the rules.

use crate::CfmlVirtualMachine;
use cfml_common::dynamic::{CfmlValue, ValueMap};
use cfml_common::name::Name;
use std::sync::Arc;

/// `IsDefined`
#[inline]
pub(crate) fn op_is_defined(
    vm: &mut CfmlVirtualMachine,
    stack: &mut Vec<CfmlValue>,
    locals: &ValueMap,
    var_name: &Name,
) {
    let defined = vm.is_variable_defined(&var_name, &locals);
    stack.push(CfmlValue::Bool(defined));
}

/// `LoadStaticHolder`
#[inline]
pub(crate) fn op_load_static_holder(
    vm: &mut CfmlVirtualMachine,
    stack: &mut Vec<CfmlValue>,
    locals: &ValueMap,
    name: &Name,
) {
    let holder = vm
        .resolve_static_holder(name, &locals)
        .unwrap_or(CfmlValue::Null);
    stack.push(holder);
}

/// `SetLastExceptionFromLocal`
#[inline]
pub(crate) fn op_set_last_exception_from_local(
    vm: &mut CfmlVirtualMachine,
    locals: &ValueMap,
    name: &Name,
) {
    // GH #244: re-point `last_exception` at the exception bound to
    // the enclosing catch clause's variable (the full cfcatch
    // struct) so the following Rethrow re-raises it, not an
    // already-handled inner exception left in the register by a
    // nested try/catch. A missing local (shouldn't happen for a
    // real catch var) leaves the register untouched.
    if let Some(v) = vm.lookup_name_in_scopes(name, name.lower(), &locals) {
        if !matches!(v, CfmlValue::Null) {
            vm.last_exception = Some(v);
        }
    }
}

/// `DeleteScopeKey`
#[inline]
pub(crate) fn op_delete_scope_key(
    vm: &mut CfmlVirtualMachine,
    stack: &mut Vec<CfmlValue>,
    locals: &mut ValueMap,
    effective_local_mode_modern: bool,
    scope: &Name,
) {
    // `StructDelete(<scope>, keyExpr)` — pop the key and delete
    // `<scope>.<key>` from the live scope container (scopes are
    // snapshotted when passed as a builtin arg, so an in-place
    // struct mutation wouldn't reach them).
    let key = stack.pop().unwrap_or(CfmlValue::Null).as_string();
    let path = format!("{}.{}", scope, key);
    vm.delete_scope_path(&path, locals, effective_local_mode_modern);
}

/// `SetDynamicVar`
#[inline]
pub(crate) fn op_set_dynamic_var(
    vm: &mut CfmlVirtualMachine,
    stack: &mut Vec<CfmlValue>,
    locals: &mut ValueMap,
    effective_local_mode_modern: bool,
) {
    // Dynamic/quoted-string LHS assignment: the path string was
    // resolved at runtime (e.g. "variables.propDep" from
    // `"#scope#.#prop#" = v`). Store scope-aware into the current
    // frame so `variables.x` lands in a CFC's __variables (not the
    // page scope) — matching a normal `variables.x = v` assignment.
    let value = stack.pop().unwrap_or(CfmlValue::Null);
    let path = stack
        .pop()
        .map(|v| v.as_string())
        .unwrap_or_default();
    vm.store_runtime_path(&path, value.clone(), locals, effective_local_mode_modern);
    stack.push(value);
}

/// `MarkAccessorPrivate`
#[inline]
pub(crate) fn op_mark_accessor_private(
    locals: &ValueMap,
    name: &Name,
) {
    // Emitted at the tail of a generated `setX()` accessor. Record
    // the property on the frame's `this` so introspection/for-in
    // hide it (Lucee keeps accessor values private in `variables`);
    // getX()/serializeJSON still read the top-level value. Persists
    // to the receiver the same way the setter's value write does.
    if let Some(CfmlValue::Struct(this_s)) = locals.get(&*cfml_common::key::well_known::THIS) {
        CfmlVirtualMachine::mark_accessor_private(this_s, name);
    }
    // Flyweight instance: record on the instance's accessor-private set
    // (the marker's `__cfml_accessor_private__` analogue) so a runtime
    // `setX()` after construction hides the property from introspection.
    #[cfg(feature = "component-instance")]
    if let Some(CfmlValue::Instance(inst)) = locals.get(&*cfml_common::key::well_known::THIS) {
        inst.read()
            .accessor_private
            .write()
            .insert(name.to_ascii_lowercase());
    }
}

/// `SetIndex`
#[inline]
pub(crate) fn op_set_index(
    stack: &mut Vec<CfmlValue>,
) {
    let index = stack.pop().unwrap_or(CfmlValue::Null);
    let mut collection = stack.pop().unwrap_or(CfmlValue::Null);
    let value = stack.pop().unwrap_or(CfmlValue::Null);
    match &mut collection {
        CfmlValue::Array(arr) => {
            // 1-based index; accept Int or numeric Double/String.
            let one_based: i64 = match &index {
                CfmlValue::Int(i) => *i,
                CfmlValue::Double(d) => *d as i64,
                other => other.as_string().trim().parse::<i64>().unwrap_or(0),
            };
            if one_based >= 1 {
                let idx = (one_based - 1) as usize;
                // Interior mutability on the shared handle: the
                // assignment is visible to every alias. Auto-grow
                // past the end leaves skipped slots as null holes
                // (Lucee): `a=[]; a[3]="x"` → len 3, [1]/[2] null.
                arr.set_or_grow(idx, value);
            }
        }
        CfmlValue::Struct(s) => {
            // §3.5: the key is inserted (owned is genuinely needed),
            // but `index` was just popped off the stack, so move the
            // backing String out rather than copying its contents.
            let key = index.into_string();
            // Propagate to __variables for declared CFC properties
            if s.contains_key(&*cfml_common::key::well_known::VARIABLES) && s.contains_key(&*cfml_common::key::well_known::PROPERTIES) {
                let key_lower = key.to_lowercase();
                let is_declared =
                    if let Some(CfmlValue::Array(props)) = s.get(&*cfml_common::key::well_known::PROPERTIES) {
                        props.iter().any(|p| {
                            if let CfmlValue::Struct(ps) = p {
                                ps.iter().any(|(k, v)| {
                                    k.to_lowercase() == "name"
                                        && v.as_string().to_lowercase() == key_lower
                                })
                            } else {
                                false
                            }
                        })
                    } else {
                        false
                    };
                if is_declared {
                    if let Some(CfmlValue::Struct(vars)) = s.get(&*cfml_common::key::well_known::VARIABLES) {
                        vars.insert(key.clone(), value.clone());
                    }
                }
            }
            s.insert(key, value);
        }
        CfmlValue::Null => {
            // Auto-vivification: subscript-assigning into a variable
            // (or member) that does not yet exist creates it, matching
            // Lucee/ACF/BoxLang. A genuine numeric index creates an
            // Array; any other key creates a Struct. e.g.
            // `this.mappings["/app"] = x` where this.mappings is unset.
            let numeric_idx = match &index {
                CfmlValue::Int(i) => Some(*i),
                CfmlValue::Double(d) => Some(*d as i64),
                _ => None,
            };
            if let Some(i) = numeric_idx.filter(|i| *i >= 1) {
                let arr = cfml_common::dynamic::CfmlArray::empty();
                arr.set_or_grow((i - 1) as usize, value);
                collection = CfmlValue::Array(arr);
            } else {
                let mut s = ValueMap::default();
                s.insert(index.as_string(), value);
                collection = CfmlValue::strukt(s);
            }
        }
        CfmlValue::QueryColumn(arc, _) => {
            // Inner step of `q[col][row] = v`: GetIndex(query, col)
            // produced this column proxy; assign the 1-based row cell.
            // The column CoW-detaches here; the outer SetIndex's
            // Query arm writes the whole modified column back.
            let one_based: i64 = match &index {
                CfmlValue::Int(i) => *i,
                CfmlValue::Double(d) => *d as i64,
                other => other.as_string().trim().parse::<i64>().unwrap_or(0),
            };
            if one_based >= 1 {
                let idx = (one_based - 1) as usize;
                let col = Arc::make_mut(arc);
                if idx >= col.len() {
                    col.resize(idx + 1, CfmlValue::Null);
                }
                col[idx] = value;
            }
        }
        CfmlValue::Query(q) => {
            // Outer step of `q[col][row] = v`, or a whole-column
            // assign `q[col] = arrayOrColumn`. Replace the named
            // column in place on the shared query so every alias
            // (e.g. the cached query in request scope) observes it.
            let col_name = index.as_string();
            let new_values: Vec<CfmlValue> = match value {
                CfmlValue::QueryColumn(a, _) => a.as_ref().clone(),
                CfmlValue::Array(a) => a.snapshot(),
                other => vec![other],
            };
            q.set_column(&col_name, new_values);
        }
        // Phase C.3 — Slice 3: `instance["key"] = v` writes the public
        // DATA map in place (shared Arc → persists on the instance).
        #[cfg(feature = "component-instance")]
        CfmlValue::Instance(inst) => {
            inst.read().set_public_member(index.as_string(), value);
        }
        _ => {}
    }
    stack.push(collection);
}

