//! QoQ execution engine.
//!
//! A single `eval` walks each expression with a `RowCtx` discriminator —
//! `Row` for scalar (per-row) evaluation, `Group` for aggregate (per-partition)
//! evaluation — so operator logic is written once (BoxLang's dual-path
//! `evaluate` / `evaluateAggregate`, unified). The pipeline per SELECT core is:
//! resolve tables → build join intersections → WHERE → GROUP BY/HAVING (or
//! simple projection) → DISTINCT → (statement level) UNION → ORDER BY →
//! LIMIT/OFFSET.

use std::cell::RefCell;
use std::cmp::Ordering;
use std::collections::HashSet;

use cfml_common::dynamic::{CfmlQuery, CfmlQueryData, CfmlValue, ValueMap};
use cfml_common::vm::{CfmlError, CfmlResult};
use indexmap::IndexMap;

use crate::ast::*;
use crate::compare::{append_group_key, compare_sql, compare_total, group_key, sql_equal};
use crate::compiled::{self, CmpOp, CompiledExpr};
use crate::function::{QoQFnKind, QoQFunctionRegistry};
use crate::functions;
use crate::intersection::{build_intersections, Intersections};
use crate::like::{self, like_match};
use crate::table::{QoQTable, TableSet};

/// Upper bound on materialised join row-combinations before the engine refuses
/// (catchable error) rather than risk exhausting memory. Filtered `JOIN … ON`
/// keeps intersections small and is unaffected; an unfiltered N-table comma
/// join over a huge table is what this stops. ~30M × small Vec ≈ a few hundred MB.
const MAX_INTERSECTIONS: usize = 30_000_000;

/// Above this many rows, the per-row WHERE filter and projection fan out across
/// cores with rayon — but only when the statement is "pure" (no custom CFML UDF
/// and no subquery anywhere), so evaluation never needs the non-`Send` VM
/// callback. Non-wasm only.
#[cfg(not(target_arch = "wasm32"))]
// Lowered from 10_000 to 1_000 (v0.105.0) — measured against BoxLang, which
// fans out at 50–100. Above ~1k rows the rayon fan-out cost is dwarfed by the
// per-row WHERE/projection evaluation; below that, the sequential path wins.
const PARALLEL_ROW_THRESHOLD: usize = 1_000;

/// Bind parameters supplied to a parameterised QoQ query.
#[derive(Debug, Default)]
pub struct QoQParams {
    /// Positional `?` parameters, in order.
    pub positional: Vec<CfmlValue>,
    /// Named `:name` parameters (matched case-insensitively).
    pub named: ValueMap,
}

impl QoQParams {
    pub fn none() -> Self {
        Self::default()
    }

    fn lookup_named(&self, name: &str) -> Option<&CfmlValue> {
        self.named
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v)
    }
}

/// Execute a parsed statement against the supplied source queries.
///
/// `udf` invokes a CFML UDF/closure (its first arg is the `CfmlValue::Function`
/// or `Closure`); the VM supplies a closure wrapping its own call machinery.
pub fn execute(
    stmt: &Statement,
    sources: &[(String, &CfmlQuery)],
    params: &QoQParams,
    registry: &QoQFunctionRegistry,
    udf: &mut dyn FnMut(&CfmlValue, Vec<CfmlValue>) -> CfmlResult,
) -> CfmlResult {
    let Statement::Select(select) = stmt;
    // A statement is parallel-eligible only if no expression anywhere calls a
    // custom CFML UDF and there are no subqueries/derived tables — i.e. row
    // evaluation never needs the (non-`Send`) VM callback or a recursive run.
    let parallel = is_parallel_safe(select, registry);
    // Constant-pattern LIKE matchers are pre-compiled by `bind_expr` directly
    // onto the cloned AST node (`Expr::Like.compiled`) per `run_core`, so the
    // per-row filter reuses it instead of recompiling per row.
    //
    // The sequential invoker owns the VM callback behind a `RefCell` so the
    // engine's methods can be `&self` (the only thing that ever needed `&mut`).
    let seq = SeqInvoker {
        udf: RefCell::new(udf),
    };
    let engine = Engine {
        sources,
        params,
        registry,
        inv: &seq,
        parallel,
    };
    let query = engine.run_statement(select)?;
    Ok(CfmlValue::Query(CfmlQuery::from_data(query)))
}

/// The one side-effecting operation expression evaluation needs that the
/// parallel path must not perform: invoking a custom CFML UDF through the VM.
/// (Subqueries are handled by `Engine::run_statement` directly, so they don't
/// need to go through the invoker.) The sequential path uses [`SeqInvoker`];
/// the parallel path uses [`PureInvoker`], which is `Sync` and rejects the call
/// (unreachable — purity is checked before the parallel path is taken).
trait Invoker {
    fn invoke_custom(&self, f: &CfmlValue, args: Vec<CfmlValue>) -> CfmlResult;
}

/// Sequential invoker: forwards to the VM callback. Holds it behind a `RefCell`
/// so the engine is `&self` throughout; single-threaded, so the borrow is never
/// contended.
struct SeqInvoker<'a> {
    udf: RefCell<&'a mut dyn FnMut(&CfmlValue, Vec<CfmlValue>) -> CfmlResult>,
}

impl Invoker for SeqInvoker<'_> {
    fn invoke_custom(&self, f: &CfmlValue, args: Vec<CfmlValue>) -> CfmlResult {
        let mut udf = self.udf.borrow_mut();
        (&mut **udf)(f, args)
    }
}

/// Pure invoker for the parallel path: zero-sized and `Sync`. Reaching either
/// method means the purity check was wrong, so it's a hard internal error.
/// Only the rayon path constructs it, so it doesn't exist on wasm32.
#[cfg(not(target_arch = "wasm32"))]
struct PureInvoker;

#[cfg(not(target_arch = "wasm32"))]
impl Invoker for PureInvoker {
    fn invoke_custom(&self, _f: &CfmlValue, _args: Vec<CfmlValue>) -> CfmlResult {
        Err(CfmlError::runtime(
            "Query of Queries: custom function reached the parallel path (internal error)".to_string(),
        ))
    }
}

/// Whether `stmt` can be evaluated on the parallel (pure) path: no expression
/// calls a registered custom UDF, and there are no subqueries or derived tables.
fn is_parallel_safe(stmt: &SelectStatement, registry: &QoQFunctionRegistry) -> bool {
    select_is_pure(stmt, registry)
}

fn select_is_pure(s: &SelectStatement, reg: &QoQFunctionRegistry) -> bool {
    core_is_pure(&s.body, reg)
        && s.unions.iter().all(|u| core_is_pure(&u.select, reg))
        && s.order_by.iter().all(|ob| expr_is_pure(&ob.expr, reg))
}

fn core_is_pure(c: &SelectCore, reg: &QoQFunctionRegistry) -> bool {
    // A derived table (subquery in FROM) forces the sequential path.
    if let Some(from) = &c.from {
        if !tableref_is_pure(from, reg) {
            return false;
        }
    }
    c.joins.iter().all(|j| {
        tableref_is_pure(&j.table, reg) && j.on.as_ref().map(|on| expr_is_pure(on, reg)).unwrap_or(true)
    }) && c.where_clause.as_ref().map(|w| expr_is_pure(w, reg)).unwrap_or(true)
        && c.group_by.iter().all(|g| expr_is_pure(g, reg))
        && c.having.as_ref().map(|h| expr_is_pure(h, reg)).unwrap_or(true)
        && c.columns.iter().all(|col| expr_is_pure(&col.expr, reg))
}

fn tableref_is_pure(t: &TableRef, _reg: &QoQFunctionRegistry) -> bool {
    matches!(t, TableRef::Named { .. })
}

fn expr_is_pure(e: &Expr, reg: &QoQFunctionRegistry) -> bool {
    match e {
        // A custom UDF call needs the VM callback → not parallel-safe.
        Expr::Function { name, args, .. } => {
            // An extension's function is not assumed pure: it may read a scope
            // or call CFML, so it must not be hoisted out of a row loop.
            reg.get_custom(name).is_none()
                && !reg.dynamics.contains_key(&name.to_lowercase())
                && args.iter().all(|a| expr_is_pure(a, reg))
        }
        // Any subquery forces the sequential path.
        Expr::ScalarSubquery(_) | Expr::InSubquery { .. } => false,
        Expr::Binary { left, right, .. } => expr_is_pure(left, reg) && expr_is_pure(right, reg),
        Expr::Unary { expr, .. } | Expr::Cast { expr, .. } | Expr::IsNull { expr, .. } => {
            expr_is_pure(expr, reg)
        }
        Expr::Case { operand, whens, else_expr } => {
            operand.as_ref().map(|o| expr_is_pure(o, reg)).unwrap_or(true)
                && whens.iter().all(|w| expr_is_pure(&w.when, reg) && expr_is_pure(&w.then, reg))
                && else_expr.as_ref().map(|e| expr_is_pure(e, reg)).unwrap_or(true)
        }
        Expr::InList { expr, list, .. } => {
            expr_is_pure(expr, reg) && list.iter().all(|e| expr_is_pure(e, reg))
        }
        Expr::Between { expr, low, high, .. } => {
            expr_is_pure(expr, reg) && expr_is_pure(low, reg) && expr_is_pure(high, reg)
        }
        Expr::Like { expr, pattern, escape, .. } => {
            expr_is_pure(expr, reg)
                && expr_is_pure(pattern, reg)
                && escape.as_ref().map(|e| expr_is_pure(e, reg)).unwrap_or(true)
        }
        Expr::Star { .. }
        | Expr::Column { .. }
        | Expr::ResolvedColumn { .. }
        | Expr::Literal(_)
        | Expr::Param(_) => true,
    }
}

/// Collect every named (query-variable) table referenced anywhere in the
/// statement — including derived-table bodies and subqueries — so the VM can
/// resolve them from CFML scope before executing. Case-insensitive, first
/// spelling wins.
pub fn base_table_names(stmt: &Statement) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    let Statement::Select(s) = stmt;
    walk_select(s, &mut out, &mut seen);
    out
}

fn walk_select(s: &SelectStatement, out: &mut Vec<String>, seen: &mut HashSet<String>) {
    walk_core(&s.body, out, seen);
    for u in &s.unions {
        walk_core(&u.select, out, seen);
    }
    for ob in &s.order_by {
        walk_expr(&ob.expr, out, seen);
    }
}

fn walk_core(c: &SelectCore, out: &mut Vec<String>, seen: &mut HashSet<String>) {
    if let Some(t) = &c.from {
        walk_tableref(t, out, seen);
    }
    for j in &c.joins {
        walk_tableref(&j.table, out, seen);
        if let Some(on) = &j.on {
            walk_expr(on, out, seen);
        }
    }
    if let Some(w) = &c.where_clause {
        walk_expr(w, out, seen);
    }
    for g in &c.group_by {
        walk_expr(g, out, seen);
    }
    if let Some(h) = &c.having {
        walk_expr(h, out, seen);
    }
    for col in &c.columns {
        walk_expr(&col.expr, out, seen);
    }
}

fn walk_tableref(t: &TableRef, out: &mut Vec<String>, seen: &mut HashSet<String>) {
    match t {
        TableRef::Named { name, .. } => {
            if seen.insert(name.to_lowercase()) {
                out.push(name.clone());
            }
        }
        TableRef::Derived { select, .. } => walk_select(select, out, seen),
    }
}

fn walk_expr(e: &Expr, out: &mut Vec<String>, seen: &mut HashSet<String>) {
    match e {
        Expr::Binary { left, right, .. } => {
            walk_expr(left, out, seen);
            walk_expr(right, out, seen);
        }
        Expr::Unary { expr, .. } => walk_expr(expr, out, seen),
        Expr::Function { args, .. } => {
            for a in args {
                walk_expr(a, out, seen);
            }
        }
        Expr::Case {
            operand,
            whens,
            else_expr,
        } => {
            if let Some(o) = operand {
                walk_expr(o, out, seen);
            }
            for w in whens {
                walk_expr(&w.when, out, seen);
                walk_expr(&w.then, out, seen);
            }
            if let Some(e) = else_expr {
                walk_expr(e, out, seen);
            }
        }
        Expr::Cast { expr, .. } => walk_expr(expr, out, seen),
        Expr::IsNull { expr, .. } => walk_expr(expr, out, seen),
        Expr::InList { expr, list, .. } => {
            walk_expr(expr, out, seen);
            for e in list {
                walk_expr(e, out, seen);
            }
        }
        Expr::InSubquery { expr, select, .. } => {
            walk_expr(expr, out, seen);
            walk_select(select, out, seen);
        }
        Expr::Between {
            expr, low, high, ..
        } => {
            walk_expr(expr, out, seen);
            walk_expr(low, out, seen);
            walk_expr(high, out, seen);
        }
        Expr::Like {
            expr,
            pattern,
            escape,
            ..
        } => {
            walk_expr(expr, out, seen);
            walk_expr(pattern, out, seen);
            if let Some(e) = escape {
                walk_expr(e, out, seen);
            }
        }
        Expr::ScalarSubquery(select) => walk_select(select, out, seen),
        Expr::Star { .. }
        | Expr::Column { .. }
        | Expr::ResolvedColumn { .. }
        | Expr::Literal(_)
        | Expr::Param(_) => {}
    }
}

/// Evaluation context: a single row (scalar) or a partition (aggregate).
#[derive(Clone, Copy)]
enum RowCtx<'b> {
    Row(&'b [usize]),
    Group(&'b Intersections),
}

/// Result of executing one SELECT core. Column-major end-to-end: every
/// downstream stage (UNION extend, dedup, sort, limit, build_query) operates
/// on `Vec<Vec<CfmlValue>>` where `data[ci]` is the column ci's values and
/// has length `row_count`. This lets `exec_no_join_fused` build passthrough
/// columns by indexing the source column array directly — no per-row eval,
/// no per-cell `CfmlValue::clone()` chain overhead, no row-major→col-major
/// transpose at the very end.
struct CoreResult {
    columns: Vec<String>,
    row_count: usize,
    /// Column-major output: `data[ci]` has length `row_count`.
    data: Vec<Vec<CfmlValue>>,
    /// Column-major ORDER BY keys: `sort_keys[k]` has length `row_count`
    /// (outer Vec empty when no statement-level ORDER BY was supplied to
    /// the core).
    sort_keys: Vec<Vec<CfmlValue>>,
}

struct Engine<'a, I: Invoker> {
    sources: &'a [(String, &'a CfmlQuery)],
    params: &'a QoQParams,
    registry: &'a QoQFunctionRegistry,
    inv: &'a I,
    /// Whether the whole statement is parallel-safe (set once in `execute`).
    /// Only read by the rayon path, so it's dead on wasm32.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    parallel: bool,
}

impl<'a, I: Invoker> Engine<'a, I> {
    // ── Statement / core ───────────────────────────────────────────────

    fn run_statement(&self, stmt: &SelectStatement) -> Result<CfmlQueryData, CfmlError> {
        // ORDER BY keys are computed in-context only for the single-core case;
        // for UNION, ordering is by output column post-merge.
        let body_order: &[OrderByExpr] = if stmt.unions.is_empty() {
            &stmt.order_by
        } else {
            &[]
        };

        let body = self.run_core(&stmt.body, body_order)?;
        let columns = body.columns;
        let mut data = body.data;
        let mut row_count = body.row_count;

        if stmt.unions.is_empty() {
            let sort_keys = body.sort_keys;
            sort_cols(&mut data, &sort_keys, &stmt.order_by, row_count);
            row_count = apply_limit_cols(&mut data, &stmt.limit, row_count);
            return Ok(build_query(columns, data, row_count));
        }

        // UNION: extend each output column with the corresponding arm column.
        let any_distinct = stmt.unions.iter().any(|u| !u.all);
        for u in &stmt.unions {
            let arm = self.run_core(&u.select, &[])?;
            if arm.columns.len() != columns.len() {
                return Err(CfmlError::runtime(format!(
                    "Query of Queries: UNION column count mismatch ({} vs {})",
                    columns.len(),
                    arm.columns.len()
                )));
            }
            for (ci, arm_col) in arm.data.into_iter().enumerate() {
                data[ci].extend(arm_col);
            }
            row_count += arm.row_count;
        }
        if any_distinct {
            row_count = dedup_cols(&mut data, row_count);
        }
        let mut sort_keys: Vec<Vec<CfmlValue>> = Vec::new();
        self.build_output_sort_keys(&data, row_count, &columns, &stmt.order_by, &mut sort_keys)?;
        sort_cols(&mut data, &sort_keys, &stmt.order_by, row_count);
        row_count = apply_limit_cols(&mut data, &stmt.limit, row_count);
        Ok(build_query(columns, data, row_count))
    }

    fn run_core(
        &self,
        core_in: &SelectCore,
        order_by_in: &[OrderByExpr],
    ) -> Result<CoreResult, CfmlError> {
        let tables = self.resolve_tables(core_in)?;

        if tables.is_empty() {
            return self.run_no_from(core_in, order_by_in);
        }

        // Clone the core + ORDER BY so we can rewrite column refs into
        // pre-resolved `(ti, ci)` slots before any per-row evaluation. The
        // clone is O(expression-tree) — microseconds — vs the per-row
        // `resolve_column` linear scan it eliminates over 1M rows.
        let mut core: SelectCore = core_in.clone();
        let mut order_by: Vec<OrderByExpr> = order_by_in.to_vec();

        let mut select_cols = expand_columns(&core.columns, &tables)?;
        bind_core(&mut core, &mut select_cols, &mut order_by, &tables);
        let columns = derive_column_names(&select_cols);

        // Compile the WHERE clause once into the specialised CompiledExpr form
        // (see `compiled.rs`); the per-row evaluator dispatches on it in fewer
        // match arms than the full AST eval. Unrecognised subtrees fall through
        // to the existing AST evaluator via `CompiledExpr::Generic`.
        let compiled_where: Option<CompiledExpr> = core.where_clause.as_ref().map(compiled::compile);

        let has_agg = select_cols.iter().any(|c| expr_has_aggregate(&c.expr, self.registry))
            || core
                .having
                .as_ref()
                .map(|h| expr_has_aggregate(h, self.registry))
                .unwrap_or(false);

        // Fast path — single table, no joins, no aggregation: fuse WHERE +
        // projection + sort-key build into one parallel pass over the source
        // row indices. Skips:
        //   • the 1M-element `Vec<Vec<usize>>` intersection seed allocation
        //   • the separate `filter_where` materialisation pass
        //   • per-row 1-element `Vec<usize>` intersection allocations
        // Single-row `inter` lives on the stack (`[r]`) inside the worker.
        let can_fused = !has_agg
            && core.group_by.is_empty()
            && core.having.is_none()
            && core.joins.is_empty()
            && tables.tables.len() == 1;

        let (mut data, mut sort_keys, mut row_count) = if can_fused {
            self.exec_no_join_fused(&select_cols, compiled_where.as_ref(), &tables, &order_by)?
        } else {
            let intersections = self.build_core_intersections(&core, &tables)?;
            let filtered = self.filter_where(intersections, compiled_where.as_ref(), &tables)?;
            if has_agg || !core.group_by.is_empty() {
                self.exec_aggregate(&core, &select_cols, &filtered, &tables, &order_by)?
            } else {
                self.exec_simple(&select_cols, &filtered, &tables, &order_by)?
            }
        };

        if core.distinct {
            row_count = dedup_cols_and_keys(&mut data, &mut sort_keys, row_count);
        }
        if let Some(n) = core.top {
            if row_count > n {
                truncate_cols(&mut data, n);
                truncate_cols(&mut sort_keys, n);
                row_count = n;
            }
        }

        Ok(CoreResult {
            columns,
            row_count,
            data,
            sort_keys,
        })
    }

    fn run_no_from(
        &self,
        core: &SelectCore,
        order_by: &[OrderByExpr],
    ) -> Result<CoreResult, CfmlError> {
        let tables = TableSet::new();
        let inter: Vec<usize> = Vec::new();
        let mut row = Vec::with_capacity(core.columns.len());
        for sc in &core.columns {
            if matches!(sc.expr, Expr::Star { .. }) {
                return Err(CfmlError::runtime(
                    "Query of Queries: '*' requires a FROM clause".to_string(),
                ));
            }
            row.push(self.eval(&sc.expr, &tables, RowCtx::Row(&inter))?);
        }
        let columns = derive_column_names(&core.columns);
        let mut key = Vec::with_capacity(order_by.len());
        for ob in order_by {
            key.push(self.order_key(ob, &tables, RowCtx::Row(&inter), &row)?);
        }
        // 1-row output → each column is a 1-element Vec.
        let data: Vec<Vec<CfmlValue>> = row.into_iter().map(|v| vec![v]).collect();
        let sort_keys: Vec<Vec<CfmlValue>> = key.into_iter().map(|v| vec![v]).collect();
        Ok(CoreResult {
            columns,
            row_count: 1,
            data,
            sort_keys,
        })
    }

    fn resolve_tables(&self, core: &SelectCore) -> Result<TableSet, CfmlError> {
        let mut ts = TableSet::new();
        if let Some(from) = &core.from {
            ts.add(self.resolve_table_ref(from)?);
            for j in &core.joins {
                ts.add(self.resolve_table_ref(&j.table)?);
            }
        }
        Ok(ts)
    }

    fn resolve_table_ref(&self, tref: &TableRef) -> Result<QoQTable, CfmlError> {
        match tref {
            TableRef::Named { name, alias } => {
                let query = self
                    .sources
                    .iter()
                    .find(|(n, _)| n.eq_ignore_ascii_case(name))
                    .map(|(_, q)| *q)
                    .ok_or_else(|| {
                        CfmlError::runtime(format!(
                            "Query of Queries: table '{}' not found (no such query variable)",
                            name
                        ))
                    })?;
                let binding = alias.clone().unwrap_or_else(|| name.clone());
                // Read the shared source handle under a guard once, column-major.
                Ok(query.with_read(|d| QoQTable::from_query_data(&binding, d)))
            }
            TableRef::Derived { select, alias } => {
                let q = self.run_statement(select)?;
                Ok(QoQTable::from_query_data(alias, &q))
            }
        }
    }

    fn build_core_intersections(
        &self,
        core: &SelectCore,
        tables: &TableSet,
    ) -> Result<Intersections, CfmlError> {
        // Hash-join fast path (H4): every INNER JOIN with a single equi
        // `ResolvedColumn = ResolvedColumn` ON folds in O(L + R) instead of
        // the generic nested-loop's O(L × R). Falls back to the generic
        // builder for non-equi / OUTER / cross / multi-clause ONs.
        if let Some(result) = self.try_hash_join_chain(core, tables) {
            return result;
        }

        let row_counts = tables.row_counts();
        let join_types: Vec<JoinType> = core.joins.iter().map(|j| j.join_type).collect();
        let joins = &core.joins;
        let mut on_err: Option<CfmlError> = None;

        let result = build_intersections(&row_counts, &join_types, MAX_INTERSECTIONS, |k, cand| {
            if on_err.is_some() {
                return false;
            }
            match &joins[k].on {
                None => true, // CROSS / comma join
                Some(expr) => match self.eval(expr, tables, RowCtx::Row(cand)) {
                    Ok(v) => is_truthy(&v),
                    Err(e) => {
                        on_err = Some(e);
                        false
                    }
                },
            }
        });

        if let Some(e) = on_err {
            return Err(e);
        }
        result.map_err(|size| {
            CfmlError::runtime(format!(
                "Query of Queries: join would materialise {} row combinations, exceeding the limit \
                 of {}. Add an explicit `JOIN ... ON` (filtered as it builds) or reduce the data.",
                size, MAX_INTERSECTIONS
            ))
        })
    }

    /// Hash-join detector + driver. Returns `Some(Ok(intersections))` for two
    /// patterns:
    ///
    /// 1. **Explicit equi-joins** — every `JOIN` in `core.joins` is
    ///    `INNER ... ON ResolvedCol = ResolvedCol`, with one side pointing at
    ///    the newly-added right table and the other at any already-joined
    ///    table.
    /// 2. **Comma joins with WHERE-pushdown** — every join is CROSS (`FROM a, b,
    ///    c`), and the WHERE clause is an AND-chain that includes an equi
    ///    predicate for each new table (linking it to an already-joined one).
    ///    The equi predicates are used as implicit join keys; the rest of the
    ///    WHERE is still evaluated per row by `filter_where`. Re-evaluating the
    ///    consumed predicates is correct (they're satisfied by construction
    ///    after the hash probe) — at worst, a few microseconds of redundant
    ///    work per surviving row.
    ///
    /// `None` ⇒ fall back to the generic nested-loop `build_intersections`
    /// (OUTER joins, non-equi ONs, expressions, unbound columns, cross joins
    /// without WHERE-pushdown predicates).
    fn try_hash_join_chain(
        &self,
        core: &SelectCore,
        tables: &TableSet,
    ) -> Option<Result<Intersections, CfmlError>> {
        if core.joins.is_empty() {
            return None;
        }

        // Pattern 1: every join has its own equi ON.
        if core.joins.iter().all(|j| j.join_type == JoinType::Inner && j.on.is_some()) {
            let mut probes: Vec<(usize, usize, usize, usize)> =
                Vec::with_capacity(core.joins.len());
            let mut ok = true;
            for (k, j) in core.joins.iter().enumerate() {
                let on = j.on.as_ref().unwrap();
                let Some((lt, lc, rt, rc)) = equi_pair(on) else { ok = false; break };
                let new_ti = k + 1;
                let probe = if lt == new_ti && rt <= k {
                    (rt, rc, lt, lc)
                } else if rt == new_ti && lt <= k {
                    (lt, lc, rt, rc)
                } else {
                    ok = false;
                    break;
                };
                probes.push(probe);
            }
            if ok {
                return Some(self.hash_join_chain(tables, &probes));
            }
        }

        // Pattern 2: comma joins (every join is CROSS with no ON) — pull join
        // keys out of the WHERE clause.
        let all_comma = core
            .joins
            .iter()
            .all(|j| j.join_type == JoinType::Cross && j.on.is_none());
        if !all_comma {
            return None;
        }
        let where_eq = collect_equi_conjuncts(core.where_clause.as_ref());
        if where_eq.is_empty() {
            return None;
        }
        let mut probes: Vec<(usize, usize, usize, usize)> =
            Vec::with_capacity(core.joins.len());
        let mut used = vec![false; where_eq.len()];
        for k in 0..core.joins.len() {
            let new_ti = k + 1;
            let mut found: Option<(usize, (usize, usize, usize, usize))> = None;
            for (i, &(t1, c1, t2, c2)) in where_eq.iter().enumerate() {
                if used[i] {
                    continue;
                }
                let probe = if t1 == new_ti && t2 <= k {
                    Some((t2, c2, t1, c1))
                } else if t2 == new_ti && t1 <= k {
                    Some((t1, c1, t2, c2))
                } else {
                    None
                };
                if let Some(p) = probe {
                    found = Some((i, p));
                    break;
                }
            }
            let (idx, probe) = found?;
            used[idx] = true;
            probes.push(probe);
        }
        Some(self.hash_join_chain(tables, &probes))
    }

    fn hash_join_chain(
        &self,
        tables: &TableSet,
        probes: &[(usize, usize, usize, usize)],
    ) -> Result<Intersections, CfmlError> {
        use std::collections::HashMap;
        let row0 = tables.tables[0].row_count;
        let mut inters = Intersections::with_capacity(1, row0);
        for r in 1..=row0 {
            inters.flat.push(r);
        }
        for (k, &(lt, lc, rt, rc)) in probes.iter().enumerate() {
            let right_tbl = &tables.tables[rt];
            // Hash index: stringified right-key → list of 1-based right rows.
            let mut idx: HashMap<String, Vec<usize>> =
                HashMap::with_capacity(right_tbl.row_count);
            for r in 1..=right_tbl.row_count {
                let key = group_key(&[right_tbl.get(r, rc)]);
                idx.entry(key).or_default().push(r);
            }
            let new_width = k + 2;
            let mut next = Intersections::with_capacity(new_width, inters.len());
            for inter in inters.iter() {
                let lkey = group_key(&[tables.value(inter, lt, lc)]);
                if let Some(rows) = idx.get(&lkey) {
                    for &r in rows {
                        next.flat.extend_from_slice(inter);
                        next.flat.push(r);
                    }
                }
            }
            inters = next;
        }
        Ok(inters)
    }

    /// A `Sync` companion engine sharing this query's tables/params/registry but
    /// using the pure (UDF-rejecting) invoker — the evaluator handed to rayon.
    /// Only built when `self.parallel` (the statement is pure), so its `eval`
    /// never reaches `invoke_custom`.
    #[cfg(not(target_arch = "wasm32"))]
    fn pure_engine<'p>(&'p self, pure: &'p PureInvoker) -> Engine<'p, PureInvoker> {
        Engine {
            sources: self.sources,
            params: self.params,
            registry: self.registry,
            inv: pure,
            parallel: false,
        }
    }

    fn filter_where(
        &self,
        intersections: Intersections,
        where_clause: Option<&CompiledExpr>,
        tables: &TableSet,
    ) -> Result<Intersections, CfmlError> {
        let Some(expr) = where_clause else {
            return Ok(intersections);
        };
        let width = intersections.width;
        // Pure statement + many rows → evaluate the predicate across cores.
        #[cfg(not(target_arch = "wasm32"))]
        if self.parallel && intersections.len() >= PARALLEL_ROW_THRESHOLD {
            use rayon::prelude::*;
            let pure = PureInvoker;
            let pe = self.pure_engine(&pure);
            let n_workers = rayon::current_num_threads().max(1);
            let rows_per_chunk = (intersections.len() / (n_workers * 4)).max(1024);
            let chunk_results: Result<Vec<Vec<usize>>, CfmlError> = intersections
                .par_chunks_rows(rows_per_chunk)
                .map(|chunk| {
                    let mut keep = Vec::new();
                    for inter in chunk.iter() {
                        let v = pe.eval_compiled(expr, tables, RowCtx::Row(inter))?;
                        if is_truthy(&v) {
                            keep.extend_from_slice(inter);
                        }
                    }
                    Ok(keep)
                })
                .collect();
            let chunk_results = chunk_results?;
            let total: usize = chunk_results.iter().map(|c| c.len()).sum();
            let mut flat = Vec::with_capacity(total);
            for c in chunk_results {
                flat.extend(c);
            }
            return Ok(Intersections { width, flat });
        }
        let mut out = Intersections::new(width);
        for inter in intersections.iter() {
            let v = self.eval_compiled(expr, tables, RowCtx::Row(inter))?;
            if is_truthy(&v) {
                out.push_row(inter);
            }
        }
        Ok(out)
    }

    /// Multi-table / joined exec path. Like `exec_no_join_fused`, but each
    /// "row" is an intersection (`Vec<usize>`) of 1-based row indices, one per
    /// table. Writes column-major directly via chunked parallel evaluation;
    /// each cell is moved (not cloned) from the eval result into its output
    /// column.
    fn exec_simple(
        &self,
        select_cols: &[SelectColumn],
        intersections: &Intersections,
        tables: &TableSet,
        order_by: &[OrderByExpr],
    ) -> Result<(Vec<Vec<CfmlValue>>, Vec<Vec<CfmlValue>>, usize), CfmlError> {
        let n_rows = intersections.len();
        let n_cols = select_cols.len();
        let n_keys = order_by.len();

        // ORDER BY's bare-int-literal alias (`ORDER BY 1`) needs to read back
        // the just-projected row's k-th cell. Pre-compute the alias slot per
        // sort key (None for normal expression keys).
        let int_alias: Vec<Option<usize>> = order_by
            .iter()
            .map(|ob| {
                if let Expr::Literal(CfmlValue::Int(n)) = &ob.expr {
                    Some((*n - 1).max(0) as usize)
                } else {
                    None
                }
            })
            .collect();

        #[cfg(not(target_arch = "wasm32"))]
        if self.parallel && n_rows >= PARALLEL_ROW_THRESHOLD {
            use rayon::prelude::*;
            let pure = PureInvoker;
            let pe = self.pure_engine(&pure);
            let n_workers = rayon::current_num_threads().max(1);
            let chunk_size = (n_rows / (n_workers * 4)).max(1024);
            type Chunk = (Vec<Vec<CfmlValue>>, Vec<Vec<CfmlValue>>);
            let chunk_results: Result<Vec<Chunk>, CfmlError> = intersections
                .par_chunks_rows(chunk_size)
                .map(|chunk| {
                    let len = chunk.len();
                    let mut data: Vec<Vec<CfmlValue>> =
                        (0..n_cols).map(|_| Vec::with_capacity(len)).collect();
                    let mut keys: Vec<Vec<CfmlValue>> =
                        (0..n_keys).map(|_| Vec::with_capacity(len)).collect();
                    for inter in chunk.iter() {
                        let ctx = RowCtx::Row(inter);
                        for (ci, sc) in select_cols.iter().enumerate() {
                            data[ci].push(pe.eval(&sc.expr, tables, ctx)?);
                        }
                        for (k, ob) in order_by.iter().enumerate() {
                            let v = match int_alias[k] {
                                Some(idx) => data
                                    .get(idx)
                                    .and_then(|c| c.last())
                                    .cloned()
                                    .unwrap_or(CfmlValue::Null),
                                None => pe.eval(&ob.expr, tables, ctx)?,
                            };
                            keys[k].push(v);
                        }
                    }
                    Ok((data, keys))
                })
                .collect();
            let chunk_results = chunk_results?;
            let mut data: Vec<Vec<CfmlValue>> =
                (0..n_cols).map(|_| Vec::with_capacity(n_rows)).collect();
            let mut keys: Vec<Vec<CfmlValue>> =
                (0..n_keys).map(|_| Vec::with_capacity(n_rows)).collect();
            for (chunk_data, chunk_keys) in chunk_results {
                for (ci, col) in chunk_data.into_iter().enumerate() {
                    data[ci].extend(col);
                }
                for (k, col) in chunk_keys.into_iter().enumerate() {
                    keys[k].extend(col);
                }
            }
            return Ok((data, keys, n_rows));
        }

        // Sequential fallback.
        let mut data: Vec<Vec<CfmlValue>> =
            (0..n_cols).map(|_| Vec::with_capacity(n_rows)).collect();
        let mut keys: Vec<Vec<CfmlValue>> =
            (0..n_keys).map(|_| Vec::with_capacity(n_rows)).collect();
        for inter in intersections.iter() {
            let ctx = RowCtx::Row(inter);
            for (ci, sc) in select_cols.iter().enumerate() {
                data[ci].push(self.eval(&sc.expr, tables, ctx)?);
            }
            for (k, ob) in order_by.iter().enumerate() {
                let v = match int_alias[k] {
                    Some(idx) => data
                        .get(idx)
                        .and_then(|c| c.last())
                        .cloned()
                        .unwrap_or(CfmlValue::Null),
                    None => self.eval(&ob.expr, tables, ctx)?,
                };
                keys[k].push(v);
            }
        }
        Ok((data, keys, n_rows))
    }

    /// Single-table, no-join, no-aggregate fast path. Two phases:
    ///
    /// 1. **Filter**: build `survivors: Vec<usize>` of 1-based source-row
    ///    indices that pass WHERE. Parallel across the row range when
    ///    eligible.
    /// 2. **Column build** (one rayon worker per output column + per sort
    ///    key): if the expression is a bare `Expr::ResolvedColumn{ti=0, ci}`,
    ///    the output column is built by indexing the source column directly —
    ///    no per-cell `eval()` dispatch chain. Otherwise the column is built
    ///    by per-row eval. No per-row `Vec<CfmlValue>` allocations.
    fn exec_no_join_fused(
        &self,
        select_cols: &[SelectColumn],
        where_compiled: Option<&CompiledExpr>,
        tables: &TableSet,
        order_by: &[OrderByExpr],
    ) -> Result<(Vec<Vec<CfmlValue>>, Vec<Vec<CfmlValue>>, usize), CfmlError> {
        let row_count = tables.tables[0].row_count;

        // Phase 1: filter into a flat `Vec<usize>` of surviving 1-based row
        // indices. With a parallel-safe statement and enough rows, fan out the
        // WHERE predicate across cores.
        let survivors: Vec<usize> = {
            #[cfg(not(target_arch = "wasm32"))]
            {
                if self.parallel && row_count >= PARALLEL_ROW_THRESHOLD {
                    use rayon::prelude::*;
                    let pure = PureInvoker;
                    let pe = self.pure_engine(&pure);
                    if let Some(w) = where_compiled {
                        let result: Result<Vec<Option<usize>>, CfmlError> = (1..=row_count)
                            .into_par_iter()
                            .map(|r| {
                                let inter = [r];
                                let v = pe.eval_compiled(w, tables, RowCtx::Row(&inter))?;
                                Ok(if is_truthy(&v) { Some(r) } else { None })
                            })
                            .collect();
                        result?.into_iter().flatten().collect()
                    } else {
                        (1..=row_count).collect()
                    }
                } else {
                    let mut out = Vec::with_capacity(row_count);
                    if let Some(w) = where_compiled {
                        for r in 1..=row_count {
                            let inter = [r];
                            let v = self.eval_compiled(w, tables, RowCtx::Row(&inter))?;
                            if is_truthy(&v) {
                                out.push(r);
                            }
                        }
                    } else {
                        out.extend(1..=row_count);
                    }
                    out
                }
            }
            #[cfg(target_arch = "wasm32")]
            {
                let mut out = Vec::with_capacity(row_count);
                if let Some(w) = where_compiled {
                    for r in 1..=row_count {
                        let inter = [r];
                        let v = self.eval_compiled(w, tables, RowCtx::Row(&inter))?;
                        if is_truthy(&v) {
                            out.push(r);
                        }
                    }
                } else {
                    out.extend(1..=row_count);
                }
                out
            }
        };

        // Phase 2a: build each output projection column. The build can
        // parallelise across columns when the statement is parallel-safe and
        // the survivor set is large; otherwise sequential.
        let n_surv = survivors.len();
        let data = self.build_cols_for_exprs(
            select_cols.iter().map(|sc| &sc.expr),
            &survivors,
            tables,
        )?;

        // Phase 2b: sort keys. `order_key` special-cases bare integer literals
        // as 1-based references into the projected row — handle that here by
        // aliasing the corresponding output column rather than re-evaluating.
        let mut sort_keys: Vec<Vec<CfmlValue>> = Vec::with_capacity(order_by.len());
        // Collect non-literal exprs to bulk-build, preserving slot positions.
        let mut slot_for_expr: Vec<usize> = Vec::new();
        let mut exprs_to_eval: Vec<&Expr> = Vec::new();
        for (k, ob) in order_by.iter().enumerate() {
            if let Expr::Literal(CfmlValue::Int(n)) = &ob.expr {
                let idx = (*n - 1).max(0) as usize;
                sort_keys.push(
                    data.get(idx)
                        .cloned()
                        .unwrap_or_else(|| vec![CfmlValue::Null; n_surv]),
                );
            } else {
                slot_for_expr.push(k);
                exprs_to_eval.push(&ob.expr);
                sort_keys.push(Vec::new()); // placeholder
            }
        }
        if !exprs_to_eval.is_empty() {
            let built = self.build_cols_for_exprs(exprs_to_eval.into_iter(), &survivors, tables)?;
            for (slot, col) in slot_for_expr.into_iter().zip(built.into_iter()) {
                sort_keys[slot] = col;
            }
        }

        Ok((data, sort_keys, n_surv))
    }

    /// Build one output column per expression, indexing into `survivors`
    /// (1-based source-row indices, with 0 meaning the NULL sentinel for
    /// outer-join misses — irrelevant in the no-join fused path).
    ///
    /// Passthrough fast-path: `Expr::ResolvedColumn{ti, ci}` skips the eval
    /// dispatch chain and reads the source column array directly. For Q1
    /// this fires on 10 of 12 select columns and all 3 ORDER BY keys.
    fn build_cols_for_exprs<'e, It>(
        &self,
        exprs: It,
        survivors: &[usize],
        tables: &TableSet,
    ) -> Result<Vec<Vec<CfmlValue>>, CfmlError>
    where
        It: IntoIterator<Item = &'e Expr>,
    {
        let exprs: Vec<&Expr> = exprs.into_iter().collect();
        let n_surv = survivors.len();
        let n_exprs = exprs.len();
        let mut out: Vec<Option<Vec<CfmlValue>>> = (0..n_exprs).map(|_| None).collect();

        // Sort exprs into two buckets:
        //   • Passthrough — `Expr::ResolvedColumn{ti, ci}` — built by direct
        //     column indexing, no eval dispatch.
        //   • Everything else — evaluated.
        let mut needs_eval: Vec<usize> = Vec::new();
        for (i, e) in exprs.iter().enumerate() {
            if !matches!(e, Expr::ResolvedColumn { .. }) {
                needs_eval.push(i);
            }
        }

        // 1. Passthrough columns. With many survivors, build each column with
        //    rayon par_iter; otherwise sequential. The work is a cheap
        //    `Arc<Vec<CfmlValue>>` indexed clone per cell, so spinning up a
        //    parallel pass is only worthwhile above the row threshold.
        for (i, e) in exprs.iter().enumerate() {
            if let Expr::ResolvedColumn { ti, ci, .. } = e {
                let src = &tables.tables[*ti as usize].data[*ci as usize];
                let col: Vec<CfmlValue> = {
                    #[cfg(not(target_arch = "wasm32"))]
                    {
                        if self.parallel && n_surv >= PARALLEL_ROW_THRESHOLD {
                            use rayon::prelude::*;
                            survivors
                                .par_iter()
                                .map(|&r| passthrough_cell(src, r))
                                .collect()
                        } else {
                            survivors
                                .iter()
                                .map(|&r| passthrough_cell(src, r))
                                .collect()
                        }
                    }
                    #[cfg(target_arch = "wasm32")]
                    {
                        survivors.iter().map(|&r| passthrough_cell(src, r)).collect()
                    }
                };
                out[i] = Some(col);
            }
        }

        // 2. Non-passthrough exprs. Run ONE chunked parallel pass over the
        //    survivors that evaluates ALL of them per row, then concatenate
        //    each chunk's column-major output. This brings back the old fused
        //    per-row pattern's worker utilisation, but writes column-major
        //    so we avoid the row-major→col-major transpose at the end of the
        //    pipeline.
        if !needs_eval.is_empty() {
            #[cfg(not(target_arch = "wasm32"))]
            {
                let n_e = needs_eval.len();
                if self.parallel && n_surv >= PARALLEL_ROW_THRESHOLD {
                    use rayon::prelude::*;
                    let pure = PureInvoker;
                    let pe = self.pure_engine(&pure);
                    let n_workers = rayon::current_num_threads().max(1);
                    let chunk_size = (n_surv / (n_workers * 4)).max(1024);
                    let chunks: Vec<&[usize]> = survivors.chunks(chunk_size).collect();
                    let chunk_results: Result<Vec<Vec<Vec<CfmlValue>>>, CfmlError> = chunks
                        .par_iter()
                        .map(|chunk| {
                            let mut cols: Vec<Vec<CfmlValue>> =
                                (0..n_e).map(|_| Vec::with_capacity(chunk.len())).collect();
                            for &r in *chunk {
                                let inter = [r];
                                let ctx = RowCtx::Row(&inter);
                                for (k, &j) in needs_eval.iter().enumerate() {
                                    cols[k].push(pe.eval(exprs[j], tables, ctx)?);
                                }
                            }
                            Ok(cols)
                        })
                        .collect();
                    let chunk_results = chunk_results?;
                    let mut concat: Vec<Vec<CfmlValue>> =
                        (0..n_e).map(|_| Vec::with_capacity(n_surv)).collect();
                    for chunk in chunk_results {
                        for (k, col) in chunk.into_iter().enumerate() {
                            concat[k].extend(col);
                        }
                    }
                    for (k, &i) in needs_eval.iter().enumerate() {
                        out[i] = Some(std::mem::take(&mut concat[k]));
                    }
                } else {
                    for &i in &needs_eval {
                        let mut col = Vec::with_capacity(n_surv);
                        for &r in survivors {
                            let inter = [r];
                            col.push(self.eval(exprs[i], tables, RowCtx::Row(&inter))?);
                        }
                        out[i] = Some(col);
                    }
                }
            }
            #[cfg(target_arch = "wasm32")]
            {
                for &i in &needs_eval {
                    let mut col = Vec::with_capacity(n_surv);
                    for &r in survivors {
                        let inter = [r];
                        col.push(self.eval(exprs[i], tables, RowCtx::Row(&inter))?);
                    }
                    out[i] = Some(col);
                }
            }
        }

        Ok(out.into_iter().map(|o| o.unwrap_or_default()).collect())
    }

    fn exec_aggregate(
        &self,
        core: &SelectCore,
        select_cols: &[SelectColumn],
        intersections: &Intersections,
        tables: &TableSet,
        order_by: &[OrderByExpr],
    ) -> Result<(Vec<Vec<CfmlValue>>, Vec<Vec<CfmlValue>>, usize), CfmlError> {
        let partitions: Vec<Intersections> = if core.group_by.is_empty() {
            // Pure aggregate (no GROUP BY): one partition over all rows — even
            // if empty, which still yields one row (COUNT = 0, etc.).
            vec![intersections.clone()]
        } else {
            self.partition(intersections, &core.group_by, tables)?
        };

        let mut rows = Vec::new();
        let mut keys = Vec::new();
        for part in &partitions {
            if let Some(h) = &core.having {
                let v = self.eval(h, tables, RowCtx::Group(part))?;
                if !is_truthy(&v) {
                    continue;
                }
            }
            let mut row = Vec::with_capacity(select_cols.len());
            for sc in select_cols {
                row.push(self.eval(&sc.expr, tables, RowCtx::Group(part))?);
            }
            let mut key = Vec::with_capacity(order_by.len());
            for ob in order_by {
                key.push(self.order_key(ob, tables, RowCtx::Group(part), &row)?);
            }
            rows.push(row);
            keys.push(key);
        }
        // Aggregate result sets are typically small — transpose row-major →
        // column-major sequentially here.
        let row_count = rows.len();
        let data = rows_to_cols(rows, select_cols.len());
        let sort_keys = rows_to_cols(keys, order_by.len());
        Ok((data, sort_keys, row_count))
    }

    fn partition(
        &self,
        intersections: &Intersections,
        group_by: &[Expr],
        tables: &TableSet,
    ) -> Result<Vec<Intersections>, CfmlError> {
        let width = intersections.width;
        // Compute the group key for every row in parallel (PureInvoker, no UDFs
        // touched on the GROUP BY expressions in practice), then fold into a
        // single IndexMap sequentially to preserve first-seen order. The key
        // build is O(R·G) and was a sequential 200ms+ on the 1M-row bench.
        #[cfg(not(target_arch = "wasm32"))]
        if self.parallel && intersections.len() >= PARALLEL_ROW_THRESHOLD {
            use rayon::prelude::*;
            let pure = PureInvoker;
            let pe = self.pure_engine(&pure);
            let n_rows = intersections.len();
            let keys: Result<Vec<String>, CfmlError> = (0..n_rows)
                .into_par_iter()
                .map(|i| {
                    let inter = intersections.row(i);
                    let mut key_vals = Vec::with_capacity(group_by.len());
                    for g in group_by {
                        key_vals.push(pe.eval(g, tables, RowCtx::Row(inter))?);
                    }
                    Ok(group_key(&key_vals))
                })
                .collect();
            let keys = keys?;
            let mut groups: IndexMap<String, Intersections> = IndexMap::new();
            for (i, k) in keys.into_iter().enumerate() {
                let inter = intersections.row(i);
                groups
                    .entry(k)
                    .or_insert_with(|| Intersections::new(width))
                    .push_row(inter);
            }
            return Ok(groups.into_values().collect());
        }

        let mut groups: IndexMap<String, Intersections> = IndexMap::new();
        for inter in intersections.iter() {
            let mut key_vals = Vec::with_capacity(group_by.len());
            for g in group_by {
                key_vals.push(self.eval(g, tables, RowCtx::Row(inter))?);
            }
            let k = group_key(&key_vals);
            groups
                .entry(k)
                .or_insert_with(|| Intersections::new(width))
                .push_row(inter);
        }
        Ok(groups.into_values().collect())
    }

    /// The sort-key value for one ORDER BY item. A bare integer literal is a
    /// 1-based reference into the projected row; otherwise evaluate the expr.
    fn order_key(
        &self,
        ob: &OrderByExpr,
        tables: &TableSet,
        ctx: RowCtx,
        projected_row: &[CfmlValue],
    ) -> CfmlResult {
        if let Expr::Literal(CfmlValue::Int(n)) = &ob.expr {
            let idx = (*n - 1).max(0) as usize;
            return Ok(projected_row.get(idx).cloned().unwrap_or(CfmlValue::Null));
        }
        self.eval(&ob.expr, tables, ctx)
    }

    /// Order a UNION's merged rows by output column (position / name / expr over
    /// the output columns).
    /// Build column-major sort keys for a UNION-merged output: each key is a
    /// column of length `row_count`. Bare integer-literal ORDER BY items
    /// (`ORDER BY 1`, …) alias the corresponding output column directly; named
    /// references resolve through a synthetic single-row TableSet per row.
    fn build_output_sort_keys(
        &self,
        data: &[Vec<CfmlValue>],
        row_count: usize,
        columns: &[String],
        order_by: &[OrderByExpr],
        sort_keys: &mut Vec<Vec<CfmlValue>>,
    ) -> Result<(), CfmlError> {
        if order_by.is_empty() || row_count < 2 {
            return Ok(());
        }
        let mut cols: Vec<Vec<CfmlValue>> =
            (0..order_by.len()).map(|_| Vec::with_capacity(row_count)).collect();

        // Resolve each OB expr to a direct output-column alias when possible:
        // a bare integer literal (`ORDER BY 1`) names position k-1; a bare
        // `Expr::Column { name }` whose `name` matches one of the output
        // `columns` (case-insensitively) names that position. Hitting either
        // path lets us skip the synthetic per-row TableSet for that key —
        // the key column IS the referenced output column.
        let col_alias: Vec<Option<usize>> = order_by
            .iter()
            .map(|ob| match &ob.expr {
                Expr::Literal(CfmlValue::Int(n)) => Some((*n - 1).max(0) as usize),
                Expr::Column { name, table: None } => columns
                    .iter()
                    .position(|c| c.eq_ignore_ascii_case(name)),
                Expr::ResolvedColumn { name, .. } => columns
                    .iter()
                    .position(|c| c.eq_ignore_ascii_case(name)),
                _ => None,
            })
            .collect();

        let need_synth = col_alias.iter().any(|x| x.is_none());
        // Pre-fill aliased keys: the key column IS the referenced output
        // column. For wide output (~1M rows), parallelise the per-key clone
        // across cores so each aliased key gets its own worker.
        #[cfg(not(target_arch = "wasm32"))]
        if row_count >= PARALLEL_ROW_THRESHOLD * 8 {
            use rayon::prelude::*;
            let built: Vec<(usize, Vec<CfmlValue>)> = col_alias
                .par_iter()
                .enumerate()
                .filter_map(|(k, alias)| {
                    alias.map(|idx| {
                        let v = data
                            .get(idx)
                            .cloned()
                            .unwrap_or_else(|| vec![CfmlValue::Null; row_count]);
                        (k, v)
                    })
                })
                .collect();
            for (k, v) in built {
                cols[k] = v;
            }
        } else {
            for (k, alias) in col_alias.iter().enumerate() {
                if let Some(idx) = alias {
                    if let Some(src) = data.get(*idx) {
                        cols[k] = src.clone();
                    } else {
                        cols[k] = vec![CfmlValue::Null; row_count];
                    }
                }
            }
        }
        #[cfg(target_arch = "wasm32")]
        for (k, alias) in col_alias.iter().enumerate() {
            if let Some(idx) = alias {
                if let Some(src) = data.get(*idx) {
                    cols[k] = src.clone();
                } else {
                    cols[k] = vec![CfmlValue::Null; row_count];
                }
            }
        }

        if need_synth {
            for i in 0..row_count {
                // Build the synthetic single-row TableSet directly from the
                // column data — no intermediate row buffer. Old row-major
                // path's `data: row.iter().map(|v| Arc::new(vec![v.clone()]))`
                // was one clone per cell; this matches that.
                let synth_data: Vec<std::sync::Arc<Vec<CfmlValue>>> = data
                    .iter()
                    .map(|col| std::sync::Arc::new(vec![col[i].clone()]))
                    .collect();
                let tbl = QoQTable {
                    name: String::new(),
                    columns: columns.to_vec(),
                    data: synth_data,
                    row_count: 1,
                };
                let ts = TableSet { tables: vec![tbl] };
                let inter = [1usize];
                for (k, ob) in order_by.iter().enumerate() {
                    if col_alias[k].is_some() {
                        continue;
                    }
                    let v = self.order_key(ob, &ts, RowCtx::Row(&inter), &[])?;
                    cols[k].push(v);
                }
            }
        }
        *sort_keys = cols;
        Ok(())
    }

    // ── Expression evaluation (dual-path via RowCtx) ────────────────────

    fn eval(&self, expr: &Expr, tables: &TableSet, ctx: RowCtx) -> CfmlResult {
        match expr {
            Expr::Literal(v) => Ok(v.clone()),

            Expr::Star { .. } => Err(CfmlError::runtime(
                "Query of Queries: '*' is only valid in a SELECT list".to_string(),
            )),

            Expr::Column { table, name } => self.eval_column(table.as_deref(), name, tables, ctx),

            Expr::ResolvedColumn { ti, ci, .. } => Ok(self.eval_resolved_column(*ti, *ci, tables, ctx)),

            Expr::Param(p) => self.eval_param(p),

            Expr::Function { name, args, distinct } => {
                if is_aggregate(name, self.registry) {
                    return self.eval_aggregate_call(name, args, *distinct, tables, ctx);
                }
                let mut evaled = Vec::with_capacity(args.len());
                for a in args {
                    evaled.push(self.eval(a, tables, ctx)?);
                }
                self.call_scalar_fn(name, evaled)
            }

            Expr::Cast { expr, ty } => {
                let v = self.eval(expr, tables, ctx)?;
                functions::cast_value(&v, ty)
            }

            Expr::Unary { op, expr } => {
                let v = self.eval(expr, tables, ctx)?;
                apply_unary(*op, &v)
            }

            Expr::Binary { left, op, right } => self.eval_binary(left, *op, right, tables, ctx),

            Expr::Case {
                operand,
                whens,
                else_expr,
            } => self.eval_case(operand.as_deref(), whens, else_expr.as_deref(), tables, ctx),

            Expr::IsNull { expr, negated } => {
                let v = self.eval(expr, tables, ctx)?;
                let is_null = matches!(v, CfmlValue::Null);
                Ok(CfmlValue::Bool(is_null ^ negated))
            }

            Expr::InList {
                expr,
                negated,
                list,
            } => self.eval_in_list(expr, *negated, list, tables, ctx),

            Expr::InSubquery {
                expr,
                negated,
                select,
            } => self.eval_in_subquery(expr, *negated, select, tables, ctx),

            Expr::Between {
                expr,
                negated,
                low,
                high,
            } => self.eval_between(expr, *negated, low, high, tables, ctx),

            Expr::Like {
                expr,
                negated,
                pattern,
                escape,
                compiled,
            } => self.eval_like(
                expr,
                *negated,
                pattern,
                escape.as_deref(),
                compiled.as_ref(),
                tables,
                ctx,
            ),

            Expr::ScalarSubquery(select) => self.eval_scalar_subquery(select),
        }
    }

    /// Per-row evaluator over the pre-compiled expression form built by
    /// [`compiled::compile`] at bind time. Falls back to [`Self::eval`] for any
    /// subtree shape we haven't specialised (encoded as `CompiledExpr::Generic`).
    fn eval_compiled(
        &self,
        ce: &CompiledExpr,
        tables: &TableSet,
        ctx: RowCtx,
    ) -> CfmlResult {
        match ce {
            CompiledExpr::Null => Ok(CfmlValue::Null),
            CompiledExpr::LitBool(b) => Ok(CfmlValue::Bool(*b)),
            CompiledExpr::LitInt(i) => Ok(CfmlValue::Int(*i)),
            CompiledExpr::LitDouble(d) => Ok(CfmlValue::Double(*d)),
            CompiledExpr::LitString(s) => Ok(CfmlValue::string(s.as_ref())),
            CompiledExpr::Column { ti, ci } => {
                Ok(self.eval_resolved_column(*ti, *ci, tables, ctx))
            }
            CompiledExpr::And(parts) => {
                let mut any_unknown = false;
                for p in parts {
                    let v = self.eval_compiled(p, tables, ctx)?;
                    match tri(&v) {
                        Some(false) => return Ok(CfmlValue::Bool(false)),
                        None => any_unknown = true,
                        Some(true) => {}
                    }
                }
                Ok(if any_unknown {
                    CfmlValue::Null
                } else {
                    CfmlValue::Bool(true)
                })
            }
            CompiledExpr::Or(parts) => {
                let mut any_unknown = false;
                for p in parts {
                    let v = self.eval_compiled(p, tables, ctx)?;
                    match tri(&v) {
                        Some(true) => return Ok(CfmlValue::Bool(true)),
                        None => any_unknown = true,
                        Some(false) => {}
                    }
                }
                Ok(if any_unknown {
                    CfmlValue::Null
                } else {
                    CfmlValue::Bool(false)
                })
            }
            CompiledExpr::Not(inner) => {
                let v = self.eval_compiled(inner, tables, ctx)?;
                Ok(match tri(&v) {
                    Some(true) => CfmlValue::Bool(false),
                    Some(false) => CfmlValue::Bool(true),
                    None => CfmlValue::Null,
                })
            }
            CompiledExpr::IsNull { expr, negated } => {
                let v = self.eval_compiled(expr, tables, ctx)?;
                let is_null = matches!(v, CfmlValue::Null);
                Ok(CfmlValue::Bool(is_null ^ negated))
            }
            CompiledExpr::ColCmpLit { ti, ci, op, rhs } => {
                let v = self.eval_resolved_column(*ti, *ci, tables, ctx);
                Ok(cmp_to_bool_value(&v, *op, rhs))
            }
            CompiledExpr::Cmp { lhs, op, rhs } => {
                let l = self.eval_compiled(lhs, tables, ctx)?;
                let r = self.eval_compiled(rhs, tables, ctx)?;
                Ok(cmp_to_bool_value(&l, *op, &r))
            }
            CompiledExpr::ColInLits {
                ti,
                ci,
                negated,
                lits,
            } => {
                let v = self.eval_resolved_column(*ti, *ci, tables, ctx);
                if matches!(v, CfmlValue::Null) {
                    return Ok(CfmlValue::Null);
                }
                let mut found_null = false;
                for lit in lits {
                    match sql_equal(&v, lit) {
                        Some(true) => return Ok(CfmlValue::Bool(!negated)),
                        Some(false) => {}
                        None => found_null = true,
                    }
                }
                Ok(membership_result(false, found_null, *negated))
            }
            CompiledExpr::LikeConst {
                lhs,
                negated,
                compiled,
            } => {
                let v = self.eval_compiled(lhs, tables, ctx)?;
                if matches!(v, CfmlValue::Null) {
                    return Ok(CfmlValue::Null);
                }
                let s = v.as_string();
                let m = compiled.matches(&s);
                Ok(CfmlValue::Bool(m ^ *negated))
            }
            CompiledExpr::Generic(e) => self.eval(e, tables, ctx),
        }
    }

    /// Fast path for an already-bound column ref: skip the linear
    /// `resolve_column` (case-insensitive table-name + column-name scan) that
    /// `eval_column` performs. `ti`/`ci` are pre-resolved into the local
    /// `TableSet`.
    #[inline]
    fn eval_resolved_column(
        &self,
        ti: u32,
        ci: u32,
        tables: &TableSet,
        ctx: RowCtx,
    ) -> CfmlValue {
        let inter: &[usize] = match ctx {
            RowCtx::Row(i) => i,
            RowCtx::Group(part) => {
                if part.is_empty() {
                    return CfmlValue::Null;
                }
                part.row(0)
            }
        };
        tables.value(inter, ti as usize, ci as usize)
    }

    fn eval_column(
        &self,
        table: Option<&str>,
        name: &str,
        tables: &TableSet,
        ctx: RowCtx,
    ) -> CfmlResult {
        match ctx {
            RowCtx::Row(inter) => self.column_value(table, name, tables, inter),
            RowCtx::Group(part) => {
                if part.is_empty() {
                    Ok(CfmlValue::Null)
                } else {
                    self.column_value(table, name, tables, part.row(0))
                }
            }
        }
    }

    fn column_value(
        &self,
        table: Option<&str>,
        name: &str,
        tables: &TableSet,
        inter: &[usize],
    ) -> CfmlResult {
        match tables.resolve_column(table, name) {
            Some((ti, ci)) => Ok(tables.value(inter, ti, ci)),
            None => Err(CfmlError::runtime(format!(
                "Query of Queries: column '{}{}' not found",
                table.map(|t| format!("{}.", t)).unwrap_or_default(),
                name
            ))),
        }
    }

    fn eval_param(&self, p: &ParamRef) -> CfmlResult {
        match p {
            ParamRef::Positional(i) => self.params.positional.get(*i).cloned().ok_or_else(|| {
                CfmlError::runtime(format!(
                    "Query of Queries: positional parameter #{} has no value",
                    i + 1
                ))
            }),
            ParamRef::Named(n) => self.params.lookup_named(n).cloned().ok_or_else(|| {
                CfmlError::runtime(format!("Query of Queries: named parameter ':{}' has no value", n))
            }),
        }
    }

    fn eval_binary(
        &self,
        left: &Expr,
        op: BinaryOp,
        right: &Expr,
        tables: &TableSet,
        ctx: RowCtx,
    ) -> CfmlResult {
        // Short-circuit logical operators (also keeps 3-valued logic correct).
        match op {
            BinaryOp::And => {
                let l = self.eval(left, tables, ctx)?;
                if tri(&l) == Some(false) {
                    return Ok(CfmlValue::Bool(false));
                }
                let r = self.eval(right, tables, ctx)?;
                return Ok(and3(tri(&l), tri(&r)));
            }
            BinaryOp::Or => {
                let l = self.eval(left, tables, ctx)?;
                if tri(&l) == Some(true) {
                    return Ok(CfmlValue::Bool(true));
                }
                let r = self.eval(right, tables, ctx)?;
                return Ok(or3(tri(&l), tri(&r)));
            }
            _ => {}
        }
        let l = self.eval(left, tables, ctx)?;
        let r = self.eval(right, tables, ctx)?;
        apply_binary(&l, op, &r)
    }

    fn eval_case(
        &self,
        operand: Option<&Expr>,
        whens: &[WhenThen],
        else_expr: Option<&Expr>,
        tables: &TableSet,
        ctx: RowCtx,
    ) -> CfmlResult {
        let operand_val = match operand {
            Some(o) => Some(self.eval(o, tables, ctx)?),
            None => None,
        };
        for wt in whens {
            let matched = match &operand_val {
                // Simple CASE: operand = when-value.
                Some(ov) => {
                    let wv = self.eval(&wt.when, tables, ctx)?;
                    sql_equal(ov, &wv) == Some(true)
                }
                // Searched CASE: when is a boolean predicate.
                None => is_truthy(&self.eval(&wt.when, tables, ctx)?),
            };
            if matched {
                return self.eval(&wt.then, tables, ctx);
            }
        }
        match else_expr {
            Some(e) => self.eval(e, tables, ctx),
            None => Ok(CfmlValue::Null),
        }
    }

    fn eval_in_list(
        &self,
        expr: &Expr,
        negated: bool,
        list: &[Expr],
        tables: &TableSet,
        ctx: RowCtx,
    ) -> CfmlResult {
        let target = self.eval(expr, tables, ctx)?;
        if matches!(target, CfmlValue::Null) {
            return Ok(CfmlValue::Null);
        }
        let mut found_null = false;
        for item in list {
            let v = self.eval(item, tables, ctx)?;
            // A `list=true` cfqueryparam binds to an array; each element is a
            // candidate value, so `IN (:ids)` matches any of them (Lucee/ACF
            // IN-list expansion). A scalar element is tested directly.
            if let CfmlValue::Array(arr) = &v {
                for elem in arr.iter() {
                    match sql_equal(&target, &elem) {
                        Some(true) => return Ok(CfmlValue::Bool(!negated)),
                        Some(false) => {}
                        None => found_null = true,
                    }
                }
                continue;
            }
            match sql_equal(&target, &v) {
                Some(true) => return Ok(CfmlValue::Bool(!negated)),
                Some(false) => {}
                None => found_null = true,
            }
        }
        Ok(membership_result(false, found_null, negated))
    }

    fn eval_in_subquery(
        &self,
        expr: &Expr,
        negated: bool,
        select: &SelectStatement,
        tables: &TableSet,
        ctx: RowCtx,
    ) -> CfmlResult {
        let target = self.eval(expr, tables, ctx)?;
        if matches!(target, CfmlValue::Null) {
            return Ok(CfmlValue::Null);
        }
        let values = self.subquery_first_column(select)?;
        let mut found_null = false;
        for v in &values {
            match sql_equal(&target, v) {
                Some(true) => return Ok(CfmlValue::Bool(!negated)),
                Some(false) => {}
                None => found_null = true,
            }
        }
        Ok(membership_result(false, found_null, negated))
    }

    fn eval_between(
        &self,
        expr: &Expr,
        negated: bool,
        low: &Expr,
        high: &Expr,
        tables: &TableSet,
        ctx: RowCtx,
    ) -> CfmlResult {
        let v = self.eval(expr, tables, ctx)?;
        let lo = self.eval(low, tables, ctx)?;
        let hi = self.eval(high, tables, ctx)?;
        let (Some(c_lo), Some(c_hi)) = (
            crate::compare::compare_sql(&v, &lo),
            crate::compare::compare_sql(&v, &hi),
        ) else {
            return Ok(CfmlValue::Null);
        };
        let in_range = c_lo != Ordering::Less && c_hi != Ordering::Greater;
        Ok(CfmlValue::Bool(in_range ^ negated))
    }

    fn eval_like(
        &self,
        expr: &Expr,
        negated: bool,
        pattern: &Expr,
        escape: Option<&Expr>,
        compiled: Option<&like::Compiled>,
        tables: &TableSet,
        ctx: RowCtx,
    ) -> CfmlResult {
        let v = self.eval(expr, tables, ctx)?;
        if matches!(v, CfmlValue::Null) {
            return Ok(CfmlValue::Null);
        }
        let text = v.as_string();
        // Constant pattern: use the once-per-query pre-compiled matcher embedded
        // on the AST node by `bind_expr`, avoiding a recompile on every row.
        if let Some(c) = compiled {
            return Ok(CfmlValue::Bool(c.matches(&text) ^ negated));
        }
        // Dynamic pattern (or a NULL literal, which isn't cached): evaluate it.
        let p = self.eval(pattern, tables, ctx)?;
        if matches!(p, CfmlValue::Null) {
            return Ok(CfmlValue::Null);
        }
        let esc = match escape {
            Some(e) => self.eval(e, tables, ctx)?.as_string().chars().next(),
            None => None,
        };
        let matched = like_match(&text, &p.as_string(), esc);
        Ok(CfmlValue::Bool(matched ^ negated))
    }

    fn eval_scalar_subquery(&self, select: &SelectStatement) -> CfmlResult {
        let q = self.run_statement(select)?;
        Ok(q
            .data
            .first()
            .and_then(|c| c.first().cloned())
            .unwrap_or(CfmlValue::Null))
    }

    fn subquery_first_column(
        &self,
        select: &SelectStatement,
    ) -> Result<Vec<CfmlValue>, CfmlError> {
        let q = self.run_statement(select)?;
        Ok(q.data.first().map(|a| (**a).clone()).unwrap_or_default())
    }

    // ── Function dispatch ───────────────────────────────────────────────

    fn call_scalar_fn(&self, name: &str, args: Vec<CfmlValue>) -> CfmlResult {
        // 1. built-in scalar
        if let Some(r) = functions::call_scalar(name, &args) {
            return r;
        }
        // 2. native scalar registered from Rust
        if let Some((QoQFnKind::Scalar, f)) = self.registry.get_native(name) {
            return f(args);
        }
        // 3. a native extension's scalar
        if let Some(f) = self.registry.get_dynamic(name, QoQFnKind::Scalar) {
            return f(args);
        }
        // 4. custom CFML UDF (scalar)
        if let Some((cfval, QoQFnKind::Scalar)) = self.registry.get_custom(name).cloned() {
            return self.inv.invoke_custom(&cfval, args);
        }
        Err(CfmlError::runtime(format!(
            "Query of Queries: unknown function '{}'",
            name
        )))
    }

    fn eval_aggregate_call(
        &self,
        name: &str,
        args: &[Expr],
        distinct: bool,
        tables: &TableSet,
        ctx: RowCtx,
    ) -> CfmlResult {
        // Aggregates need a partition. In a Row context (no grouping) treat the
        // single row as a one-element partition.
        let single;
        let partition: &Intersections = match ctx {
            RowCtx::Group(p) => p,
            RowCtx::Row(r) => {
                single = Intersections {
                    width: r.len(),
                    flat: r.to_vec(),
                };
                &single
            }
        };

        let lname = name.to_lowercase();

        // COUNT is special: COUNT(*) counts rows; COUNT(x) counts non-null.
        if lname == "count" {
            let is_star = args.is_empty() || matches!(args.first(), Some(Expr::Star { .. }));
            if is_star {
                return Ok(CfmlValue::Int(partition.len() as i64));
            }
            let mut vals = self.collect_arg(&args[0], tables, partition)?;
            vals.retain(|v| !matches!(v, CfmlValue::Null));
            if distinct {
                dedup_values(&mut vals);
            }
            return Ok(CfmlValue::Int(vals.len() as i64));
        }

        if is_builtin_aggregate(&lname) {
            let arg0 = args.first().ok_or_else(|| {
                CfmlError::runtime(format!("Query of Queries: {} requires an argument", lname))
            })?;
            let mut col = self.collect_arg(arg0, tables, partition)?;
            if distinct {
                let mut nonnull: Vec<CfmlValue> =
                    col.into_iter().filter(|v| !matches!(v, CfmlValue::Null)).collect();
                dedup_values(&mut nonnull);
                col = nonnull;
            }
            if lname == "group_concat" || lname == "string_agg" {
                let sep = if partition.is_empty() {
                    match args.get(1) {
                        Some(_) => ",".to_string(),
                        None => ",".to_string(),
                    }
                } else {
                    match args.get(1) {
                        Some(e) => self
                            .eval(e, tables, RowCtx::Row(partition.row(0)))?
                            .as_string(),
                        None => ",".to_string(),
                    }
                };
                let parts: Vec<String> = col
                    .iter()
                    .filter(|v| !matches!(v, CfmlValue::Null))
                    .map(|v| v.as_string())
                    .collect();
                if parts.is_empty() {
                    return Ok(CfmlValue::Null);
                }
                return Ok(CfmlValue::string(parts.join(&sep)));
            }
            return Ok(aggregate_numeric(&lname, &col));
        }

        // Native / custom aggregate: pass each arg as a column array.
        let mut arrays = Vec::with_capacity(args.len());
        for a in args {
            let col = self.collect_arg(a, tables, partition)?;
            arrays.push(CfmlValue::array(col));
        }
        if let Some((QoQFnKind::Aggregate, f)) = self.registry.get_native(&lname) {
            return f(arrays);
        }
        if let Some(f) = self.registry.get_dynamic(&lname, QoQFnKind::Aggregate) {
            return f(arrays);
        }
        if let Some((cfval, QoQFnKind::Aggregate)) = self.registry.get_custom(&lname).cloned() {
            return self.inv.invoke_custom(&cfval, arrays);
        }
        Err(CfmlError::runtime(format!(
            "Query of Queries: unknown aggregate function '{}'",
            name
        )))
    }

    fn collect_arg(
        &self,
        arg: &Expr,
        tables: &TableSet,
        partition: &Intersections,
    ) -> Result<Vec<CfmlValue>, CfmlError> {
        let mut out = Vec::with_capacity(partition.len());
        for inter in partition.iter() {
            out.push(self.eval(arg, tables, RowCtx::Row(inter))?);
        }
        Ok(out)
    }
}

// ── Aggregate helpers ───────────────────────────────────────────────────

fn is_builtin_aggregate(name: &str) -> bool {
    matches!(
        name.to_lowercase().as_str(),
        "count" | "sum" | "avg" | "min" | "max" | "group_concat" | "string_agg"
    )
}

fn is_aggregate(name: &str, registry: &QoQFunctionRegistry) -> bool {
    is_builtin_aggregate(name) || registry.is_aggregate(name)
}

fn expr_has_aggregate(expr: &Expr, registry: &QoQFunctionRegistry) -> bool {
    match expr {
        Expr::Function { name, args, .. } => {
            is_aggregate(name, registry) || args.iter().any(|a| expr_has_aggregate(a, registry))
        }
        Expr::Binary { left, right, .. } => {
            expr_has_aggregate(left, registry) || expr_has_aggregate(right, registry)
        }
        Expr::Unary { expr, .. } | Expr::Cast { expr, .. } | Expr::IsNull { expr, .. } => {
            expr_has_aggregate(expr, registry)
        }
        Expr::Case {
            operand,
            whens,
            else_expr,
        } => {
            operand.as_ref().map(|e| expr_has_aggregate(e, registry)).unwrap_or(false)
                || whens.iter().any(|w| {
                    expr_has_aggregate(&w.when, registry) || expr_has_aggregate(&w.then, registry)
                })
                || else_expr.as_ref().map(|e| expr_has_aggregate(e, registry)).unwrap_or(false)
        }
        Expr::InList { expr, list, .. } => {
            expr_has_aggregate(expr, registry) || list.iter().any(|e| expr_has_aggregate(e, registry))
        }
        Expr::Between {
            expr, low, high, ..
        } => {
            expr_has_aggregate(expr, registry)
                || expr_has_aggregate(low, registry)
                || expr_has_aggregate(high, registry)
        }
        Expr::Like { expr, pattern, .. } => {
            expr_has_aggregate(expr, registry) || expr_has_aggregate(pattern, registry)
        }
        // Subqueries are their own scope; their aggregates don't make the outer
        // query aggregate.
        _ => false,
    }
}

/// SUM / AVG / MIN / MAX over a column of values (NULLs ignored).
fn aggregate_numeric(name: &str, col: &[CfmlValue]) -> CfmlValue {
    match name {
        "sum" => {
            let mut sum = 0.0f64;
            let mut all_int = true;
            let mut any = false;
            for v in col {
                if matches!(v, CfmlValue::Null) {
                    continue;
                }
                if let Some(n) = functions::to_f64(v) {
                    any = true;
                    if !matches!(v, CfmlValue::Int(_)) {
                        all_int = false;
                    }
                    sum += n;
                }
            }
            if !any {
                CfmlValue::Null
            } else if all_int && sum.fract() == 0.0 && sum.abs() < 9.0e15 {
                CfmlValue::Int(sum as i64)
            } else {
                CfmlValue::Double(sum)
            }
        }
        "avg" => {
            let mut sum = 0.0f64;
            let mut count = 0i64;
            for v in col {
                if let Some(n) = functions::to_f64(v) {
                    if !matches!(v, CfmlValue::Null) {
                        sum += n;
                        count += 1;
                    }
                }
            }
            if count == 0 {
                CfmlValue::Null
            } else {
                CfmlValue::Double(sum / count as f64)
            }
        }
        "min" | "max" => {
            let want_min = name == "min";
            let mut best: Option<&CfmlValue> = None;
            for v in col {
                if matches!(v, CfmlValue::Null) {
                    continue;
                }
                best = Some(match best {
                    None => v,
                    Some(cur) => {
                        let ord = compare_total(v, cur);
                        if (want_min && ord == Ordering::Less)
                            || (!want_min && ord == Ordering::Greater)
                        {
                            v
                        } else {
                            cur
                        }
                    }
                });
            }
            best.cloned().unwrap_or(CfmlValue::Null)
        }
        _ => CfmlValue::Null,
    }
}

// ── Boolean / operator helpers ──────────────────────────────────────────

/// Three-valued truth of a value: `None` = unknown (NULL).
/// Compiled-expression comparison helper: applies a [`CmpOp`] to two values
/// via [`compare_sql`] and wraps the 3-valued result back into a CfmlValue.
fn cmp_to_bool_value(l: &CfmlValue, op: CmpOp, r: &CfmlValue) -> CfmlValue {
    match compare_sql(l, r) {
        None => CfmlValue::Null,
        Some(ord) => CfmlValue::Bool(match op {
            CmpOp::Eq => ord == Ordering::Equal,
            CmpOp::Neq => ord != Ordering::Equal,
            CmpOp::Lt => ord == Ordering::Less,
            CmpOp::Lte => ord != Ordering::Greater,
            CmpOp::Gt => ord == Ordering::Greater,
            CmpOp::Gte => ord != Ordering::Less,
        }),
    }
}

fn tri(v: &CfmlValue) -> Option<bool> {
    match v {
        CfmlValue::Null => None,
        CfmlValue::Bool(b) => Some(*b),
        other => Some(other.is_true()),
    }
}

/// `true` only when definitely true (NULL / unknown → false). For WHERE, ON,
/// HAVING and CASE conditions.
fn is_truthy(v: &CfmlValue) -> bool {
    tri(v) == Some(true)
}

fn and3(l: Option<bool>, r: Option<bool>) -> CfmlValue {
    match (l, r) {
        (Some(false), _) | (_, Some(false)) => CfmlValue::Bool(false),
        (Some(true), Some(true)) => CfmlValue::Bool(true),
        _ => CfmlValue::Null,
    }
}

fn or3(l: Option<bool>, r: Option<bool>) -> CfmlValue {
    match (l, r) {
        (Some(true), _) | (_, Some(true)) => CfmlValue::Bool(true),
        (Some(false), Some(false)) => CfmlValue::Bool(false),
        _ => CfmlValue::Null,
    }
}

fn membership_result(found_true: bool, found_null: bool, negated: bool) -> CfmlValue {
    let base = if found_true {
        Some(true)
    } else if found_null {
        None
    } else {
        Some(false)
    };
    match base {
        Some(b) => CfmlValue::Bool(b ^ negated),
        None => CfmlValue::Null,
    }
}

fn apply_unary(op: UnaryOp, v: &CfmlValue) -> CfmlResult {
    match op {
        UnaryOp::Not => Ok(match tri(v) {
            Some(b) => CfmlValue::Bool(!b),
            None => CfmlValue::Null,
        }),
        UnaryOp::Neg => {
            if matches!(v, CfmlValue::Null) {
                return Ok(CfmlValue::Null);
            }
            match v {
                CfmlValue::Int(i) => Ok(CfmlValue::Int(-*i)),
                _ => match functions::to_f64(v) {
                    Some(n) => Ok(CfmlValue::Double(-n)),
                    None => Err(CfmlError::runtime(
                        "Query of Queries: cannot negate a non-numeric value".to_string(),
                    )),
                },
            }
        }
        UnaryOp::Plus => Ok(v.clone()),
    }
}

fn apply_binary(l: &CfmlValue, op: BinaryOp, r: &CfmlValue) -> CfmlResult {
    use crate::compare::compare_sql;
    match op {
        BinaryOp::Eq | BinaryOp::Neq | BinaryOp::Lt | BinaryOp::Lte | BinaryOp::Gt | BinaryOp::Gte => {
            match compare_sql(l, r) {
                None => Ok(CfmlValue::Null),
                Some(ord) => {
                    let res = match op {
                        BinaryOp::Eq => ord == Ordering::Equal,
                        BinaryOp::Neq => ord != Ordering::Equal,
                        BinaryOp::Lt => ord == Ordering::Less,
                        BinaryOp::Lte => ord != Ordering::Greater,
                        BinaryOp::Gt => ord == Ordering::Greater,
                        BinaryOp::Gte => ord != Ordering::Less,
                        _ => unreachable!(),
                    };
                    Ok(CfmlValue::Bool(res))
                }
            }
        }
        BinaryOp::And => Ok(and3(tri(l), tri(r))),
        BinaryOp::Or => Ok(or3(tri(l), tri(r))),
        BinaryOp::Concat => {
            if matches!(l, CfmlValue::Null) || matches!(r, CfmlValue::Null) {
                Ok(CfmlValue::Null)
            } else {
                Ok(CfmlValue::string(l.as_string() + &r.as_string()))
            }
        }
        BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div | BinaryOp::Mod => {
            arith(l, op, r)
        }
        BinaryOp::BitAnd | BinaryOp::BitOr | BinaryOp::BitXor => {
            if matches!(l, CfmlValue::Null) || matches!(r, CfmlValue::Null) {
                return Ok(CfmlValue::Null);
            }
            let (a, b) = (
                functions::to_i64(l).unwrap_or(0),
                functions::to_i64(r).unwrap_or(0),
            );
            Ok(CfmlValue::Int(match op {
                BinaryOp::BitAnd => a & b,
                BinaryOp::BitOr => a | b,
                BinaryOp::BitXor => a ^ b,
                _ => unreachable!(),
            }))
        }
    }
}

fn arith(l: &CfmlValue, op: BinaryOp, r: &CfmlValue) -> CfmlResult {
    if matches!(l, CfmlValue::Null) || matches!(r, CfmlValue::Null) {
        return Ok(CfmlValue::Null);
    }
    // `+` is numeric when BOTH operands coerce to numbers (Lucee QoQ: '10'+'5'
    // = 15), otherwise string concatenation (name+dept = "JohnIT", name+5 =
    // "John5"). Matches SQL-Server-style `+` overloading.
    if op == BinaryOp::Add {
        match (functions::to_f64(l), functions::to_f64(r)) {
            (Some(a), Some(b)) => {
                if let (CfmlValue::Int(x), CfmlValue::Int(y)) = (l, r) {
                    if let Some(v) = x.checked_add(*y) {
                        return Ok(CfmlValue::Int(v));
                    }
                }
                return Ok(integral_or_double(a + b));
            }
            _ => return Ok(CfmlValue::string(l.as_string() + &r.as_string())),
        }
    }
    // Sub / Mul / Mod — integer-preserving when both sides are integers.
    if let (CfmlValue::Int(a), CfmlValue::Int(b)) = (l, r) {
        match op {
            BinaryOp::Sub => {
                if let Some(v) = a.checked_sub(*b) {
                    return Ok(CfmlValue::Int(v));
                }
            }
            BinaryOp::Mul => {
                if let Some(v) = a.checked_mul(*b) {
                    return Ok(CfmlValue::Int(v));
                }
            }
            BinaryOp::Mod => {
                if *b == 0 {
                    return Err(CfmlError::runtime("Query of Queries: modulo by zero".to_string()));
                }
                return Ok(CfmlValue::Int(a % b));
            }
            _ => {}
        }
    }
    let a = functions::to_f64(l).ok_or_else(|| non_numeric(l))?;
    let b = functions::to_f64(r).ok_or_else(|| non_numeric(r))?;
    let res = match op {
        BinaryOp::Sub => a - b,
        BinaryOp::Mul => a * b,
        BinaryOp::Div => {
            if b == 0.0 {
                return Err(CfmlError::runtime("Query of Queries: division by zero".to_string()));
            }
            a / b
        }
        BinaryOp::Mod => {
            if b == 0.0 {
                return Err(CfmlError::runtime("Query of Queries: modulo by zero".to_string()));
            }
            a % b
        }
        _ => unreachable!(),
    };
    // Render an integral result as Int (preserve native types); division stays Double.
    if op != BinaryOp::Div {
        Ok(integral_or_double(res))
    } else {
        Ok(CfmlValue::Double(res))
    }
}

/// An integral f64 becomes `Int` (preserve native types), else `Double`.
fn integral_or_double(n: f64) -> CfmlValue {
    if n.fract() == 0.0 && n.abs() < 9.0e15 {
        CfmlValue::Int(n as i64)
    } else {
        CfmlValue::Double(n)
    }
}

fn non_numeric(v: &CfmlValue) -> CfmlError {
    CfmlError::runtime(format!(
        "Query of Queries: '{}' is not numeric in an arithmetic expression",
        v.as_string()
    ))
}

// ── Hash-join helpers (H4 / H5) ─────────────────────────────────────────

/// Recognise `ResolvedCol = ResolvedCol` and return both sides' `(ti, ci)`.
fn equi_pair(e: &Expr) -> Option<(usize, usize, usize, usize)> {
    if let Expr::Binary { left, op: BinaryOp::Eq, right } = e {
        if let (
            Expr::ResolvedColumn { ti: lt, ci: lc, .. },
            Expr::ResolvedColumn { ti: rt, ci: rc, .. },
        ) = (&**left, &**right)
        {
            if lt != rt {
                return Some((*lt as usize, *lc as usize, *rt as usize, *rc as usize));
            }
        }
    }
    None
}

/// Walk an optional WHERE expression as an AND-chain and collect every
/// `ResolvedCol = ResolvedCol` conjunct linking two distinct tables.
/// Non-AND/non-equi nodes are ignored (per-row filters stay where they are).
fn collect_equi_conjuncts(expr: Option<&Expr>) -> Vec<(usize, usize, usize, usize)> {
    fn walk(e: &Expr, out: &mut Vec<(usize, usize, usize, usize)>) {
        if let Expr::Binary { op: BinaryOp::And, left, right, .. } = e {
            walk(left, out);
            walk(right, out);
            return;
        }
        if let Some(p) = equi_pair(e) {
            out.push(p);
        }
    }
    let mut out = Vec::new();
    if let Some(e) = expr {
        walk(e, &mut out);
    }
    out
}

// ── Column-ref binding (H1) ──────────────────────────────────────────────

/// Rewrite every `Expr::Column` in this core's outer scope into
/// `Expr::ResolvedColumn { ti, ci }` against the freshly-built `TableSet`,
/// so per-row evaluation can skip the linear `resolve_column` lookup.
///
/// Scope: WHERE, GROUP BY, HAVING, JOIN ON, the (expanded) SELECT list, and
/// any pushed-down ORDER BY. Subqueries (`ScalarSubquery`/`InSubquery`) and
/// derived tables in FROM are *not* descended — they have their own table
/// scope and are bound when their own `run_core` runs.
///
/// Unresolvable column refs are left as `Expr::Column` and will produce
/// the usual "column not found" error at eval time (same behaviour as before).
fn bind_core(
    core: &mut SelectCore,
    expanded_cols: &mut [SelectColumn],
    order_by: &mut [OrderByExpr],
    tables: &TableSet,
) {
    if let Some(w) = core.where_clause.as_mut() {
        bind_expr(w, tables);
    }
    for g in core.group_by.iter_mut() {
        bind_expr(g, tables);
    }
    if let Some(h) = core.having.as_mut() {
        bind_expr(h, tables);
    }
    for j in core.joins.iter_mut() {
        if let Some(on) = j.on.as_mut() {
            bind_expr(on, tables);
        }
    }
    for sc in expanded_cols.iter_mut() {
        bind_expr(&mut sc.expr, tables);
    }
    for ob in order_by.iter_mut() {
        bind_expr(&mut ob.expr, tables);
    }
}

fn bind_expr(expr: &mut Expr, tables: &TableSet) {
    match expr {
        Expr::Column { table, name } => {
            if let Some((ti, ci)) = tables.resolve_column(table.as_deref(), name) {
                let resolved = Expr::ResolvedColumn {
                    ti: ti as u32,
                    ci: ci as u32,
                    name: std::mem::take(name),
                };
                *expr = resolved;
            }
        }
        // Already bound — no-op (idempotent for safety).
        Expr::ResolvedColumn { .. } => {}
        Expr::Binary { left, right, .. } => {
            bind_expr(left, tables);
            bind_expr(right, tables);
        }
        Expr::Unary { expr: inner, .. }
        | Expr::Cast { expr: inner, .. }
        | Expr::IsNull { expr: inner, .. } => bind_expr(inner, tables),
        Expr::Function { args, .. } => {
            for a in args {
                bind_expr(a, tables);
            }
        }
        Expr::Case {
            operand,
            whens,
            else_expr,
        } => {
            if let Some(o) = operand {
                bind_expr(o, tables);
            }
            for w in whens.iter_mut() {
                bind_expr(&mut w.when, tables);
                bind_expr(&mut w.then, tables);
            }
            if let Some(e) = else_expr {
                bind_expr(e, tables);
            }
        }
        Expr::InList { expr: lhs, list, .. } => {
            bind_expr(lhs, tables);
            for e in list {
                bind_expr(e, tables);
            }
        }
        Expr::Between {
            expr: lhs,
            low,
            high,
            ..
        } => {
            bind_expr(lhs, tables);
            bind_expr(low, tables);
            bind_expr(high, tables);
        }
        Expr::Like {
            expr: lhs,
            pattern,
            escape,
            compiled,
            ..
        } => {
            bind_expr(lhs, tables);
            bind_expr(pattern, tables);
            if let Some(e) = escape {
                bind_expr(e, tables);
            }
            // Pre-compile constant literal patterns (and constant escape, if
            // present) — once per query, used on every row.
            if compiled.is_none() {
                if let Expr::Literal(pv) = &**pattern {
                    if !matches!(pv, CfmlValue::Null) {
                        let esc = match escape.as_deref() {
                            None => Some(None),
                            Some(Expr::Literal(ev)) => Some(ev.as_string().chars().next()),
                            Some(_) => None, // non-literal escape → don't pre-compile
                        };
                        if let Some(esc) = esc {
                            *compiled = Some(like::compile(&pv.as_string(), esc));
                        }
                    }
                }
            }
        }
        // Subqueries get their own scope; don't descend.
        Expr::InSubquery { expr: lhs, .. } => bind_expr(lhs, tables),
        Expr::ScalarSubquery(_) => {}
        Expr::Star { .. } | Expr::Literal(_) | Expr::Param(_) => {}
    }
}

// ── Projection / column-name helpers ────────────────────────────────────

/// Expand `*` and `table.*` into explicit column references.
fn expand_columns(
    columns: &[SelectColumn],
    tables: &TableSet,
) -> Result<Vec<SelectColumn>, CfmlError> {
    let mut out = Vec::new();
    for sc in columns {
        match &sc.expr {
            Expr::Star { table: None } => {
                for t in &tables.tables {
                    for col in &t.columns {
                        out.push(SelectColumn {
                            expr: Expr::Column {
                                table: Some(t.name.clone()),
                                name: col.clone(),
                            },
                            alias: Some(col.clone()),
                        });
                    }
                }
            }
            Expr::Star { table: Some(tn) } => {
                let t = tables
                    .tables
                    .iter()
                    .find(|t| t.name.eq_ignore_ascii_case(tn))
                    .ok_or_else(|| {
                        CfmlError::runtime(format!("Query of Queries: unknown table '{}' in {}.*", tn, tn))
                    })?;
                for col in &t.columns {
                    out.push(SelectColumn {
                        expr: Expr::Column {
                            table: Some(t.name.clone()),
                            name: col.clone(),
                        },
                        alias: Some(col.clone()),
                    });
                }
            }
            _ => out.push(sc.clone()),
        }
    }
    Ok(out)
}

/// Output column names: explicit alias, else the column name, else a generated
/// `column_N`. Names are made unique (case-insensitively) by suffixing.
fn derive_column_names(columns: &[SelectColumn]) -> Vec<String> {
    let mut names = Vec::with_capacity(columns.len());
    let mut seen: HashSet<String> = HashSet::new();
    for (i, sc) in columns.iter().enumerate() {
        let base = match (&sc.alias, &sc.expr) {
            (Some(a), _) => a.clone(),
            (None, Expr::Column { name, .. }) => name.clone(),
            (None, Expr::ResolvedColumn { name, .. }) => name.clone(),
            (None, Expr::Function { name, .. }) => name.clone(),
            _ => format!("column_{}", i),
        };
        let mut name = base.clone();
        let mut n = 1;
        while !seen.insert(name.to_lowercase()) {
            n += 1;
            name = format!("{}_{}", base, n);
        }
        names.push(name);
    }
    names
}

/// Final column-major output → CFML query data. With CoreResult now
/// column-major end-to-end, this is a thin Arc-wrap; the row-major transpose
/// machinery is gone (see git history if you need to resurrect it).
fn build_query(
    columns: Vec<String>,
    data: Vec<Vec<CfmlValue>>,
    _row_count: usize,
) -> CfmlQueryData {
    let data = data.into_iter().map(std::sync::Arc::new).collect();
    CfmlQueryData {
        columns,
        data,
        sql: None,
        execution_time: None,
        current_row: 1,
    }
}

/// Read one passthrough cell — the inner loop of every passthrough column
/// build. 0 is the NULL sentinel for outer-join misses.
#[inline]
fn passthrough_cell(src: &[CfmlValue], r: usize) -> CfmlValue {
    if r == 0 {
        CfmlValue::Null
    } else {
        src.get(r - 1).cloned().unwrap_or(CfmlValue::Null)
    }
}

/// Row-major → column-major transpose used by `exec_simple` /
/// `exec_aggregate` to lift their internal row-major output into the
/// `CoreResult` column-major shape. Parallel above the threshold using
/// disjoint raw-pointer reads; see commit history for the safety
/// argument (this is the v0.110 build_query_parallel logic moved here).
fn rows_to_cols(rows: Vec<Vec<CfmlValue>>, col_count: usize) -> Vec<Vec<CfmlValue>> {
    let row_count = rows.len();
    if col_count == 0 || row_count == 0 {
        return (0..col_count).map(|_| Vec::new()).collect();
    }

    #[cfg(not(target_arch = "wasm32"))]
    if row_count >= PARALLEL_BUILD_THRESHOLD && col_count >= 2 {
        return rows_to_cols_parallel(rows, col_count, row_count);
    }

    let mut data: Vec<Vec<CfmlValue>> =
        (0..col_count).map(|_| Vec::with_capacity(row_count)).collect();
    for row in rows {
        let mut it = row.into_iter();
        for ci in 0..col_count {
            data[ci].push(it.next().unwrap_or(CfmlValue::Null));
        }
    }
    data
}

#[cfg(not(target_arch = "wasm32"))]
const PARALLEL_BUILD_THRESHOLD: usize = 5_000;

#[cfg(not(target_arch = "wasm32"))]
fn rows_to_cols_parallel(
    rows: Vec<Vec<CfmlValue>>,
    col_count: usize,
    row_count: usize,
) -> Vec<Vec<CfmlValue>> {
    use rayon::prelude::*;

    // Flatten on main thread — empty inner Vecs drop here, not on rayon
    // workers (the v0.108 20× regression).
    let total = row_count * col_count;
    let mut flat: Vec<CfmlValue> = Vec::with_capacity(total);
    for row in rows {
        let len_before = flat.len();
        flat.extend(row);
        let added = flat.len() - len_before;
        if added < col_count {
            for _ in added..col_count {
                flat.push(CfmlValue::Null);
            }
        } else if added > col_count {
            flat.truncate(len_before + col_count);
        }
    }
    debug_assert_eq!(flat.len(), total);

    let cap = flat.capacity();
    let ptr = flat.as_mut_ptr();
    std::mem::forget(flat);
    let ptr_addr = ptr as usize;

    let cols: Vec<Vec<CfmlValue>> = (0..col_count)
        .into_par_iter()
        .map(|ci| {
            let base = ptr_addr as *mut CfmlValue;
            let mut col = Vec::with_capacity(row_count);
            for r in 0..row_count {
                // SAFETY: every (r, ci) is read by exactly one worker (ci),
                // so the ptr::read calls are pairwise non-aliasing. Each
                // cell was initialised by the flatten step above.
                let v = unsafe { std::ptr::read(base.add(r * col_count + ci)) };
                col.push(v);
            }
            col
        })
        .collect();

    // SAFETY: every cell has been moved out; reclaim the backing buffer
    // as an empty Vec on the main thread (same thread as the original
    // alloc).
    unsafe {
        drop(Vec::from_raw_parts(ptr, 0, cap));
    }

    cols
}

// ── DISTINCT / ORDER BY / LIMIT (column-major) ──────────────────────────

fn dedup_values(values: &mut Vec<CfmlValue>) {
    let mut seen = HashSet::new();
    values.retain(|v| seen.insert(group_key(std::slice::from_ref(v))));
}

/// Compute the per-row keep mask for a col-major dataset. Builds the
/// type-tagged group key one cell at a time directly off the column data —
/// no per-row `Vec<CfmlValue>` materialisation, no per-cell `Arc::clone`.
fn dedup_keep_mask(data: &[Vec<CfmlValue>], row_count: usize) -> Vec<bool> {
    // Above this many rows, build per-row group keys in parallel (pure column
    // reads, no shared state) then dedup sequentially against an FxHashSet.
    // Key building is the dominant cost on wide dedups (UNION DISTINCT over
    // ~1M rows): on Q10 it's ~60% of total wall time before this fan-out.
    #[cfg(not(target_arch = "wasm32"))]
    if row_count >= 4_096 {
        use rayon::prelude::*;
        use rustc_hash::FxHashSet;
        let keys: Vec<String> = (0..row_count)
            .into_par_iter()
            .map(|i| {
                let mut s = String::new();
                for col in data {
                    append_group_key(&mut s, &col[i]);
                }
                s
            })
            .collect();
        let mut seen: FxHashSet<String> = FxHashSet::default();
        seen.reserve(row_count);
        let mut keep = Vec::with_capacity(row_count);
        for k in keys {
            keep.push(seen.insert(k));
        }
        return keep;
    }
    let mut seen = HashSet::new();
    let mut keep = Vec::with_capacity(row_count);
    let mut key_buf = String::new();
    for i in 0..row_count {
        key_buf.clear();
        for col in data {
            append_group_key(&mut key_buf, &col[i]);
        }
        keep.push(seen.insert(std::mem::take(&mut key_buf)));
    }
    keep
}

fn apply_keep_mask(data: &mut [Vec<CfmlValue>], keep: &[bool]) -> usize {
    let mut kept = 0;
    for col in data.iter_mut() {
        let mut i = 0;
        col.retain(|_| {
            let k = keep[i];
            i += 1;
            k
        });
    }
    for &k in keep {
        if k {
            kept += 1;
        }
    }
    kept
}

fn dedup_cols(data: &mut [Vec<CfmlValue>], row_count: usize) -> usize {
    let keep = dedup_keep_mask(data, row_count);
    apply_keep_mask(data, &keep)
}

fn dedup_cols_and_keys(
    data: &mut [Vec<CfmlValue>],
    keys: &mut [Vec<CfmlValue>],
    row_count: usize,
) -> usize {
    // Dedup is keyed on output row data, not sort keys.
    let keep = dedup_keep_mask(data, row_count);
    apply_keep_mask(data, &keep);
    apply_keep_mask(keys, &keep)
}

fn truncate_cols(data: &mut [Vec<CfmlValue>], n: usize) {
    for col in data.iter_mut() {
        col.truncate(n);
    }
}

/// Above this many rows, ORDER BY uses a parallel (stable) sort on non-wasm
/// targets. The comparator is pure (no VM callback), so it parallelises safely;
/// WHERE/projection stay sequential because they may invoke a CFML UDF through
/// the VM's non-thread-safe callback.
#[cfg(not(target_arch = "wasm32"))]
const PARALLEL_SORT_THRESHOLD: usize = 2048;

/// Column-major ORDER BY: build a permutation by sorting row-index slots
/// using the sort-key columns, then apply that permutation to every output
/// column (and discard the sort keys — they aren't needed downstream).
fn sort_cols(
    data: &mut [Vec<CfmlValue>],
    keys: &[Vec<CfmlValue>],
    order_by: &[OrderByExpr],
    row_count: usize,
) {
    if order_by.is_empty() || row_count < 2 {
        return;
    }
    let null = CfmlValue::Null;
    let dirs: Vec<SortDirection> = order_by.iter().map(|o| o.direction).collect();
    let cmp = |&a: &usize, &b: &usize| -> Ordering {
        for (c, dir) in dirs.iter().enumerate() {
            let ka = keys
                .get(c)
                .and_then(|col| col.get(a))
                .unwrap_or(&null);
            let kb = keys
                .get(c)
                .and_then(|col| col.get(b))
                .unwrap_or(&null);
            let mut ord = compare_total(ka, kb);
            if *dir == SortDirection::Desc {
                ord = ord.reverse();
            }
            if ord != Ordering::Equal {
                return ord;
            }
        }
        Ordering::Equal
    };

    let mut idx: Vec<usize> = (0..row_count).collect();
    #[cfg(not(target_arch = "wasm32"))]
    {
        if idx.len() >= PARALLEL_SORT_THRESHOLD {
            use rayon::slice::ParallelSliceMut;
            idx.par_sort_by(cmp); // stable parallel sort
        } else {
            idx.sort_by(cmp);
        }
    }
    #[cfg(target_arch = "wasm32")]
    idx.sort_by(cmp);

    // Apply the permutation to each output column by MOVING cells out via
    // `Option::take`. No clones, no refcount traffic. With multi-column
    // outputs the per-column work parallelises trivially.
    #[cfg(not(target_arch = "wasm32"))]
    {
        use rayon::prelude::*;
        data.par_iter_mut().for_each(|col| {
            let mut taken: Vec<Option<CfmlValue>> =
                std::mem::take(col).into_iter().map(Some).collect();
            *col = idx
                .iter()
                .map(|&i| taken[i].take().expect("sort permutation must be a bijection"))
                .collect();
        });
    }
    #[cfg(target_arch = "wasm32")]
    for col in data.iter_mut() {
        let mut taken: Vec<Option<CfmlValue>> =
            std::mem::take(col).into_iter().map(Some).collect();
        *col = idx
            .iter()
            .map(|&i| taken[i].take().expect("sort permutation must be a bijection"))
            .collect();
    }
}

fn apply_limit_cols(
    data: &mut [Vec<CfmlValue>],
    limit: &Option<LimitClause>,
    row_count: usize,
) -> usize {
    let Some(l) = limit else {
        return row_count;
    };
    let start = l.offset.min(row_count);
    let end = l.offset.saturating_add(l.count).min(row_count);
    let new_len = end.saturating_sub(start);
    for col in data.iter_mut() {
        if start == 0 {
            col.truncate(end);
        } else {
            *col = col[start..end].to_vec();
        }
    }
    new_len
}
