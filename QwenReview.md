# RustCFML Code Review

> Automated analysis of ~153K lines of Rust across 168 source files in 11 workspace members.
> Focus: inefficiencies, redundancies, security, and architectural advice.

## Executive Summary

The codebase is well-architected with clear separation of compilation pipeline stages, thoughtful use of `Arc`-based sharing, and extensive caching strategies. However, there are several areas with concrete performance opportunities and a few security concerns worth addressing.

**Priority matrix:**
| Severity | Count | Examples |
|----------|-------|----------|
| Critical (Perf) | 3 | Snapshot overuse, linear scans in hot paths, redundant allocations |
| High (Security) | 2 | Path traversal in includes, unbounded output buffer |
| Medium (Perf) | 5 | Redundant `.to_lowercase()`, VFS normalization, dedup patterns |
| Medium (Arch) | 3 | God-class files, module organization |

---

## 1. Performance Inefficiencies

### 1.1 `snapshot()` Overuse — The Dominant Cost

**Severity: CRITICAL**

`CfmlStruct::snapshot()` and `CfmlArray::snapshot()` clone the entire underlying `IndexMap`/`Vec` on every call. This pattern appears **100+ times** across the codebase, and is called implicitly by `iter()` and `into_iter()`.

**Impact:** The doc comment at `dynamic.rs:1438` quantifies this: *"iter()/snapshot() clone the whole IndexMap on every call, which on a live Preside admin profile was ~10% of total CPU."*

**Offending patterns found:**
- `dynamic.rs:1365` — struct merge uses `other.snapshot()`
- `builtins.rs:15078` — `s.snapshot()` in struct operations
- `builtins.rs:2452-2453` — dual snapshots in array comparison
- `component.rs:789,817,834` — `this_members.snapshot()` in CFC dispatch
- `dump.rs:152,185,451,466` — debug dumps (less critical)

**Recommendation:** The `with_map()` and `with_read()` methods already exist as drop-in alternatives for read-only paths. Systematically audit the 100+ call sites and replace `snapshot()` with `with_read()` wherever the iterator doesn't mutate under the lock.

---

### 1.2 Linear Dedup Scans in `all_keys()` / `all_entries()`

**Severity: CRITICAL**

`dynamic.rs:1416-1487` implements struct key/entry enumeration with a linear `any()` scan for case-insensitive dedup:

```rust
!keys.iter().any(|e| e.eq_ignore_ascii_case(k))
```

For a CFC with ~40 method table entries and ~360 instance keys, this is O(n×m) comparisons — ~1,440 string comparisons per `all_entries()` call.

**Recommendation:** Use a `HashSet` (or the existing `ValueMap`/`IndexMap` with `Key`) for O(1) case-insensitive dedup.

---

### 1.3 Redundant `.to_lowercase()` Allocations

**Severity: MEDIUM**

The compiler generates a new `String` for `.to_lowercase()` on every identifier during codegen:

- `compiler.rs:29` — `name.to_ascii_lowercase().as_str()` for reserved-scope-name check
- `lib.rs:26811` — `include_path.to_lowercase()` in include mapping resolution
- Multiple custom-tag resolution paths

The `Key` type was designed to eliminate this pattern, but raw string comparisons still appear in hot paths.

**Recommendation:** Use the `Key` type consistently for all case-insensitive identifier comparisons. For the include path specifically, a single pre-lowercased cache key would avoid repeated allocations across requests.

---

### 1.4 VFS Path Normalization Allocates 3+ Strings per Call

**Severity: MEDIUM**

`vfs.rs:152-187` normalizes paths with:
```rust
path.replace('\\', "/")      // Allocation 1
.to_lowercase()             // Allocation 2
split('/').join("/")       // Allocation 3+
```

For serve-mode processes hammering the include/custom-tag resolution path, this is significant.

**Recommendation:** Use a stack-allocated buffer with in-place normalization. The `PathBuf` type has enough capacity in most cases.

---

### 1.5 Xml Serialization Chatty Allocations

**Severity: MEDIUM**

`dynamic.rs:825-898` serializes XML with per-attribute heap allocations:
- `as_string()` on each attribute value (Arc clone → String allocation)
- `xml_escape_into` iterates char-by-char, pushing into a `String`

For large XML documents, this is a chatty allocation pattern.

**Recommendation:** Pre-allocate the output buffer with estimated capacity. Consider `SmallString` for short attribute values (common case: <24 chars).

---

### 1.6 CfmlValue Enum Size is at Ceiling

**Severity: MEDIUM**

The `CfmlValue` enum is at its documented 32-byte ceiling (guarded by assertion at `dynamic.rs:3539`). With ~20 variants, adding a new variant risks bloaching the enum, which impacts:
- Cache line utilization in `IndexMap<Key, CfmlValue>`
- Stack frame sizes in the VM dispatch loop
- `CfmlQuery` column storage (Column-Oriented layout)

**Recommendation:** If adding new variants, use `Box<T>` for the larger ones. Consider a tag-pointer layout for the Cranelift JIT target.

---

### 1.7 Query Column Access Pattern

**Severity: LOW-MEDIUM**

`CfmlQuery` uses Column-Oriented storage (`Vec<Arc<Vec<CfmlValue>>>`), which is optimal for Query-of-Queries but causes cache-line thrashing during row iteration (the common `<cfloop query=` case). Each row iteration accesses N column Vectors at the same index.

This is a known tradeoff and is acceptable for the QoQ use case. Not urgent.

---

## 2. Security Concerns

### 2.1 Path Traversal in Include Resolution

**Severity: HIGH**

`lib.rs:26784-26805` (`resolve_leading_slash_include`) resolves include paths by:
1. Stripping leading `/`
2. Joining with webroot/base directory
3. Checking existence

**The vulnerability:** `..` segments in the include path are NOT stripped. A CFML include like `<cfinclude template="/../../etc/passwd">` would resolve to `webroot + "../../etc/passwd"` which could escape the webroot depending on the webroot path.

The `EmbeddedFs::normalize` function does strip `..` segments (vfs.rs:180), but only for the **embedded filesystem**, not the real filesystem path used during include resolution.

**Mitigation:** Add `..` stripping to `resolve_leading_slash_include`. Or route include resolution through the VFS canonicalize path which handles normalization.

### 2.2 Path Traversal in Custom Tag Resolution

**Severity: MEDIUM**

`lib.rs:26973` (`find_custom_tag_deep`) uses VFS directory entries (bounded by `MAX_DEPTH=8`, `MAX_DIRS=2048`), which is good for DoS. However, if directory entries contain `..` in their names, `Path::join` does not inherently normalize.

The depth/directory caps provide partial protection, but a crafted directory structure could still traverse.

**Mitigation:** The `find_custom_tag_deep` function iterates VFS entries, not raw strings, so the attack surface is limited. Low priority but worth a defensive normalization.

### 2.3 Output Buffer Unbounded Growth (DoS)

**Severity: MEDIUM**

`vm.rs:223` — `output_buffer` is a single `String` per request with no size cap. A `<cfloop>` with `<cfoutput>` writing large values can grow to hundreds of MB. The `saved_output_buffers: Vec<String>` stack also grows per `<cfsavecontent>` iteration.

**Mitigation:** Add a configurable `maxOutputBufferSize` setting (Lucee defaults to 512KB for `<cfsavecontent>`).

### 2.4 Scope Injection via Runtime Path Writes

**Severity: LOW-MEDIUM**

`store_runtime_path` (lib.rs:21118+) allows dotted-path writes like `variables.x.y.z = v`. Structural keys (`__variables`, `__name`, etc.) are `__`-prefixed and mostly protected, but edge cases exist where user-controlled paths could overwrite engine-internal keys.

**Mitigation:** Add a reserved-key guard in `store_runtime_path` that rejects writes to `__`-prefixed keys outside of engine-internal code paths.

### 2.5 Native Object Reentrancy Deadlock

**Severity: LOW**

`CfmlValue::NativeObject` carries a `parking_lot::RwLock<dyn CfmlNative>`. If a native method calls back into the same object (e.g., `this.foo()` calls `this.bar()`), the `RwLock` (which is non-reentrant) will deadlock.

**Mitigation:** Document the non-reentrant contract in `CfmlNative` trait. Or switch to a reentrant `MpscRwLock` for the hot path.

---

## 3. Architectural Observations

### 3.1 God-Class Files

| File | Lines | Suggested Split |
|------|-------|-----------------|
| `lib.rs` | 36,305 | dispatch, scope, frame, output, cache, include, tags, dump |
| `builtins.rs` | 19,297 | string, array, struct, date, io, xml, json, query, scope |
| `compiler.rs` | 5,978 | expressions, statements, scope, literals, functions |

The `ops/` module in cfml-vm is a good start (extracting per-op handlers). This should continue as a progressive refactoring.

### 3.2 Positive: Caching Strategy is Sophisticated

The codebase has multiple layers of caching:
- `canonicalize_cache` (cross-request, production-only)
- `component_path_cache` (fingerprint-gated)
- `request_exists_cache` (per-request negative memo)
- `arg_sources_memo` (per-call-site bytecode analysis)
- `resolved_fn_memo` (UDF wrapper dedup)

These are well-designed and documented with profiling data.

### 3.3 Positive: Key Type is Well-Designed

The `Key` type with pre-computed hash eliminates per-lookup lowercase allocations. The `well_known` module with `LazyLock<Key>` for reserved names is a good pattern.

### 3.4 Positive: Memory Safety

- Reference-typed containers use `Arc<RwLock<...>>`
- `Weak` references prevent Arc cycles
- `ValueMap::version()` detects mutation without deep diffing
- Cycle GC is request-scoped and conservative

---

## 4. Recommendations by Priority

### Immediate (This Release)
1. **Fix path traversal in `resolve_leading_slash_include`** — Add `..` segment stripping
2. **Replace `snapshot()` with `with_read()` in CFC dispatch paths** — `component.rs` lines 789, 817, 834
3. **Add output buffer size cap** — Configurable, default 512KB

### Short-Term (Next 3 Releases)
4. **Fix `all_keys()`/`all_entries()` dedup** — Replace O(n×m) linear scan with `IndexMap`-based dedup
5. **Eliminate redundant `.to_lowercase()` in compiler hot paths** — Use `Key` type
6. **VFS path normalization optimization** — Stack buffer approach

### Medium-Term (Next 10 Releases)
7. **Split `lib.rs`** — The `ops/` module is a good start; continue extracting by concern
8. **Split `builtins.rs`** — Module by domain (string, array, struct, date, io, xml, json)
9. **Add reserved-key guard to `store_runtime_path`** — Reject `__`-prefixed writes from user code

### Nice-to-Have
10. **Constant folding in bytecode compiler** — Arithmetic on two integer constants
11. **Jump threading optimization** — Collapse consecutive jumps
12. **String interning for literal strings** — Extend `Key` pattern to codegen string literals

---

## 5. Code Quality Notes

**Strengths:**
- Excellent doc comments with profiling data and rationale
- Well-structured compilation pipeline
- Strong typing with `CfmlValue` and `CfmlResult`
- Extensive caching with clear lifecycle management
- Case-insensitive semantics properly encapsulated in `Key` type
- Fused bytecode ops reduce dispatch overhead
- Slot-based local variable access

**Concerns:**
- No linter output visible (no `clippy` or `rustfmt` config examined)
- 153K LOC is substantial for a single-developer or small-team project
- Some error handling uses `unwrap_or` in hot paths (performance tradeoff)
