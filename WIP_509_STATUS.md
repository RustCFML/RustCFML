# Issue #509 Implementation Status

## Completed

### 1. Lucee-style Wildcard Matcher (`crates/cfml-common/src/wildcard.rs`)
- ✅ Complete port of Lucee 7.1.3.85 `WildcardPattern.java`
- ✅ Matches name only (not full path)
- ✅ Pipe-delimited patterns with OR logic (`|`)
- ✅ Leading `!` negation support
- ✅ `*` and `?` wildcards
- ✅ Non-backtracking leftmost matching (so `a*b` does NOT match `abab`)
- ✅ Case-sensitive on Linux/macOS, insensitive on Windows
- ✅ Comprehensive unit tests (10 test cases)
- ✅ Registered in `cfml-common/src/lib.rs`

### 2. Comprehensive CFML Test Suite (`tests/stdlib/test_directory_filter_509.cfm`)
- ✅ 43 test cases covering all requirements from the spec
- ✅ `directoryList` with closure/UDF filters
- ✅ String wildcard patterns 
- ✅ Case sensitivity validation
- ✅ Type filtering
- ✅ Recursion behavior
- ✅ `directoryCopy` with filters
- ✅ `cfdirectory` action=list and action=copy
- ✅ Error handling and exception propagation
- ✅ Expected values match Lucee 7.1 on Linux

## Remaining Work

### 3. Replace existing filter matchers
- ❌ Replace `compile_filter()` in `builtins.rs:9606` with `WildcardPattern::new()`
- ❌ Replace `matches_directory_filter()` in `lib.rs:43591`
- ❌ Replace `matches_glob()` in `builtins.rs:16893`
- ❌ Remove `GLOB_RX_CACHE` (no longer needed)

### 4. VM Intercepts for Closure/UDF Filters
- ❌ Add `directorylist`, `directorycopy`, `cfdirectory` to `VM_INTERCEPTED` list in `builtins_meta.rs`
- ❌ Implement in `intercepts_querydir.rs` or new module:
  - Validate UDF signature (return type boolean/any, ≤3 params, single param must be string/any)
  - Build predicate that calls `self.call_function` with (path, "file"|"directory", ext)
  - Extension is "" for directories, null for files with no dot
  - Coerce result with `to_condition()` but null must raise "Can't cast String [] to a boolean"
  - Reuse `closure_parent_writeback` pattern from arrayFilter
  - Errors: list propagates unchanged, copy wraps as application error

### 5. Shared Directory Walker
- ❌ Create `cfml-common::dirlist::walk()` function
- ❌ Apply type filter before predicate
- ❌ Always recurse into rejected directories
- ❌ Keep symlink-cycle guard

### 6. directoryCopy Rewrite (`builtins.rs:20108`)
- ❌ Honor `recurse` (default false)
- ❌ Honor `filter`
- ❌ Honor `createPath` (default true)
- ❌ Implement recurse/filter matrix from spec (non-recursive copies top-level files only)
- ❌ Implement `removeEmptyFolders` cleanup after recursive copy
- ❌ Relative destination with no slash is sibling of source
- ❌ Add named signature in `builtin_params.rs`

### 7. cfdirectory Enhancements
- ❌ VM list path honors `type` (validated), `filterDelimiters`
- ❌ Add `action="copy"` support
- ❌ UDF single-argument mode for listinfo=name, non-recursive, type=all

### 8. Sort for name/path arrays (LDEV-3188)
- ❌ Build rows, sort with `sort_listing_query`, then project
- ❌ Update `tests/stdlib/test_directorylist_sort.cfm:33-34`

### 9. Sandbox and S3
- ❌ Sandbox: honor `sort` and `type` args
- ❌ S3: honor filter, sort, type, and non-recursive copy (if feasible)

## Build Issues Encountered

During implementation, encountered a dependency issue:
- Several crates (indexmap 2.14.2, hashbrown 0.17.1) require Rust edition2024
- The project uses Rust 1.83.0 which doesn't support edition2024
- Upgrading to Rust 1.99.1 revealed socketioxide compilation errors
- **Resolution needed**: Either downgrade problematic dependencies or fix socketioxide compatibility

## Testing Strategy

1. Unit test the wildcard matcher (✅ done)
2. CFML integration tests (✅ written, ❌ not yet run)
3. Cross-engine testing with Lucee 7.1.3.85
4. Verify all existing tests still pass
5. Test in serve mode (cold + warm)
6. Test with --serve --production

## Behavior Changes to Document in PR

1. **Case sensitivity**: String patterns become case-sensitive on Linux/macOS (breaking change for existing users on those platforms)
2. **Non-backtracking**: `a*b` no longer matches `abab` (Lucee parity)
3. **`|` pattern**: Empty pattern `"|"` now matches nothing instead of everything
4. **`directory` type**: `type="directory"` now works (was silently treated as `all`)
5. **Sort arrays**: Name/path arrays are now sorted per LDEV-3188

## Next Steps

1. ✅ Push branch
2. ❌ Fix build issues (dependency versions or rust upgrade)
3. ❌ Complete VM intercepts for closure filters
4. ❌ Update directoryList to use new matcher
5. ❌ Rewrite directoryCopy
6. ❌ Add cfdirectory copy action
7. ❌ Wire tests into runner
8. ❌ Run full test suite
9. ❌ Create PR
