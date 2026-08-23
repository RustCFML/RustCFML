//! Reflective variable-path builtins — `structGet`, `getVariable`, `setVariable`.
//!
//! These three take a variable path as a runtime STRING, so none of them can use the
//! compiler's member-access lowering; they need the scope chain of the CALLING frame,
//! which is why they are VM-intercepted rather than plain stdlib functions.
//!
//! They share one path interpreter (`parse_variable_path` / `resolve_variable_path` /
//! `create_variable_path` on the VM) for the same reason Lucee shares
//! `VariableInterpreter` between them: when the read half and the existence half
//! disagree about what a string means, `isDefined(p)` and `getVariable(p)` start
//! contradicting each other for the same `p`.

use super::*;

/// Names this module handles. Every one returns — there is no fall-through.
#[inline]
pub(crate) fn handles(name_lower: &str) -> bool {
    matches!(name_lower, "structget" | "getvariable")
}

impl CfmlVirtualMachine {
    pub(crate) fn dispatch_varpath(
        &mut self,
        name_lower: &str,
        args: Vec<CfmlValue>,
        locals: &ValueMap,
    ) -> CfmlResult {
        let path = args.first().map(|v| v.as_string()).unwrap_or_default();
        match name_lower {
            // structGet(path): return what the path resolves to; if it resolves to
            // nothing, CREATE it as an empty struct and return that (Lucee's
            // `StructGet.call` is exactly this two-liner). The returned struct is the
            // stored handle, so the get-or-create idiom writes through.
            "structget" => match self.resolve_variable_path(&path, locals)? {
                Some(v) => Ok(v),
                None => self.create_variable_path(&path, locals),
            },
            // getVariable(path): read-only. Lucee returns null for a missing path
            // rather than throwing, so a bad path is `Null`, not an error — but an
            // unparseable NAME still throws, matching `VariableInterpreter`.
            "getvariable" => Ok(self
                .resolve_variable_path(&path, locals)?
                .unwrap_or(CfmlValue::Null)),
            _ => Err(intercepts_common::unhandled()),
        }
    }
}
