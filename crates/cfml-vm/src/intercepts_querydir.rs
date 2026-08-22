//! VM-intercepted query and directory builtins — cfdirectory, queryAppend, querySetRow.
//!
//! Extracted from `call_function`, which had grown to ~7,500 lines because the documented
//! way to add a VM-intercepted builtin was to append another `if name_lower == "…"` branch
//! and nothing ever moved one out (see `PERFORMANCE_ROADMAP.md` Part -1).
//!
//! Returns `Option<CfmlResult>`: `Some` when this domain handled the call, `None` to fall
//! through. `handles()` lives next to the implementations so the two cannot drift, and the
//! source-scanning guard in `tests/intercept_declaration_guard.rs` still sees these names,
//! so moving code here cannot silently drop an interception.

use super::*;

/// Names this module handles.
///
/// Name-only on purpose: several branches here fall through (e.g. `cfdirectory` returns
/// for `action = "list"` and defers every other action to the generic builtin path).
/// Reproducing those inner conditions here would duplicate them in a second place that
/// would drift; instead `dispatch_querydir` reports fall-through via
/// [`intercepts_common::unhandled`].
#[inline]
pub(crate) fn handles(name_lower: &str) -> bool {
    matches!(name_lower, "cfdirectory" | "__cfdirectory" | "queryappend" | "querysetrow")
}

impl CfmlVirtualMachine {
    /// Bodies moved VERBATIM from `call_function`: `args` stays an owned `Vec` exactly as
    /// it was in the original scope, so `?`, `return Ok(..)`, `&args` and `args.into_iter()`
    /// all still work and the diff is a pure move. The caller checks [`handles`] first, so
    /// `args` is only moved when this domain will actually consume it.
    pub(crate) fn dispatch_querydir(
        &mut self,
        name_lower: &str,
        args: Vec<CfmlValue>,
    ) -> CfmlResult {
            if matches!(name_lower, "cfdirectory" | "__cfdirectory") {
                if let Some(CfmlValue::Struct(opts)) = args.first() {
                    let action = opts
                        .get_ci("action")
                        .map(|v| v.as_string().to_lowercase())
                        .unwrap_or_else(|| "list".to_string());
                    if action == "list" {
                        return self.cfdirectory_list_from_opts(opts);
                    }
                }
            }

            // queryAppend: mutates the first query in-place (reference-typed —
            // the shared handle propagates to the caller), returns boolean.
            if name_lower == "queryappend" {
                if let (Some(CfmlValue::Query(q1)), Some(CfmlValue::Query(q2))) =
                    (args.first(), args.get(1))
                {
                    let q2_data: cfml_common::dynamic::CfmlQueryData =
                        q2.with_read(|d| d.clone());
                    q1.with_write(|d| d.append_query(&q2_data));
                    return Ok(CfmlValue::Bool(true));
                }
                return Ok(CfmlValue::Bool(false));
            }

            // querySetRow: mutates query in-place, returns boolean.
            if name_lower == "querysetrow" {
                if let (Some(CfmlValue::Query(q)), Some(row_pos), Some(CfmlValue::Struct(new_row))) =
                    (args.first(), args.get(1), args.get(2))
                {
                    let pos = match row_pos {
                        CfmlValue::Int(i) => *i as usize,
                        CfmlValue::Double(d) => *d as usize,
                        _ => 0,
                    };
                    let new_row = new_row.snapshot();
                    let ok = q.with_write(|d| {
                        if pos >= 1 && pos <= d.row_count() {
                            for ci in 0..d.columns.len() {
                                let col_name = d.columns[ci].clone();
                                let val = new_row
                                    .iter()
                                    .find(|(k, _)| k.eq_ignore_ascii_case(&col_name))
                                    .map(|(_, v)| v.clone())
                                    .unwrap_or(CfmlValue::Null);
                                std::sync::Arc::make_mut(&mut d.data[ci])[pos - 1] = val;
                            }
                            true
                        } else {
                            false
                        }
                    });
                    return Ok(CfmlValue::Bool(ok));
                }
                return Ok(CfmlValue::Bool(false));
            }

            // In-place array mutators that return boolean (matches Lucee):
            // arrayDelete, arrayDeleteNoCase. Mutate the caller's array via
            // arg_ref_writeback and return true/false based on whether the
            // element was found.

        // Fell out of every branch — exactly what the original `if` chain did.
        // The caller turns this into fall-through; it is never seen by CFML.
        Err(intercepts_common::unhandled())
    }
}
