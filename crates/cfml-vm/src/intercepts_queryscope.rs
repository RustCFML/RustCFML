//! The query-loop scope stack (GH #544) — `__queryScopePush` / `__queryScopePop`.
//!
//! A query loop's lowering pushes the looped query on entry and pops it in a
//! `finally`, so `return`, `break` and a thrown error all leave the stack as
//! they found it. While it is pushed, a bare name that is not a local or an
//! argument resolves against it before `variables` (see
//! `CfmlVirtualMachine::query_scope_get`), as Lucee's query stack does.

use super::*;

/// Names this module handles. Every one returns — there is no fall-through.
#[inline]
pub(crate) fn handles(name_lower: &str) -> bool {
    matches!(name_lower, "__queryscopepush" | "__queryscopepop")
}

impl CfmlVirtualMachine {
    pub(crate) fn dispatch_query_scope(
        &mut self,
        name_lower: &str,
        args: Vec<CfmlValue>,
    ) -> CfmlResult {
        match name_lower {
            // Push the query and return the stack depth it was pushed at, which
            // the matching pop is handed back. A non-query (an empty string from
            // an undefined `query=` attribute, say) pushes nothing; the loop's
            // own `.recordcount` read reports that error.
            "__queryscopepush" => {
                let depth = self.query_scope.len();
                if let Some(CfmlValue::Query(q)) = args.into_iter().next() {
                    self.query_scope.push(q);
                }
                Ok(CfmlValue::Int(depth as i64))
            }
            // Truncate back to the depth the push reported, so a pop always
            // undoes exactly its own push and any inner ones an error skipped.
            "__queryscopepop" => {
                let depth = args.first().and_then(|v| v.as_string().parse::<usize>().ok());
                if let Some(depth) = depth {
                    self.query_scope.truncate(depth);
                }
                Ok(CfmlValue::Null)
            }
            _ => Err(intercepts_common::unhandled()),
        }
    }
}
