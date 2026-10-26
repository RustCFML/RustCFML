# Scope Lookup Optimization Analysis

## Current Situation
- IndexMap::get_index_of is 7.59% self time in profiles
- FxHasher is already in use
- Slot-locals system exists for first 64 declared locals
- But scope chain lookups still hit IndexMap repeatedly

## Key Findings from Code Analysis

1. `lookup_name_in_scopes` can hit IndexMap 5+ times per variable lookup:
   - locals.get(name)
   - locals.get("arguments")  
   - args.get(name) within arguments struct
   - locals.get("__variables")
   - vars.get_ci(name) within variables struct
   - globals.get(name)

2. Most local scopes are small (< 20 entries)
3. Keys are already intern


ed with pre-computed hashes

## Proposed Optimization

Instead of complex caching, use a simpler data structure for small maps:
- For maps with <= 8 entries: use inline array with linear search
- For larger maps: use IndexMap as before
- Linear search on 8 entries with pointer equality is ~3-5x faster than hash table

This avoids:
- Hash computation (even though pre-computed, still needs extraction)
- Modulo/masking for bucket selection
- Following collision chains
- Complex IndexMap code paths

## Implementation Plan

1. Add a `SmallMap` variant to ValueMap that uses a fixed-size array
2. Transparently promote to IndexMap when size exceeds threshold
3. Keep all existing APIs compatible

This is similar to SmallVec but for maps.
