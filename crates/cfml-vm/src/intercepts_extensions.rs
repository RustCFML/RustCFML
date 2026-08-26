//! Introspection that has to see loaded extensions.
//!
//! `getFunctionList()` is an ordinary stdlib function that walks the compiled-in
//! registration table — which knows nothing about a `.rcx`. Left alone, an
//! extension's BIFs are callable but invisible: they never appear in
//! `getFunctionList()`, and anything that enumerates the engine's functions (an
//! IDE completion feed, a docs generator, `<cfdump>` of the function list)
//! reports them as absent. That is worse than a missing feature, because the
//! answer looks authoritative.

use super::*;

/// Names this module handles. Every one returns — there is no fall-through.
#[inline]
pub(crate) fn handles(name_lower: &str) -> bool {
    matches!(name_lower, "getfunctionlist")
}

impl CfmlVirtualMachine {
    pub(crate) fn dispatch_extension_introspection(
        &mut self,
        name_lower: &str,
        _args: Vec<CfmlValue>,
    ) -> CfmlResult {
        match name_lower {
            // Every callable builtin name, compiled-in AND extension-provided.
            // The value is the providing extension's name for a foreign
            // function and an empty string otherwise, which keeps the CFML
            // contract (a struct keyed by name) while making provenance
            // available to anything that wants it.
            "getfunctionlist" => {
                let mut out = ValueMap::default();
                for name in self.builtins.keys() {
                    out.insert(name.clone(), CfmlValue::string(String::new()));
                }
                for fb in self.foreign_builtins.values() {
                    out.insert(fb.name.to_string(), CfmlValue::string(fb.module.to_string()));
                }
                Ok(CfmlValue::strukt(out))
            }
            // Not ours: the caller turns this back into fall-through.
            _ => Err(intercepts_common::unhandled()),
        }
    }
}
