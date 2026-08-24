//! Frame ops with explicit frame parameters (roadmap P3 slice 5): the try/except
//! bookkeeping ops, the fused local arithmetic ops, the local load/append ops, and
//! the two super-call ops.
//!
//! These still pass frame state as individual parameters rather than a `FrameCtx`.
//! That stays deliberate while un-extracted arms in the dispatch loop borrow
//! `locals`/`slots` directly — a long-lived context struct cannot coexist with
//! those borrows, and building one per call site would add work to the hot loop.
//! `ArrayAppendLocal` at ten parameters is the signal that the remaining
//! call/store arms should get the struct instead.
//!
//! Ops with a `*Slot*` twin take `op: &BytecodeOp` so the body can pull the slot
//! index out of the variant exactly as it did in the match — keeping the move
//! verbatim, and matching what a Tier-0 shim would hand over anyway.

use crate::{
    cfml_compare, cfml_equal, CfmlVirtualMachine, DeclaredLocals, InheritedKeys, TryHandler,
};
use cfml_codegen::{BytecodeFunction, BytecodeOp, CmpOp};
use cfml_common::dynamic::{CfmlValue, ValueMap};
use cfml_common::name::Name;
use cfml_common::vm::CfmlError;
use std::sync::{Arc, RwLock};

/// `SaveException`
#[inline]
pub(crate) fn op_save_exception(
    vm: &mut CfmlVirtualMachine,
) {
    vm.exception_save_stack.push(vm.last_exception.clone());
}

/// `TryEnd`
#[inline]
pub(crate) fn op_try_end(
    vm: &mut CfmlVirtualMachine,
) {
    vm.try_stack.pop();
}

/// `RestoreException`
#[inline]
pub(crate) fn op_restore_exception(
    vm: &mut CfmlVirtualMachine,
) {
    if let Some(saved) = vm.exception_save_stack.pop() {
        vm.last_exception = saved;
    }
}

/// `TryStart`
#[inline]
pub(crate) fn op_try_start(
    vm: &mut CfmlVirtualMachine,
    stack: &mut Vec<CfmlValue>,
    catch_ip: usize,
) {
    vm.try_stack.push(TryHandler {
        catch_ip: catch_ip,
        stack_depth: stack.len(),
        saved_buffers_depth: vm.saved_output_buffers.len(),
        custom_tag_depth: vm.custom_tag_stack.len(),
        base_tag_depth: vm.base_tag_stack.len(),
    });
}

/// `LineInfo`
#[inline]
pub(crate) fn op_line_info(
    vm: &mut CfmlVirtualMachine,
    line: usize,
    col: usize,
) {
    vm.current_line = line;
    vm.current_column = col;
    // Update the current call frame's line so the stack trace
    // reflects where execution is within this function
    if let Some(frame) = vm.call_stack.last_mut() {
        frame.line = line;
    }
    // Sampling profiler (Phase 2): the whole block vanishes
    // without the feature. When on but the profiler is off for
    // this request, `self.profile` is `None` and this is a single
    // `is_none` branch. When armed, it is one relaxed atomic load
    // that is almost always `false`; only a watchdog-requested
    // sample takes the (out-of-line) snapshot.
    #[cfg(feature = "observability")]
    {
        let want = vm.profile.as_ref().map_or(false, |p| {
            p.want_sample.load(std::sync::atomic::Ordering::Relaxed)
        });
        if want {
            vm.capture_self_sample();
        }
    }
}

/// `StoreGlobal`
#[inline]
pub(crate) fn op_store_global(
    vm: &mut CfmlVirtualMachine,
    stack: &mut Vec<CfmlValue>,
    name: &Name,
) {
    if let Some(val) = stack.pop() {
        vm.globals.insert(name, val);
    }
}

/// `DeclareLocal`
#[inline]
pub(crate) fn op_declare_local(
    declared_locals: &mut DeclaredLocals,
    inherited_or_param_keys: &mut InheritedKeys,
    name: &Name,
) {
    // DeclareSlot shares this body verbatim: declaration only
    // marks the name local + reclaims inherited keys. The slot
    // itself activates at the first StoreSlot (see there) so
    // read-before-first-store keeps today's exact semantics.
    // Mark this variable as function-local (var keyword). ONE entry, in the
    // original casing: `DeclaredLocals` folds case on both insert and probe, so
    // a `var flashPath` is recognised when a later `flashpath = …` is assigned
    // (otherwise the write leaks to `variables` — ColdBox's
    // Router.buildFlashScope), and equally when the write-back loops probe with
    // the casing the CALLER seeded the key under. That third casing is the one
    // the old two-entry `HashSet<String>` could not cover, and is why
    // `var fileName` used to overwrite a caller's `filename`.
    declared_locals.insert(name.as_str());
    // PR #93: a `var x` / `local.x` declaration RECLAIMS the
    // name into THIS frame's `local` scope, shadowing any
    // same-named key inherited from the caller. Removing it
    // from `inherited_or_param_keys` makes the subsequent
    // `local.x` reads return this frame's value.
    //
    // GH #243: CFML is case-insensitive, so `var fileName` must
    // also reclaim an inherited key stored under a DIFFERENT
    // casing (`filename`, carried in from an ancestor's
    // `argumentCollection` overflow arg). Removing only the
    // exact-cased key left the CI variant flagged inherited, so
    // the subsequent `StoreLocal` (via scope_insert_ci) wrote into
    // that still-inherited key and `build_local_scope_view`
    // filtered the write straight back out — the local assignment
    // was silently lost. Drop every CI-matching entry.
    // remove_ci covers the exact match and every CI casing (GH #243).
    inherited_or_param_keys.remove_ci(name.as_str());
}

/// `ValidateParamType`
#[inline]
pub(crate) fn op_validate_param_type(
    vm: &mut CfmlVirtualMachine,
    func: &BytecodeFunction,
    locals: &ValueMap,
    index: usize,
) -> Result<(), CfmlError> {
    // §29 — the default-argument preamble just filled this param
    // from its declared default; Lucee type-checks that value the
    // same as a caller-supplied one (`numeric n = "abc"` throws on
    // an omitted argument). Only reached when the default WAS
    // applied — the op sits inside the JumpIfArgPresent-skipped
    // region — so a supplied argument is never checked twice.
    if let (Some(name), Some(Some(ptype))) =
        (func.params.get(index), func.param_types.get(index))
    {
        let value = locals.get(name).cloned().unwrap_or(CfmlValue::Null);
        let ptype = ptype.clone();
        vm.check_declared_param_type(func, index, name, &ptype, &value)?;
    }
    Ok(())
}

/// `JumpIfArgPresent`
#[inline]
pub(crate) fn op_jump_if_arg_present(
    ip: &mut usize,
    locals: &ValueMap,
    arguments_supplied: &Option<
        std::collections::HashSet<cfml_common::key::Key, cfml_common::key::KeyBuildHasher>,
    >,
    name: &Name,
    target: usize,
) {
    // Default-argument preamble: skip the default when the caller
    // actually supplied this param — i.e. its key already lives in
    // this frame's `arguments` scope. Consults ONLY the arguments
    // scope, never the enclosing scope, so an omitted param whose
    // default reads a same-named outer variable is not shadowed by
    // its own (absent) slot (GitHub #240). No stack traffic.
    //
    // Lever A skip path: when the arguments `CfmlStruct` was not
    // built (unobservable this call), the equivalent supplied-key
    // set is `arguments_supplied` — same GH #240 guarantee (it holds
    // only the params the caller passed, never carried enclosing
    // vars). `contains` uses the pre-lowercased key.
    let supplied = match locals.get(&*cfml_common::key::well_known::ARGUMENTS_SCOPE) {
        Some(CfmlValue::Struct(a)) => a.contains_key_ci(name),
        _ => arguments_supplied
            .as_ref()
            .is_some_and(|s| s.contains(name.key())),
    };
    if supplied {
        *ip = target;
    }
}

/// `SeedArgumentKey`
///
/// Default-parameter preamble tail: the applied default is on the stack (a
/// `Dup` of the value the preceding `StoreLocal`/`StoreSlot` bound to the named
/// local); publish it on the frame's `arguments` scope so `arguments.p` reads
/// the default, exactly as a supplied argument would.
///
/// Replaces `LoadLocal("arguments"); Swap; SetProperty(p); StoreLocal("arguments")`.
/// That `LoadLocal("arguments")` was load-bearing in the worst way: it is what
/// `function_needs_arguments_scope` scans for, so a single defaulted parameter
/// opted every call of the function out of Lever A's lazy `arguments` path —
/// whether or not the default ever fired.
///
/// On a lazy frame there is no `arguments` struct and this is a plain pop. That
/// is not a dropped write: a body that can observe `arguments` at all (by name,
/// through `argumentCollection`, an include, a custom tag, or a string that
/// mentions it) puts the function back on the eager path by construction, so a
/// frame reaching here without a scope has no way to read what we would write.
/// The key is never sought in the CALLER's scope — a frame's `arguments` is its
/// own, and the lazy path drops any inherited handle at frame setup.
#[inline]
pub(crate) fn op_seed_argument_key(
    stack: &mut Vec<CfmlValue>,
    locals: &mut ValueMap,
    name: &Name,
) {
    let value = stack.pop().unwrap_or(CfmlValue::Null);
    if let Some(args) = locals
        .get_mut(&*cfml_common::key::well_known::ARGUMENTS_SCOPE)
        .and_then(|v| v.as_cfml_struct())
    {
        args.insert(name, value);
    }
}

/// `Increment`
#[inline]
pub(crate) fn op_increment(
    locals: &mut ValueMap,
    slots: &mut [Option<CfmlValue>],
    closure_env: &Option<Arc<RwLock<ValueMap>>>,
    op: &BytecodeOp,
    name: &Name,
) -> Result<(), CfmlError> {
    // Slot fast path (T3.1): mutate the active slot in place.
    if let BytecodeOp::IncrementSlot(i, _) = op {
        if let Some(v) = slots[*i as usize].as_mut() {
            *v = match v {
                CfmlValue::Int(i) => CfmlValue::Int(*i + 1),
                CfmlValue::Double(d) => CfmlValue::Double(*d + 1.0),
                _ => CfmlValue::Int(1),
            };
            return Ok(());
        }
    }
    CfmlVirtualMachine::apply_numeric_delta(locals, closure_env.as_ref(), name, |val| {
        match val {
            CfmlValue::Int(i) => CfmlValue::Int(i + 1),
            CfmlValue::Double(d) => CfmlValue::Double(d + 1.0),
            _ => CfmlValue::Int(1),
        }
    });
    Ok(())
}

/// `Decrement`
#[inline]
pub(crate) fn op_decrement(
    locals: &mut ValueMap,
    slots: &mut [Option<CfmlValue>],
    closure_env: &Option<Arc<RwLock<ValueMap>>>,
    op: &BytecodeOp,
    name: &Name,
) -> Result<(), CfmlError> {
    if let BytecodeOp::DecrementSlot(i, _) = op {
        if let Some(v) = slots[*i as usize].as_mut() {
            *v = match v {
                CfmlValue::Int(i) => CfmlValue::Int(*i - 1),
                CfmlValue::Double(d) => CfmlValue::Double(*d - 1.0),
                _ => CfmlValue::Int(-1),
            };
            return Ok(());
        }
    }
    CfmlVirtualMachine::apply_numeric_delta(locals, closure_env.as_ref(), name, |val| {
        match val {
            CfmlValue::Int(i) => CfmlValue::Int(i - 1),
            CfmlValue::Double(d) => CfmlValue::Double(d - 1.0),
            _ => CfmlValue::Int(-1),
        }
    });
    Ok(())
}

/// `AddLocalConst`
#[inline]
pub(crate) fn op_add_local_const(
    locals: &mut ValueMap,
    slots: &mut [Option<CfmlValue>],
    closure_env: &Option<Arc<RwLock<ValueMap>>>,
    op: &BytecodeOp,
    name: &Name,
    k: i64,
) -> Result<(), CfmlError> {
    if let BytecodeOp::AddSlotConst(i, _, _) = op {
        if let Some(v) = slots[*i as usize].as_mut() {
            *v = match v {
                CfmlValue::Int(i) => CfmlValue::Int(*i + k),
                CfmlValue::Double(d) => CfmlValue::Double(*d + k as f64),
                _ => CfmlValue::Int(k),
            };
            return Ok(());
        }
    }
    CfmlVirtualMachine::apply_numeric_delta(locals, closure_env.as_ref(), name, |val| {
        match val {
            CfmlValue::Int(i) => CfmlValue::Int(i + k),
            CfmlValue::Double(d) => CfmlValue::Double(d + k as f64),
            _ => CfmlValue::Int(k),
        }
    });
    Ok(())
}

/// `MulLocalConst`
#[inline]
pub(crate) fn op_mul_local_const(
    locals: &mut ValueMap,
    slots: &mut [Option<CfmlValue>],
    closure_env: &Option<Arc<RwLock<ValueMap>>>,
    op: &BytecodeOp,
    name: &Name,
    k: i64,
) -> Result<(), CfmlError> {
    if let BytecodeOp::MulSlotConst(i, _, _) = op {
        if let Some(v) = slots[*i as usize].as_mut() {
            *v = match v {
                CfmlValue::Int(i) => CfmlValue::Int(*i * k),
                CfmlValue::Double(d) => CfmlValue::Double(*d * k as f64),
                _ => CfmlValue::Int(k),
            };
            return Ok(());
        }
    }
    CfmlVirtualMachine::apply_numeric_delta(locals, closure_env.as_ref(), name, |val| {
        match val {
            CfmlValue::Int(i) => CfmlValue::Int(i * k),
            CfmlValue::Double(d) => CfmlValue::Double(d * k as f64),
            _ => CfmlValue::Int(k),
        }
    });
    Ok(())
}

/// `JumpIfLocalCmpConstFalse`
#[inline]
pub(crate) fn op_jump_if_local_cmp_const_false(
    ip: &mut usize,
    locals: &ValueMap,
    slots: &[Option<CfmlValue>],
    op: &BytecodeOp,
    name: &Name,
    c: i64,
    cmp: CmpOp,
    target: usize,
) {
    // Fused loop-condition super-instruction. Equivalent to
    // LoadLocal(name) + Integer(c) + <cmp> + JumpIfFalse(target)
    // but avoids 3 dispatches per iteration.
    // T3.1: the slot twin resolves the counter from its slot
    // (any value type — the `other` arm below still applies full
    // CFML comparison semantics to a non-numeric slot value);
    // an inactive slot falls back to the named lookup.
    let slot_val = match op {
        BytecodeOp::JumpIfSlotCmpConstFalse(i, ..) => {
            slots[*i as usize].as_ref()
        }
        _ => None,
    };
    let matched = match slot_val.or_else(|| locals.get(name)) {
        Some(CfmlValue::Int(i)) => {
            let c = c;
            let i = *i;
            match cmp {
                CmpOp::Lt => i < c,
                CmpOp::Lte => i <= c,
                CmpOp::Gt => i > c,
                CmpOp::Gte => i >= c,
                CmpOp::Eq => i == c,
                CmpOp::Neq => i != c,
            }
        }
        Some(CfmlValue::Double(d)) => {
            let c = c as f64;
            let d = *d;
            match cmp {
                CmpOp::Lt => d < c,
                CmpOp::Lte => d <= c,
                CmpOp::Gt => d > c,
                CmpOp::Gte => d >= c,
                CmpOp::Eq => d == c,
                CmpOp::Neq => d != c,
            }
        }
        // Any other type (including missing): fall back to the
        // full CFML comparison semantics. Keeps correctness
        // for unusual cases (string loop var, null, etc.).
        other => {
            // An UNSCOPED loop counter inside a CFC/closure
            // component-scope context lands in `__variables`, not
            // `locals` (the routing apply_numeric_delta already
            // handles for Increment). When the counter is absent
            // from locals, consult `__variables` so this fused
            // condition test agrees with where the value lives —
            // otherwise it reads a stale/missing value and the loop
            // miscounts (Wheels miscellaneousSpec objectid
            // off-by-one). Only on the miss path, so the hot
            // plain-local case pays nothing.
            let left = match other {
                Some(v) => v.clone(),
                None => locals
                    .get(&*cfml_common::key::well_known::VARIABLES)
                    .and_then(|v| v.as_cfml_struct())
                    .and_then(|s| s.get_ci(name.as_str()))
                    .unwrap_or(CfmlValue::Null),
            };
            let right = CfmlValue::Int(c);
            match cmp {
                CmpOp::Lt => cfml_compare(&left, &right) < 0,
                CmpOp::Lte => cfml_compare(&left, &right) <= 0,
                CmpOp::Gt => cfml_compare(&left, &right) > 0,
                CmpOp::Gte => cfml_compare(&left, &right) >= 0,
                CmpOp::Eq => cfml_equal(&left, &right),
                CmpOp::Neq => !cfml_equal(&left, &right),
            }
        }
    };
    if !matched {
        *ip = target;
    }
}

/// `TryLoadLocal`
#[inline]
pub(crate) fn op_try_load_local(
    vm: &mut CfmlVirtualMachine,
    stack: &mut Vec<CfmlValue>,
    func: &BytecodeFunction,
    locals: &ValueMap,
    slots: &[Option<CfmlValue>],
    inherited_or_param_keys: &InheritedKeys,
    op: &BytecodeOp,
    name: &Name,
) -> Result<(), CfmlError> {
    // Slot fast path (T3.1) — see LoadSlot.
    if let BytecodeOp::TryLoadSlot(i, _) = op {
        if let Some(v) = slots[*i as usize].as_ref() {
            stack.push(v.clone());
            return Ok(());
        }
    }
    // Safe load: returns Null for undefined vars (used by Elvis, null-safe, isNull)
    // Same zero-alloc lowercase guard as LoadLocal.
    let name_lower: &str = name.lower();
    // GH #351: `local` names the SCOPE only in a frame that owns one. At page
    // level / in a pseudo-constructor the "locals" map IS the page `variables`
    // scope, so building a scope view here handed back every page variable — and
    // for `local.x = v`'s TryLoadLocal base it handed back a map that already
    // contained the `local` key itself, nesting it one level deeper per write.
    let val = if name_lower == "local" && vm.current_frame_has_local_scope() {
        // PR #93: per-frame `local` — only keys established here.
        CfmlValue::strukt(CfmlVirtualMachine::build_local_scope_view(
            &locals,
            &inherited_or_param_keys,
            &func.slot_names,
            &slots,
        ))
    } else if name_lower == "variables" {
        if let Some(CfmlValue::Struct(vars)) = locals.get(&*cfml_common::key::well_known::VARIABLES) {
            CfmlValue::Struct(vars.clone())
        } else {
            CfmlValue::strukt(locals.clone())
        }
    } else if name_lower == "request" {
        CfmlValue::Struct(vm.request_scope.clone())
    } else if name_lower == "application" {
        if let Some(ref app_scope) = vm.application_scope {
            // Live handle clone (see LoadLocal), not a snapshot.
            CfmlValue::Struct(app_scope.clone())
        } else {
            CfmlValue::Null
        }
    } else if name_lower == "server" {
        CfmlValue::Struct(vm.live_server_scope())
    } else {
        vm.lookup_name_in_scopes(name, &name_lower, &locals)
            .unwrap_or(CfmlValue::Null)
    };
    stack.push(val);
    Ok(())
}

/// `LoadLocalKey`
#[inline]
pub(crate) fn op_load_local_key(
    vm: &mut CfmlVirtualMachine,
    stack: &mut Vec<CfmlValue>,
    ip: &mut usize,
    locals: &ValueMap,
    slots: &[Option<CfmlValue>],
    inherited_or_param_keys: &InheritedKeys,
    op: &BytecodeOp,
    prop_name: &Name,
) -> Result<(), CfmlError> {
    // T3.1: `local.x` where x is an active slot — a slot value
    // is frame-established by definition, so it is always
    // visible through the `local` view. Inactive → generic
    // (map miss → Null/throw, matching an undeclared/deleted
    // local.x).
    if let BytecodeOp::LoadSlotKey(i, _) | BytecodeOp::TryLoadSlotKey(i, _) = op
    {
        if let Some(v) = slots[*i as usize].as_ref() {
            stack.push(v.clone());
            return Ok(());
        }
    }
    // GH #351 — a frame with NO function `local` scope (a top-level page, a CFC
    // pseudo-constructor) has no local view to read a key out of: there `local`
    // is an ordinary variable, so `local.foo` is a member read of whatever that
    // variable holds. Codegen cannot tell the two apart (a template `include`d
    // from inside a function compiles as `__main__` but DOES share the caller's
    // scope), so it emits this op at every depth and the decision is made here.
    if !vm.current_frame_has_local_scope() {
        let base = vm.lookup_name_in_scopes(
            &cfml_common::name::Name::intern("local"),
            "local",
            locals,
        );
        let val = base.as_ref().and_then(|b| match b {
            CfmlValue::Struct(st) => st.get_ci(prop_name.as_str()),
            _ => None,
        });
        match val {
            Some(v) => stack.push(v),
            // Same throw/Null split as the local-scope path below: the strict op
            // reports the missing member, the Try* twin reads Null.
            None if matches!(
                op,
                BytecodeOp::LoadLocalKey(_) | BytecodeOp::LoadSlotKey(..)
            ) =>
            {
                let cip = vm.raise_undefined_member(prop_name, stack)?;
                *ip = cip;
            }
            None => stack.push(CfmlValue::Null),
        }
        return Ok(());
    }
    // Fused LoadLocal("local") + GetProperty for an explicit
    // `local.foo` read. Reads the single member directly from
    // `locals` rather than materializing the whole per-call
    // `local` scope view. Applies the SAME visibility filter as
    // `build_local_scope_view`: a key is part of `local` only if
    // it was established in THIS frame — inherited/param keys,
    // `this`/`super`, and `__`-prefixed bridge keys are invisible.
    // A miss yields Null, matching GetProperty on the view struct.
    let is_visible = |k: &str| {
        !inherited_or_param_keys.contains(k)
            && k != "this"
            && k != "super"
            && !k.starts_with("__")
    };
    let name_lower: &str = prop_name.lower();
    // Exact hit first (the common case), then a case-insensitive
    // scan — mirrors GetProperty's resolution order.
    let val = locals
        .get_key_value(prop_name.as_str())
        .filter(|(k, _)| is_visible(k))
        .or_else(|| {
            locals
                .iter()
                .find(|(k, _)| {
                    k.eq_ignore_ascii_case(name_lower) && is_visible(k)
                })
        })
        .map(|(_, v)| v.clone());
    match val {
        Some(v) => stack.push(v),
        None if matches!(
            op,
            BytecodeOp::LoadLocalKey(_) | BytecodeOp::LoadSlotKey(..)
        ) =>
        {
            // `local.foo` read where foo isn't in this frame's local
            // scope → throw (Lucee/ACF parity).
            let cip = vm.raise_undefined_member(prop_name, stack)?;
            *ip = cip;
            return Ok(());
        }
        None => stack.push(CfmlValue::Null),
    }
    Ok(())
}

/// `LoadLocalProperty`
#[inline]
pub(crate) fn op_load_local_property(
    vm: &mut CfmlVirtualMachine,
    stack: &mut Vec<CfmlValue>,
    ip: &mut usize,
    locals: &ValueMap,
    slots: &[Option<CfmlValue>],
    op: &BytecodeOp,
    local_name: &Name,
    prop_name: &Name,
) -> Result<(), CfmlError> {
    // Fused LoadLocal + GetProperty. Avoids the intermediate
    // dispatch and the stack push/pop of the struct itself.
    // Only emitted when the receiver is a plain identifier
    // and access is non-null-safe (hot-path struct read).
    //
    // Resolve the receiver through the full scope chain, not
    // just `locals`: at page scope (template top-level), user
    // variables live in `globals`. Falling back through
    // `lookup_name_in_scopes` matches the semantics of plain
    // `LoadLocal` so `p.foo` reads agree with `p["foo"]`.
    // T3.1: slot twins resolve the receiver from the slot; an
    // inactive slot falls back to the identical named lookup.
    let slot_receiver = match op {
        BytecodeOp::LoadSlotProperty(i, ..)
        | BytecodeOp::TryLoadSlotProperty(i, ..) => {
            slots[*i as usize].clone()
        }
        _ => None,
    };
    let name_lower: &str = local_name.lower();
    let receiver = slot_receiver.or_else(|| {
        locals.get(local_name).cloned().or_else(|| {
            vm.lookup_name_in_scopes(
                local_name,
                name_lower,
                &locals,
            )
        })
    });
    let throw_on_miss = matches!(
        op,
        BytecodeOp::LoadLocalProperty(_, _)
            | BytecodeOp::LoadSlotProperty(..)
    );
    match receiver {
        // Undefined receiver variable: same as the unfused
        // `LoadLocal(root)` — throw on the root name.
        None if throw_on_miss => {
            let cip = vm.raise_undefined_member(local_name, stack)?;
            *ip = cip;
            return Ok(());
        }
        None => stack.push(CfmlValue::Null),
        Some(obj) => match CfmlVirtualMachine::lookup_property_opt(&obj, prop_name) {
            Some(v) => stack.push(v),
            // A declared-but-unpassed `arguments` param reads as
            // Null (Lucee/ACF), not an undefined-variable throw.
            None if throw_on_miss
                && !CfmlVirtualMachine::is_declared_arg_param(&obj, prop_name) =>
            {
                let cip = vm.raise_undefined_member(prop_name, stack)?;
                *ip = cip;
                return Ok(());
            }
            None => stack.push(CfmlValue::Null),
        },
    }
    Ok(())
}

/// `ArrayAppendLocal`
#[inline]
pub(crate) fn op_array_append_local(
    vm: &mut CfmlVirtualMachine,
    stack: &mut Vec<CfmlValue>,
    func: &BytecodeFunction,
    locals: &mut ValueMap,
    slots: &mut [Option<CfmlValue>],
    closure_env: &Option<Arc<RwLock<ValueMap>>>,
    declared_locals: &DeclaredLocals,
    effective_local_mode_modern: bool,
    is_inside_function: bool,
    op: &BytecodeOp,
    name: &Name,
) -> Result<(), CfmlError> {
    // Fused arrayAppend(<ident>, value). The value is on top of
    // the stack; the array lives in the named variable. With
    // reference-typed arrays the variable holds a shared handle,
    // so pushing in place is O(1) AND visible to every alias —
    // no clone, no store-back, no env sync needed. (Pre-reference
    // this had to fight Arc copy-on-write; that's all gone now.)
    let value = stack.pop().unwrap_or(CfmlValue::Null);

    // Slot fast path (T3.1): active slot holds the array handle.
    // A non-array active slot mirrors the generic "replace with a
    // fresh single-element array" tail (the slot IS the declared
    // local, so the store routes here by definition).
    if let BytecodeOp::ArrayAppendSlot(i, _) = op {
        match slots[*i as usize].as_ref() {
            Some(CfmlValue::Array(arr)) => {
                arr.push(value);
                return Ok(());
            }
            Some(_) => {
                slots[*i as usize] = Some(CfmlValue::array(vec![value]));
                return Ok(());
            }
            None => {}
        }
    }

    // Fast path: array held directly in this frame's locals.
    if let Some(CfmlValue::Array(arr)) = locals.get(name) {
        arr.push(value);
        return Ok(());
    }

    // Resolve through the full scope chain; the returned handle
    // shares the backing with the scope slot, so a push is seen
    // by the original (globals/__variables/case-insensitive).
    let name_lower: &str = name.lower();
    if let Some(CfmlValue::Array(arr)) =
        vm.lookup_name_in_scopes(name, name_lower, &locals)
    {
        arr.push(value);
        return Ok(());
    }

    // Not found (or not an array): create a fresh single-element
    // array and store it in the correct scope, mirroring how
    // StoreLocal routes a plain identifier.
    let val = CfmlValue::array(vec![value]);
    if locals.contains_key(&*cfml_common::key::well_known::VARIABLES)
        && !declared_locals.contains(name.as_str())
        && !locals.contains_key(name)
        && !effective_local_mode_modern
        // A declared parameter is local, never the component scope —
        // see the matching guard in StoreLocal above.
        && !func.params.iter().any(|p| p.eq_ignore_ascii_case(name))
    {
        // CFC method, classic localmode: component (variables) scope.
        if let Some(vars) =
            locals.get_mut(&*cfml_common::key::well_known::VARIABLES).and_then(|v| v.as_cfml_struct())
        {
            vars.insert(name, val);
        }
    } else {
        locals.insert(name, val.clone());
        if is_inside_function
            && !declared_locals.contains(name.as_str())
            && func.params.iter().any(|p| p.eq_ignore_ascii_case(name))
        {
            if let Some(args) =
                locals.get_mut(&*cfml_common::key::well_known::ARGUMENTS_SCOPE).and_then(|v| v.as_cfml_struct())
            {
                args.insert(name, val.clone());
            }
        }
        if let Some(ref env) = closure_env {
            let mut m = env.write().unwrap();
            if m.contains_key(name) {
                m.insert(name, val);
            }
        }
    }
    Ok(())
}

/// `UnsetPath`
#[inline]
pub(crate) fn op_unset_path(
    vm: &mut CfmlVirtualMachine,
    func: &BytecodeFunction,
    locals: &mut ValueMap,
    slots: &mut [Option<CfmlValue>],
    closure_env: &Option<Arc<RwLock<ValueMap>>>,
    inherited_or_param_keys: &mut InheritedKeys,
    effective_local_mode_modern: bool,
    path: &str,
) {
    // CFML null-assignment: `x = voidFn()` (Null RHS) must DELETE
    // the target rather than materialize a null-valued key. The
    // value (Null) was already popped by the guard's `Pop`.
    //
    // T3.1: an active slot for a bare `x` / `local.x` target is
    // cleared back to `None` — the name reads as undefined again
    // (slot ops fall back to the generic chain) and the generic
    // delete below still removes any inherited map copy. Codegen
    // excludes names that root a DEEPER dotted UnsetPath from
    // slotting, so `a.b.c` targets never involve a slot.
    if !slots.is_empty() {
        let mut segs = path.split('.');
        let first = segs.next().unwrap_or("");
        let second = segs.next();
        let target = match (second, segs.next()) {
            (None, _) => Some(first),
            (Some(s), None) if first.eq_ignore_ascii_case("local") => Some(s),
            _ => None,
        };
        if let Some(t) = target {
            if let Some(i) =
                func.slot_names.iter().position(|n| n.eq_ci(t))
            {
                slots[i] = None;
            }
        }
    }
    vm.delete_scope_path(path, locals, effective_local_mode_modern);
    // Drop any closure-env copy so sibling closures see the
    // deletion too (mirrors StoreLocal's env sync).
    if let Some(ref env) = closure_env {
        let mut m = env.write().unwrap();
        let found = m.keys().find(|k| k.eq_ignore_ascii_case(path)).cloned();
        if let Some(k) = found {
            m.shift_remove(&k);
        }
    }
    // A `var x` / `local.x` that resolved to Null leaves no key,
    // but the name was still claimed for this frame's `local`
    // view — keep it shadowing any inherited caller key.
    let leaf = path.rsplit('.').next().unwrap_or(path);
    inherited_or_param_keys.remove(leaf);
}

/// `CallSpread`
#[inline]
pub(crate) fn op_call_spread(
    vm: &mut CfmlVirtualMachine,
    stack: &mut Vec<CfmlValue>,
    func: &BytecodeFunction,
    locals: &mut ValueMap,
    slots: &mut [Option<CfmlValue>],
    slot_blocked: &mut u64,
) -> Result<(), CfmlError> {
    // Stack: [func_ref, args_array]
    let args_val = stack.pop().unwrap_or(CfmlValue::array(Vec::new()));
    let func_ref = stack.pop().unwrap_or(CfmlValue::Null);
    let args: Vec<CfmlValue> = if let CfmlValue::Array(a) = args_val {
        a.snapshot()
    } else {
        vec![args_val]
    };
    vm.closure_parent_writeback = None;
    vm.closure_parent_deletes = None;
    let result = vm.call_function(&func_ref, args, &locals)?;
    // Write back mutations into the shared closure environment
    if let Some(ref writeback) = vm.closure_parent_writeback {
        CfmlVirtualMachine::write_back_to_captured_scope(&func_ref, writeback);
    }
    if !slots.is_empty() && vm.closure_parent_writeback.is_some() {
        CfmlVirtualMachine::spill_slots_for_writeback(locals, &func.slot_names, slots, slot_blocked);
    }
    if let Some(writeback) = vm.closure_parent_writeback.take() {
        for (k, v) in writeback {
            locals.insert(k, v);
        }
    }
    stack.push(result);
    Ok(())
}

/// `CallRustSuperCtor`
#[inline]
pub(crate) fn op_call_rust_super_ctor(
    vm: &mut CfmlVirtualMachine,
    stack: &mut Vec<CfmlValue>,
    locals: &mut ValueMap,
    arg_count: usize,
) -> Result<(), CfmlError> {
    let mut ctor_args: Vec<CfmlValue> =
        (0..arg_count).filter_map(|_| stack.pop()).collect();
    ctor_args.reverse();

    let this_val = locals.get(&*cfml_common::key::well_known::THIS).cloned().ok_or_else(|| {
        CfmlError::runtime(
            "super(...) called outside of a CFC method".to_string(),
        )
    })?;
    // Flyweight: `this` is an `Instance`. Reconstruct the native
    // parent with the passed args (the parent CLASS name lives on
    // the blueprint's `rust_extends`) and store it PER-INSTANCE on
    // `native_parent`, replacing the default-constructed one.
    #[cfg(feature = "component-instance")]
    if let CfmlValue::Instance(ref inst) = this_val {
        let rust_class = {
            inst.read().class.rust_extends.clone().ok_or_else(|| {
                CfmlError::runtime(
                    "super(...) is only valid in a CFC that extends a rust: class"
                        .to_string(),
                )
            })?
        };
        let key = rust_class.to_lowercase();
        let ctor = vm.native_classes.get(&key).copied().ok_or_else(|| {
            CfmlError::runtime(format!(
                "No native (Rust) class registered with name '{}'",
                rust_class
            ))
        })?;
        let parent = ctor(ctor_args)?;
        inst.write().native_parent = Some(parent);
        stack.push(CfmlValue::Null);
        return Ok(());
    }
    let this_struct = match this_val {
        CfmlValue::Struct(s) => s,
        _ => {
            return Err(CfmlError::runtime(
                "super(...) requires `this` to be a component instance".to_string(),
            ));
        }
    };
    let rust_class = match this_struct.get("__rust_extends") {
        Some(CfmlValue::String(n)) => n.clone(),
        _ => {
            return Err(CfmlError::runtime(
                "super(...) is only valid in a CFC that extends a rust: class"
                    .to_string(),
            ));
        }
    };
    let key = rust_class.to_lowercase();
    let ctor = vm.native_classes.get(&key).copied().ok_or_else(|| {
        CfmlError::runtime(format!(
            "No native (Rust) class registered with name '{}'",
            rust_class
        ))
    })?;
    let parent = ctor(ctor_args)?;
    this_struct.insert("__super".to_string(), parent);
    let new_this = CfmlValue::Struct(this_struct);
    locals.insert("this".to_string(), new_this.clone());
    vm.method_this_writeback = Some(new_this);
    stack.push(CfmlValue::Null);
    Ok(())
}

/// `LoadSuper`
#[inline]
pub(crate) fn op_load_super(
    vm: &mut CfmlVirtualMachine,
    stack: &mut Vec<CfmlValue>,
    locals: &mut ValueMap,
) -> Result<(), CfmlError> {
    // Phase C.3 — Slice 5: flyweight instance super. The parent
    // super structs live on the shared blueprint (captured `__super`
    // / `__super_map`); resolve relative to the executing method's
    // defining source, then reuse the marker `__is_super` dispatch
    // (which binds `this` to the live instance from the frame).
    #[cfg(feature = "component-instance")]
    if let Some(CfmlValue::Instance(inst)) = locals.get(&*cfml_common::key::well_known::THIS) {
        let g = inst.read();
        let mut pushed = false;
        if let (Some(src), Some(CfmlValue::Struct(map))) =
            (vm.source_file.as_ref(), g.class.super_map.as_ref())
        {
            if let Some(sup) = map.get(src.as_str()).or_else(|| {
                map.iter()
                    .find(|(k, _)| k.eq_ignore_ascii_case(src))
                    .map(|(_, v)| v.clone())
            }) {
                stack.push(sup);
                pushed = true;
            }
        }
        if !pushed {
            if let Some(sup) = g.class.super_handle.clone() {
                stack.push(sup);
                pushed = true;
            }
        }
        // `rust:` parent: the per-instance NativeObject IS `super`
        // (a subsequent `CallMethod`/`GetProperty` on it dispatches
        // through the native short-circuit / trait getter).
        if !pushed {
            if let Some(np) = g.native_parent.clone() {
                stack.push(np);
                pushed = true;
            }
        }
        if !pushed {
            drop(g);
            if let Some(sup) = vm.pseudo_ctor_super.last() {
                stack.push(sup.clone());
            } else {
                stack.push(CfmlValue::Null);
            }
        }
        return Ok(());
    }
    // `super` resolves relative to the DEFINING class of the
    // currently-executing method, not the leaf instance. During
    // method execution `self.source_file` is the defining class's
    // path, so look up `this.__super_map[<that source>]` — the
    // parent-method struct for THIS level of the inheritance
    // chain. Falls back to the flat `__super` (2-level CFCs and
    // rust-parent objects, which carry no map).
    let mut pushed = false;
    if let Some(CfmlValue::Struct(s)) = locals.get(&*cfml_common::key::well_known::THIS) {
        if let (Some(src), Some(CfmlValue::Struct(map))) =
            (vm.source_file.as_ref(), s.get_ci("__super_map"))
        {
            if let Some(sup) = map.get(src.as_str()).or_else(|| {
                map.iter()
                    .find(|(k, _)| k.eq_ignore_ascii_case(src))
                    .map(|(_, v)| v.clone())
            }) {
                stack.push(sup);
                pushed = true;
            }
        }
        if !pushed {
            if let Some(sup) = s.get_ci(&*cfml_common::key::well_known::SUPER_NATIVE) {
                stack.push(sup);
                pushed = true;
            }
        }
    }
    // Fallback for a `super.x()` reference inside a component
    // pseudo-constructor, where `this.__super` isn't assembled
    // yet (the instance is still being built). See
    // `pseudo_ctor_super`.
    if !pushed {
        if let Some(sup) = vm.pseudo_ctor_super.last() {
            stack.push(sup.clone());
            pushed = true;
        }
    }
    if !pushed {
        stack.push(CfmlValue::Null);
    }
    Ok(())
}

/// `StoreLocalScopeKey` — `local.X = v` compiled at template level (GH #351).
///
/// See the opcode's own docs for why the choice cannot be made at compile time.
/// When the frame owns a function `local` scope this is exactly what a function
/// body compiles `local.X = v` to (declare the name frame-private, then store);
/// when it does not, `local` is an ordinary variable that this write has to
/// auto-vivify as a struct — which is what Lucee does when it creates
/// `variables.local`.
#[inline]
pub(crate) fn op_store_local_scope_key(
    vm: &mut CfmlVirtualMachine,
    stack: &mut Vec<CfmlValue>,
    locals: &mut ValueMap,
    declared_locals: &mut DeclaredLocals,
    inherited_or_param_keys: &mut InheritedKeys,
    frame_has_local_scope: bool,
    prop_name: &Name,
) {
    let Some(value) = stack.pop() else { return };
    if frame_has_local_scope {
        op_declare_local(declared_locals, inherited_or_param_keys, prop_name);
        crate::scope_insert_ci_pub(locals, prop_name.as_str(), value);
        return;
    }
    // No local scope: read-modify-write the ordinary `local` variable. In such a
    // frame `locals` IS the page / component `variables` scope, so that variable
    // lives right here.
    match vm.lookup_local_name(locals, "local") {
        Some(CfmlValue::Struct(st)) => {
            // A live handle — mutate in place so aliases see the write, exactly
            // as `s.x = v` on any other struct variable does. `insert` is
            // case-insensitive (Key compares CI), matching CFML.
            st.insert(prop_name.as_str().to_string(), value);
        }
        _ => {
            let mut m = ValueMap::default();
            m.insert(prop_name.as_str().to_string(), value);
            let fresh = CfmlValue::strukt(m);
            // Store where a bare `variables` READ resolves in this frame: the
            // component scope under `__variables` when there is one (a CFC
            // pseudo-constructor), otherwise the frame's own map (a page). Getting
            // this wrong is invisible to `variables.local.x`, which walks the
            // chain and finds it either way — but `structKeyExists( variables,
            // "local" )` reads the materialized scope and would miss it.
            match locals
                .get_mut(&*cfml_common::key::well_known::VARIABLES)
                .and_then(|v| v.as_cfml_struct())
            {
                Some(vars) => {
                    vars.insert("local".to_string(), fresh);
                }
                None => crate::scope_insert_ci_pub(locals, "local", fresh),
            }
        }
    }
}
