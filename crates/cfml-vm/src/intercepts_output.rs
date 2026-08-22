//! VM-intercepted output builtins — writeOutput/echo/writeDump and friends, which must reach `output_buffer` rather than the builtin's stdout path.
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
#[inline]
pub(crate) fn handles(name_lower: &str) -> bool {
    matches!(name_lower, "writeoutput" | "echo" | "__writetext" | "writedump" | "dump" | "cfdump")
}

impl CfmlVirtualMachine {
    /// Bodies moved verbatim, so `?` and the original `return Ok(..)` still work.
    /// Bodies moved VERBATIM: `args` stays an owned `Vec` as in the original scope, so
    /// `?`, `return Ok(..)`, `&args` and `args.into_iter()` all still work. The caller
    /// checks [`handles`] first, so `args` only moves when this domain consumes it.
    pub(crate) fn dispatch_output(
        &mut self,
        name_lower: &str,
        args: Vec<CfmlValue>,
    ) -> CfmlResult {
            // so output goes to output_buffer (not stdout via the builtin fn)
            if name_lower == "writeoutput" || name_lower == "echo" {
                // Lucee parity: writeOutput/echo of a complex value throws a
                // catchable `expression` error rather than dumping it.
                for arg in &args {
                    let s = arg.to_string_strict().map_err(|e| self.wrap_error(e))?;
                    self.output_buffer.push_str(&s);
                }
                return Ok(CfmlValue::Null);
            }

            // __writeText: same as writeOutput but suppressed when enableCFOutputOnly > 0
            if name_lower == "__writetext" {
                if self.enable_cfoutput_only <= 0 {
                    for arg in &args {
                        // §3.5: the copy was pure waste — the text is immediately
                        // pushed into the output buffer and dropped.
                        self.output_buffer.push_str(&arg.as_str_cow());
                    }
                }
                return Ok(CfmlValue::Null);
            }
            if name_lower == "writedump" || name_lower == "dump" || name_lower == "cfdump" {
                let named = self.pending_dump_named.take();
                // Resolve the value to dump and the options. Named args (when
                // present) take precedence; otherwise fall back to positional
                // (var is the first positional arg).
                let mut opts = dump::DumpOptions::default();
                let mut var: Option<CfmlValue> = None;
                let mut output: Option<String> = None;
                let mut abort_after = false;
                if let Some(pairs) = &named {
                    for (k, v) in pairs {
                        match k.to_lowercase().as_str() {
                            "var" => var = Some(v.clone()),
                            "label" => opts.label = Some(v.as_string()),
                            "expand" => opts.expand = v.is_true(),
                            "top" => {
                                let n = v.as_string().parse::<i64>().unwrap_or(0);
                                if n > 0 {
                                    opts.top = Some(n as usize);
                                }
                            }
                            "output" => output = Some(v.as_string()),
                            "abort" => abort_after = v.is_true(),
                            _ => {}
                        }
                    }
                }
                let value = var.or_else(|| args.into_iter().next()).unwrap_or(CfmlValue::Null);
                // output="console" sends a plain-text dump to the server console
                // (stdout), leaving the page / HTTP response untouched (issue #207
                // — Lucee/ACF parity). TestBox/ColdBox use this for non-fatal
                // diagnostics; routing it into the response polluted reporter output.
                if output
                    .as_deref()
                    .map(|o| o.eq_ignore_ascii_case("console"))
                    .unwrap_or(false)
                {
                    let rendered = dump::render(&value, &opts, false, false);
                    print!("{}", rendered);
                    use std::io::Write;
                    let _ = std::io::stdout().flush();
                    return self.finish_dump(abort_after);
                }
                // `output="<path>"` writes the dump to a FILE and keeps the
                // response clean. Lucee uses the plain-text rendering (the same
                // one `output="console"` emits), APPENDS to an existing file, and
                // resolves the path the way ExpandPath does — leading slash =
                // web root, otherwise relative to the calling template (all
                // probed on 7.0.4). A missing parent directory is an
                // `application` error with Lucee's wording, not a silent skip.
                if let Some(path) = output
                    .as_deref()
                    .filter(|o| !o.is_empty() && !o.eq_ignore_ascii_case("browser"))
                {
                    let resolved = self.resolve_template_relative(path, false);
                    let target = std::path::Path::new(&resolved);
                    match target.parent() {
                        Some(dir) if !dir.as_os_str().is_empty() && !dir.is_dir() => {
                            return Err(CfmlError::new(
                                format!(
                                    "Parent directory for [{}] doesn't exist",
                                    target.display()
                                ),
                                CfmlErrorType::Application,
                            ));
                        }
                        _ => {}
                    }
                    let rendered = dump::render(&value, &opts, false, false);
                    use std::io::Write;
                    let appended = std::fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(target)
                        .and_then(|mut f| f.write_all(rendered.as_bytes()));
                    if let Err(e) = appended {
                        return Err(CfmlError::new(
                            format!("cannot write dump to [{}]: {}", target.display(), e),
                            CfmlErrorType::Application,
                        ));
                    }
                    return self.finish_dump(abort_after);
                }
                let include_assets = self.web_context && !self.dump_assets_emitted;
                let rendered = dump::render(&value, &opts, self.web_context, include_assets);
                if include_assets {
                    self.dump_assets_emitted = true;
                }
                self.output_buffer.push_str(&rendered);
                return self.finish_dump(abort_after);
            }


        // Fell out of every branch — exactly what the original `if` chain did.
        // The caller turns this into fall-through; it is never seen by CFML.
        Err(intercepts_common::unhandled())
    }
}
