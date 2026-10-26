# Scope Lookup Optimization Investigation - Summary

## Task
Reduce warm CFML scope-lookup cost. Target: IndexMap::get_index_of at 7.59% self-time in profiles.

## Investigation Findings

### 1. Current Architecture Analysis
- **FxHasher already in use**: The engine switched from SipHash to FxHasher (~3-5x faster on short keys) in v0.192
- **Key interning exists**: Keys are pre-computed with their hash at compile time via the `Name` type
- **Slot-locals system exists**: First 64 declared local variables use direct array indexing (slots), bypassing IndexMap entirely
- **Well-known keys are optimized**: Common scope names (`arguments`, `variables`, etc.) use LazyLock constants

### 2. Why IndexMap Is Still Hot

Despite the optimizations above, `IndexMap::get_index_of` remains at 7.59% because:

1. **Scope chain lookups hit IndexMap multiple times per variable**:
   - `locals.get(name)` - frame locals
   - `locals.get("arguments")` - arguments scope container
   - `args.get(name)` - within arguments struct  
   - `locals.get("__variables")` - variables scope container
   - `vars.get_ci(name)` - within variables struct
   - `globals.get(name)` - page globals
   
   Even with pre-computed hashes, each lookup still requires:
   - Hash table bucket calculation
   - Collision chain traversal
   - Key equality checks

2. **Most local scopes are small (< 20 entries)**, but still use the full IndexMap hash table machinery

3. **Parameters are not slotted** because they're part of the `arguments` bidi-sync mechanism

### 3. Attempted Optimizations

#### A. Inline Small-Map Storage (Attempted but Reverted)
- **Idea**: Use a 4-entry inline array for maps ≤4 entries, with linear search
- **Rationale**: Linear search on 4 entries with pointer equality is ~3-5x faster than hash table
- **Result**: Complex to implement correctly - requires rewriting ~20 trait implementations and careful handling of the inline→map promotion
- **Risk**: High - affects every scope operation, easy to introduce subtle bugs

#### B. Inline Cache (Considered but Not Implemented)
- **Idea**: Cache last N lookups with raw pointers
- **Issues**: Requires unsafe code and interior mutability; complex lifetime management

### 4. Baseline Performance
```
bench_loop (10M iterations): ~1055ms
LOCAL_LOOKUP (5M iterations, 5 vars): ~1137ms  
SCOPED_LOOKUP (5M iterations, 5 vars): ~1139ms
MIXED_LOOKUP (5M iterations): ~884ms
```

## Recommendations

### Short Term (Low Risk, Moderate Gain)
1. **Extend slots to cover function parameters**: Currently only `var`-declared locals use slots. Function parameters are known at compile time and could also use slots, avoiding the arguments-scope IndexMap lookup for the common case. Estimated win: ~2-3%.

2. **Add fast-path for 1-2 entry maps**: Special-case the single-entry and two-entry map cases with inline storage. These are common for simple functions. Lower implementation complexity than the full small-map optimization.

### Medium Term (Moderate Risk, Higher Gain)  
3. **Implement small-map optimization properly**: Complete the inline-array storage for maps ≤4 entries. Requires careful testing but could yield ~5-8% improvement on typical CFML code with many small scopes.

4. **Profile-guided compilation**: Use PGO (Profile-Guided Optimization) on rustc itself to optimize the hot IndexMap paths for this workload.

### Long Term (Research)
5. **Custom hash table**: Implement a specialized hash table optimized for:
   - Small maps (most CFML scopes are < 20 entries)
   - Pre-computed hashes (Keys carry their hash)
   - Ordered iteration (CFML requires it)
   - Case-insensitive equality

## Conclusion

The IndexMap cost is inherent to CFML's dynamic scope chain resolution. The existing optimizations (FxHasher, key interning, slots for locals) have already captured the low-hanging fruit. Further gains require either:

1. Reducing the **number** of IndexMap lookups (extend slots to parameters)
2. Making small maps faster with specialized storage
3. Optimizing the IndexMap implementation itself

The safest next step is #1 (extend slots to parameters), which is localized to the codegen phase and doesn't touch the hot runtime paths.

## Files for Reference
- Scope lookup: `crates/cfml-vm/src/lib.rs` ~line 25008 (`lookup_name_in_scopes`)
- Slot assignment: `crates/cfml-codegen/src/compiler.rs` ~line 497 (`assign_local_slots`)  
- ValueMap: `crates/cfml-common/src/dynamic.rs`
- Key implementation: `crates/cfml-common/src/key.rs`
