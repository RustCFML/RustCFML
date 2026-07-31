//! The VM hook bus — Phase 0 of the observability/debugging plan
//! (`docs/observability-implementation-plan.md`).
//!
//! A single, near-zero-cost event bus inside the VM that every later layer
//! (the classic debug footer, the sampling profiler, OpenTelemetry, the DAP
//! step debugger) subscribes to. The whole module is behind the
//! `observability` Cargo feature — when the feature is off the call sites in
//! the VM vanish entirely, so the hot path is byte-identical and the wasm
//! crates (which build `cfml-vm` with `default-features = false`) pay nothing.
//!
//! ## Design
//! * Each subscriber implements [`VmObserver`] and declares an [`Interest`]
//!   bitmask. The VM caches the OR of every registered observer's interest in a
//!   plain field, so a hook site is a `bitand`+branch when nobody cares about
//!   that category.
//! * Boundaries have default no-op trait methods, so a subscriber only
//!   implements the events it actually wants.
//!
//! Stage 1 wires the *non-hot* boundaries (request, query, template,
//! transaction, error, log, bif) that the classic CF debug footer needs. The
//! two hot hooks the plan calls out — `function_enter/exit` and `line` — are
//! deliberately **not** fired yet: no stage-1 subscriber needs them, and adding
//! calls to `call_function`/`LineInfo` risks the JIT-admission regressions
//! CLAUDE.md warns about. They land with the profiler/DAP (phases 2/5), whose
//! observers set the `FUNCTION`/`LINE` interest bits reserved below.

#![cfg(feature = "observability")]

/// Interest bitmask. A subscriber returns the union of the categories whose
/// events it wants; the VM ORs every observer's mask and checks it before
/// firing a hook. The bit layout matches the plan's `observe.rs` sketch.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Interest(u32);

impl Interest {
    pub const NONE: Interest = Interest(0);
    pub const REQUEST: Interest = Interest(1 << 0);
    /// Hot — only the profiler/DAP set this (function enter/exit). Reserved.
    pub const FUNCTION: Interest = Interest(1 << 1);
    pub const TEMPLATE: Interest = Interest(1 << 2);
    pub const QUERY: Interest = Interest(1 << 3);
    pub const TRANSACTION: Interest = Interest(1 << 4);
    /// Metrics only — never a span; a per-BIF counter bump.
    pub const BIF: Interest = Interest(1 << 5);
    pub const ERROR: Interest = Interest(1 << 6);
    pub const LOG: Interest = Interest(1 << 7);
    /// Hottest — only the per-line profiler + DAP debugger set this. Reserved.
    pub const LINE: Interest = Interest(1 << 8);

    /// True when every bit in `other` is also set in `self`.
    #[inline]
    pub fn contains(self, other: Interest) -> bool {
        (self.0 & other.0) == other.0 && other.0 != 0
    }

    #[inline]
    pub fn is_empty(self) -> bool {
        self.0 == 0
    }
}

impl std::ops::BitOr for Interest {
    type Output = Interest;
    #[inline]
    fn bitor(self, rhs: Interest) -> Interest {
        Interest(self.0 | rhs.0)
    }
}

impl std::ops::BitOrAssign for Interest {
    #[inline]
    fn bitor_assign(&mut self, rhs: Interest) {
        self.0 |= rhs.0;
    }
}

// ── Event payloads ────────────────────────────────────────────────────────
// Borrowed slices/`&str` so firing a hook allocates nothing on the VM side;
// the subscriber copies only what it keeps.

/// A `queryExecute` / `<cfquery>` completed. Carries the already-measured
/// execution time (reused from the query intercept's existing `Instant`).
pub struct QueryEvent<'a> {
    pub name: &'a str,
    pub sql: &'a str,
    pub datasource: &'a str,
    pub rowcount: i64,
    /// Execution time in **microseconds** (Lucee stores query times in µs).
    pub elapsed_us: i64,
    pub cached: bool,
    /// Issuing template.
    pub src: &'a str,
    pub line: usize,
    /// Bound query parameters, in supply order — Lucee shows these (name, value
    /// and cfsqltype) alongside the SQL. Empty for a param-less query.
    pub params: &'a [QueryParam],
}

/// One bound query parameter. Mirrors Lucee's `PARAMVALUE` + `PARAMTYPE`
/// debug columns.
#[derive(Clone, Default)]
pub struct QueryParam {
    /// Parameter name (named params) or 1-based index (positional).
    pub name: String,
    pub value: String,
    /// cfsqltype (e.g. `cf_sql_integer`), empty when not specified.
    pub sqltype: String,
}

/// A template executed: an included file (`<cfinclude>`), a custom tag or
/// `<cfmodule>` body, a component method call, or an `Application.cfc`
/// lifecycle method — the same set Lucee's Execution Time section covers
/// ("templates, includes, modules, custom tags, and component method calls").
/// `path` is the source file; the footer aggregates per file (Lucee's
/// `pages`/Templates section).
pub struct TemplateEvent<'a> {
    pub path: &'a str,
    /// Execution time in **microseconds**.
    pub elapsed_us: i64,
}

/// A CFML exception was raised. `uncaught` distinguishes a genuinely unhandled
/// error from one a `try/catch` recovered.
pub struct ErrorEvent<'a> {
    pub etype: &'a str,
    pub message: &'a str,
    pub detail: &'a str,
    pub src: &'a str,
    pub line: usize,
    pub uncaught: bool,
    /// `(template, line)` frames, outermost first.
    pub stack: Vec<(String, usize)>,
}

/// A `<cflog>` / `writeLog()` call.
pub struct LogEvent<'a> {
    pub text: &'a str,
    pub log_type: &'a str,
    pub file: &'a str,
}

/// A user function / method / closure is about to execute. `depth` is the call
/// depth *before* this frame (0 for a call made from top-level page code), so an
/// observer can apply a depth cap. Fired only when an observer sets the
/// `FUNCTION` interest — the hot path is otherwise a bitand+branch.
pub struct FnEnter<'a> {
    pub name: &'a str,
    /// The name the function was invoked under at the call site (may differ from
    /// `name` for aliased/delegated methods).
    pub called_name: &'a str,
    pub depth: usize,
}

/// A user function returned — on a normal return AND on exception unwind (fired
/// from the call wrapper, which runs on both paths). `is_error` is true when the
/// body returned an `Err` (so a span can be closed with Error status).
pub struct FnExit<'a> {
    pub name: &'a str,
    pub depth: usize,
    pub is_error: bool,
}

/// A `<cftransaction>` boundary. `action` is `begin` / `commit` / `rollback`.
pub struct TxnEvent<'a> {
    pub action: &'a str,
    pub datasource: &'a str,
}

/// One observer = one subscriber. The VM holds at most one composed observer
/// today (the footer collector); a `Composite` fan-out can be added when a
/// second subscriber ships.
pub trait VmObserver: Send + Sync {
    /// The categories this observer wants events for.
    fn interest(&self) -> Interest;

    fn on_query(&self, _q: &QueryEvent) {}
    fn on_template(&self, _t: &TemplateEvent) {}
    fn on_error(&self, _e: &ErrorEvent) {}
    fn on_log(&self, _l: &LogEvent) {}
    fn on_bif(&self, _name: &str) {}
    /// Hot — fired per function call only when `FUNCTION` interest is set.
    fn on_fn_enter(&self, _f: &FnEnter) {}
    /// Symmetric with [`Self::on_fn_enter`]; also fires on exception unwind.
    fn on_fn_exit(&self, _f: &FnExit) {}
    fn on_transaction(&self, _t: &TxnEvent) {}
}

/// Fans a single VM observer slot out to several subscribers (e.g. the debug
/// footer collector *and* the OpenTelemetry exporter in one request). Its
/// `interest()` is the OR of its members', so a hook fires if *any* member
/// wants it; each member re-checks its own interest cheaply inside its handler.
pub struct Composite {
    observers: Vec<std::sync::Arc<dyn VmObserver>>,
    interest: Interest,
}

impl Composite {
    pub fn new(observers: Vec<std::sync::Arc<dyn VmObserver>>) -> Self {
        let mut interest = Interest::NONE;
        for o in &observers {
            interest |= o.interest();
        }
        Self { observers, interest }
    }
}

impl VmObserver for Composite {
    fn interest(&self) -> Interest {
        self.interest
    }
    fn on_query(&self, q: &QueryEvent) {
        for o in &self.observers {
            o.on_query(q);
        }
    }
    fn on_template(&self, t: &TemplateEvent) {
        for o in &self.observers {
            o.on_template(t);
        }
    }
    fn on_error(&self, e: &ErrorEvent) {
        for o in &self.observers {
            o.on_error(e);
        }
    }
    fn on_log(&self, l: &LogEvent) {
        for o in &self.observers {
            o.on_log(l);
        }
    }
    fn on_bif(&self, name: &str) {
        for o in &self.observers {
            o.on_bif(name);
        }
    }
    fn on_fn_enter(&self, f: &FnEnter) {
        for o in &self.observers {
            o.on_fn_enter(f);
        }
    }
    fn on_fn_exit(&self, f: &FnExit) {
        for o in &self.observers {
            o.on_fn_exit(f);
        }
    }
    fn on_transaction(&self, t: &TxnEvent) {
        for o in &self.observers {
            o.on_transaction(t);
        }
    }
}
