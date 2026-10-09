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
