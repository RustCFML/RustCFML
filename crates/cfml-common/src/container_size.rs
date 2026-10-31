//! Calibrated sizes for the shared containers, for memory REPORTING only
//! (the debug footer's memory panel and object census). Nothing allocates
//! from these; they exist so the two estimators cannot drift apart again —
//! each used to carry its own copy of the same sketch.
//!
//! The constants are MEASURED, not guessed. `32` per container and `40` per
//! struct entry were a sketch of "Arc header + lock + IndexMap headers", and
//! they under-counted by 2-4x. Held against the engine's own exact allocation
//! accounting ("Still in use as the page ends") over 300,000 containers of
//! each shape:
//!
//! | shape          | real   | old sketch | now   |
//! |----------------|--------|-----------|-------|
//! | empty struct   |  246 B |      56 B |  208 B |
//! | 1-key struct   |  346 B |      98 B |  282 B |
//! | 3-key struct   |  458 B |     182 B |  430 B |
//! | 8-key struct   |  846 B |     392 B |  800 B |
//! | 16-key struct  | 1438 B |     735 B | 1392 B |
//! | empty array    |  118 B |      56 B |   76 B |
//! | 1-elem array   |  214 B |      80 B |  172 B |
//! | 4-elem array   |  214 B |     152 B |  172 B |
//! | 16-elem array  |  502 B |     440 B |  460 B |
//!
//! On a warm Preside server whose application scope is ~1M containers, the old
//! sketch hid a couple of hundred megabytes in the footer's unattributed
//! "Other" remainder, which made the panel read as if the engine had lost
//! three quarters of its memory.
//!
//! What the sketch missed is the containers' real shape: a struct is an
//! `Arc<RwLock<StructInner>>` — an Arc control block, a lock, and an
//! `IndexMap`, which is BOTH a hashbrown index table and a `Vec<Bucket>`; a
//! `Bucket<Key, CfmlValue>` is 8 + 24 + 24 bytes before the table's own slot
//! and control byte.
//!
//! To re-derive after a layout change: hold N containers of one shape alive in
//! a request and compare the footer's retained figure with the census column.

/// Fixed cost of a struct container, before any entry.
pub const STRUCT_BASE: usize = 208;
/// Per-entry cost of a struct: bucket (hash + key + value) plus the index
/// table's slot and control byte, averaged over the capacity classes.
pub const STRUCT_ENTRY: usize = 74;
/// Fixed cost of an array container, before any slot.
pub const ARRAY_BASE: usize = 76;
/// Per-slot cost of an array: one `CfmlValue`.
pub const ARRAY_SLOT: usize = 24;
/// A non-empty `Vec` never costs less than this many slots in practice —
/// allocator size classes make a 1-element array cost what a 4-element one does.
pub const ARRAY_MIN_SLOTS: usize = 4;

/// The most elements one call may ADD to a container (array slots, query rows,
/// padding characters). `a[4000000000] = 1`, `arrayResize(a, 4e9)`,
/// `queryAddRow(q, 4e9)` and `lJustify(s, 4e9)` each used to grow element by
/// element until the process was killed or the allocator aborted — a single
/// user-supplied index was enough. Lucee has no such cap and relies on the JVM
/// heap limit; here the call throws a CFML error that names the index instead.
/// 100M slots is ~2.4 GB of `CfmlValue`, already far past anything a request
/// legitimately builds in one write.
pub const MAX_GROW_PER_CALL: usize = 100_000_000;

/// Refuse a call that would ADD more than [`MAX_GROW_PER_CALL`] elements to a
/// container in one write. The error names the function, the argument and the
/// size asked for. Shared by the stdlib BIFs and the VM's in-place
/// `queryAddRow` intercept so the two paths cannot drift.
pub fn check_grow_per_call(
    fn_name: &str,
    arg_name: &str,
    target_len: i64,
    current_len: usize,
) -> Result<(), crate::vm::CfmlError> {
    let grow = target_len as i128 - current_len as i128;
    if grow > MAX_GROW_PER_CALL as i128 {
        return Err(crate::vm::CfmlError::expression(format!(
            "Invalid call of the function [{}], Argument [{}] is invalid, growing to [{}] \
             elements would add [{}] in one call; the maximum is [{}]",
            fn_name, arg_name, target_len, grow, MAX_GROW_PER_CALL
        )));
    }
    Ok(())
}

/// The row count for `queryAddRow(q, n)`: a negative count is refused
/// (`queryAddRow(q, -1)` used to cast to usize::MAX and silently add nothing)
/// and one past [`MAX_GROW_PER_CALL`] is refused too (`queryAddRow(q,
/// 4000000000)` pushed rows until the process was killed).
pub fn query_add_row_count(n: i64, current_rows: usize) -> Result<usize, crate::vm::CfmlError> {
    if n < 0 {
        return Err(crate::vm::CfmlError::expression(format!(
            "Invalid call of the function [queryAddRow], second Argument [number] is invalid, \
             must be a non-negative integer; now [{}]",
            n
        )));
    }
    check_grow_per_call("queryAddRow", "number", n.saturating_add(current_rows as i64), current_rows)?;
    Ok(n as usize)
}

/// Reporting size of a struct container holding `len` entries, excluding the
/// entries' own keys and values.
#[inline]
pub fn struct_bytes(len: usize) -> u64 {
    (STRUCT_BASE + len * STRUCT_ENTRY) as u64
}

/// Reporting size of an array container holding `len` elements, excluding the
/// elements themselves.
#[inline]
pub fn array_bytes(len: usize) -> u64 {
    // A `Vec` grown by pushing — which is how CFML arrays are built — holds a
    // power-of-two capacity, so a 300,000-element array really owns 524,288
    // slots. Counting `len` alone read 1.75x light on a large one.
    let slots = if len == 0 {
        0
    } else {
        len.max(ARRAY_MIN_SLOTS).next_power_of_two()
    };
    (ARRAY_BASE + slots * ARRAY_SLOT) as u64
}

// ---------------------------------------------------------------------------
// Reporting hooks
// ---------------------------------------------------------------------------
//
// A couple of the debug footer's pots describe state that lives in
// `cfml-stdlib` (the compiled-regex cache, the database pool manager), but
// `cfml-vm` — which builds the panel — depends on `cfml-stdlib` only
// OPTIONALLY, under the `s3` and `mcp-client` features. Calling into it
// directly compiles in a workspace build, where something else has already
// linked the crate, and FAILS in `cargo test -p cfml-vm`, which is what CI
// runs. (It did: v0.724.0's Tests workflow, while a local
// `cargo test --workspace` stayed green.)
//
// So the owning crate installs a function pointer here at startup and the
// panel reads it, with a sensible answer when nothing has registered.

use std::sync::OnceLock;

static REGEX_CACHE_CENSUS: OnceLock<fn() -> (usize, u64)> = OnceLock::new();
static DB_POOL_COUNT: OnceLock<fn() -> usize> = OnceLock::new();

/// Install the compiled-regex cache's census. Called by `cfml-stdlib`.
pub fn set_regex_cache_census(f: fn() -> (usize, u64)) {
    let _ = REGEX_CACHE_CENSUS.set(f);
}

/// `(patterns, bytes)` in the compiled-regex cache, or `(0, 0)` in a build
/// that has no regex cache registered.
pub fn regex_cache_census() -> (usize, u64) {
    REGEX_CACHE_CENSUS.get().map_or((0, 0), |f| f())
}

/// Install the database pool count. Called by `cfml-stdlib`.
pub fn set_db_pool_count(f: fn() -> usize) {
    let _ = DB_POOL_COUNT.set(f);
}

/// Open database connection pools, or `0` in a build with no database drivers.
pub fn db_pool_count() -> usize {
    DB_POOL_COUNT.get().map_or(0, |f| f())
}

static DB_POOL_CONNECTIONS: OnceLock<fn() -> (usize, u32, u32)> = OnceLock::new();

/// Install the pooled-connection census. Called by `cfml-stdlib`.
pub fn set_db_pool_connections(f: fn() -> (usize, u32, u32)) {
    let _ = DB_POOL_CONNECTIONS.set(f);
}

/// Pooled MySQL connections: (pools, connections, of which idle). Zeros in a
/// build without the MySQL driver.
pub fn db_pool_connections() -> (usize, u32, u32) {
    DB_POOL_CONNECTIONS.get().map_or((0, 0, 0), |f| f())
}
