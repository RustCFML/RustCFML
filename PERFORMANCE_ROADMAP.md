# RustCFML Performance Plan — canonical, consolidated

*Working doc, UNTRACKED — never commit (hook blocks `.md` commits without `ALLOW_MD=1`;
this file should not be committed even with it). Consolidated 2026-08-21 at v0.613.0 from
the old PERFORMANCE_ROADMAP.md, functionanalysisPlan.md (both absorbed; the latter
deleted), and the Rust-native component analysis of 2026-08-21. This document is the
single source of truth: every stale layer of its predecessors has been removed, and the
SHIPPED / MEASURED-DEAD lists below are verified against git tags, not memory.
Revised 2026-08-22: status consolidated into THE LEDGER, Part 4 renumbered, three
dead-list entries retracted.*

**How to read this doc:** the LEDGER immediately below is the master status. Every part
is either a **WORK ITEM** (something to build) or a **CAMPAIGN** (measurement that produces
evidence, not code) — the doc never used to distinguish them, so "Part 1 COMPLETE" read as
"the work is done" when it means "the measuring is done". Where a section heading disagrees
with the ledger, **the ledger wins.** Rules (Part 6) and the dead list (Part 7) are
non-negotiable context — read them before touching anything.

---

# ⭐ THE LEDGER — done vs to do (master status, 2026-08-22)

## ✅ DONE — built, measured, **IN THE TREE** (v0.614.0 → v0.616.0)

| # | item | outcome | shipped |
|---|---|---|---|
| **P1** | interception made declarative — `cfml-common/src/builtins_meta.rs` + 3 build-failing guards | **−6.81% admin render, p=0.0022**; caught a live **sandbox bypass** and a try/catch escape | v0.614.0 |
| — | ↳ the compile-time binding of ~446 builtins that P1 unlocked | **this IS Part 2.7's "lower the remaining predicates, 1.5–2.4 ms" — that lever is now SPENT**, not open | v0.614.0 |
| **P5** | CLAUDE.md recipe rewritten to forbid appending to `call_function` | the mechanism that stops P1/P2 regrowing | v0.614.0 |
| **P2** | `call_function` collapsed — 4 slices extracted | 7,497 → 6,888 lines; `lib.rs` 36,515 → 35,950. **Target revised: the rest is core dispatch, not clutter** | v0.614.0 |
| **3B S0/S1** | instance member access encapsulated; per-class member `Shape` on `ClassBlueprint` | structure only — 3B's two perf claims were built and measured ZERO, see Part 3B | v0.615.0 |
| **⭐ rank 1** | **default parameters — `BytecodeOp::SeedArgumentKey` + the `arguments`-scope ownership invariant** | **−3.87% uncached Preside render traffic, p=0.034** (8 legs/arm × 900 renders). Reproduces the reverted prototype's −3.55%/p=0.018 on a different rig. No regression elsewhere: preside +0.09%, wheels −1.06%, testbox −0.16% | v0.616.0 |
| — | ↳ ⭐ the blocker was a **pre-existing Lucee divergence**, not something the lever created | a lazy frame inherited the CALLER's `__arguments_scope` Arc handle and wrote its own params into it. Fixed + pinned in `tests/functions/test_param_scope_ownership.cfm` | v0.616.0 |
| **⭐ instrument** | **per-frame EXCLUSIVE census** — `--features frame-census` + a counting allocator wrapping mimalloc | ns / allocations / bytes / ops per frame with CHILDREN SUBTRACTED, aggregated per function. The instrument ledger rank 1 asked for | v0.617.0 |
| **⭐⭐ finding** | **the +496 ns/frame surcharge is HALF allocation volume** | Preside **37.3 allocs/frame** vs a synthetic frame's **17.0**. ⚠️ the two-point slope (24.5 ns/alloc) was **WRONG BY 2×** — an intervention removing exactly 5 allocations measured **~13 ns each**, so allocation is ~264 ns of 496 and **half the surcharge is still dark** | — |
| **rank 1 follow-on** | `InheritedKeys` stores interned `Key`s, not `String`s | **−7.21% procfd p=0.002** · Preside ~0.5% (below A/B resolution) · allocs/frame 17.0→12.0 synthetic, 37.28→35.95 Preside. Also fixes a latent case-SENSITIVE set in a case-insensitive language | v0.617.0 |

## ✅ DONE — shipped and tagged
Part 8, v0.512 → v0.613. Nothing there needs revisiting.

## ✅ CAMPAIGNS COMPLETE — evidence banked, **no code owed by any of them**

| part | what it settled |
|---|---|
| Part 0 | baseline **20.7 ms vs Lucee 9.3 ms = 2.22×**, 91% CPU-bound in-request |
| Part 1 | H1 retired · H2 refuted · 3B gated off · reconciliation stalls at 49% |
| Part 2 | unslotted frames REFUTED (5.1% ceiling) · 4 hypotheses killed · defaults root-caused |
| Part 2.5 | the workload was too thin — cost model across 4 workloads |
| Part 2.6 | ⭐ **attribution SOLVED at 99%** — frames 71%, ops 19%, fixed 9% |
| Part 2.7 | inside the frame: the prologue is NOT the surcharge; 3 component hypotheses killed |

## 🔴 TO DO — **nothing below is started; no code in the tree for any of it**
Ordered by value ÷ effort, not by part number.

| rank | item | size | blocked on | detail |
|---|---|---|---|---|
| ~~1~~ | ~~default parameters + P4-S3 return-time diff~~ | ✅ **BUILT AND SHIPPED v0.616.0, −3.87% p=0.034** | — | see DONE above · Part 2 |
| **1** | decompose the **REST of the +496 ns/frame surcharge** *(half now explained: allocation volume)* | ~232 ns/frame still dark, plus Preside's remaining ~36 allocs/frame | instrument now EXISTS (`frame-census`). DHAT is unusable on Preside — use targeted counters | Part 2.6 · Part 2.7 · [alloc memory] |
| **2** | re-measure the phase table on v0.615 | prerequisite, not a lever | nothing | Part 4 item 2 |
| **3** | **P4-S2** lazy structural keys | ≤0.47 ms (~1–2%) | 101 read sites; full-session job | P4 · Part 4 item 3 |
| **4** | root-cause **page UDF = 3.2× a sibling method call** | unsized | not root-caused. Preside CANNOT see it — needs the 2nd workload | Part 4 item 4 |
| **5** | `LineInfo` — 10.89% of every op executed, pure bookkeeping | ~0.13 ms ⇒ **clarity item, not a speed one** | nothing | Part 4 item 5 |
| **6** | toolchain sweep — BOLT · `-Zbuild-std` · `opt-level`/`target-cpu` | never systematically A/B'd | nothing | Part 4 item 8 |
| **7** | hygiene: String-literal `Arc` · `snapshot()` census · O(n×m) CI probes | sub-1% each | only worth doing inside a wider allocation sweep | Part 4 items 10–12 |
| — | **Part 5 — app-level** (isFeatureEnabled, WireBox DI, HandlerService) | biggest per-hour wins available | **NOT engine work** — these are Preside-side fixes | Part 5 |

## ⛔ NOT DOING

*(Part 3B: un-retired, BUILT (IC + op fusion), measured zero on all four real workloads,
re-closed 2026-08-22 — this time on every axis it ever claimed. S0/S1 survive as committed
structure. Final verdict in Part 3B.)*

| item | why |
|---|---|
| **Part 3B** shape-based instances | **closed by measurement on every axis** — footprint 0.07%, GC zero, instantiation n=2, lookup zero (IC built, 98.6% hit, no win), op dispatch zero (fusion built, ±0.5%, shrinks JIT admission). Both builds reverted |
| item | why |
|---|---|
| **P3** split `lib.rs` | already rejected on the record; P2 is the sanctioned incremental route. `lib.rs` shrinks as a *consequence* of P2 |
| **Part 3A** arena, stages S1/S4/S5 | 0.61 ms total (3.2%) for a multi-week 101-site migration with silent failure modes. Only S2/S3 survive — as P4 above |
| **Part 3C** cycle-GC / RwLock-per-read | ablates to **zero** on the workload that allocates 15,384 structs/render |
| Part 7's ~30 entries | each measured dead. ⚠️ **3 of them were WRONG and are retracted at the top of Part 7** |

## The finding that should govern the ORDER of everything above
**The tidy-up out-performed every performance campaign.** P1 — structural work, no lever
hunting — measured **−6.81%**. The best lever the five campaigns found is **−3.55%**, and
it is blocked. ⇒ **Do the structural work; the speed follows.** Argument in Part -0.5.


---

# PART -1 — TIDY-UP PLAN — 🟢 **THE ACTIVE TRACK** (P1 ✅ · P5 ✅ · P2 🟡 · P3 ⛔ · P4 🔴)

User directive: stop re-challenging, stop benchmarking everything, fix the structural mess
in a sensible order. **Testing policy for every phase below: the bar is "all suites still
green". These are correctness-preserving refactors — do NOT A/B them, do NOT size them,
do NOT prove the win first.** Verify at phase boundaries, not per step.

## The root cause (evidenced 2026-08-22)
`call_function` is **7,496 lines (12,820-20,316) = 20% of lib.rs (36,515)**. CLAUDE.md
still says its intercept list is at "~line 1718" — 11,000 lines stale. This is not drift:
**CLAUDE.md's own recipe for adding a VM-intercepted builtin instructs you to append to
this function**, and nothing ever removes from it. Cross-cutting concerns
(`sandbox_intercept`, `s3_intercept`, `resolve_file_bif_paths`) were then woven INTO the
chain rather than layered around it, which is what makes "is this name intercepted?"
unanswerable without executing the function.

Contrast: Lucee DECLARES its BIFs (`FunctionLibFunction` carries class + arg types, read at
compile time). We CODE ours into a chain. Declarations stay enumerable; chains do not.
⇒ **The fix is to change the EXTENSION MECHANISM, not to tidy — tidying regrows.**

## P1 — Make interception declarative — ✅ **DONE** (keystone; everything else depended on it)

**Built, green, uncommitted. Measured −6.81% on the live admin (p=0.0022).** It caught a
live sandbox bypass (`fileWrite`/`fileDelete`/`cfexecute` among 51 undeclared names) and a
try/catch escape. The three items below are what was specified; all three exist in the tree.
1. `BUILTIN_INTERCEPTS` — one declared list (cfml-common, so codegen + VM both read it).
2. **The closing mechanism:** a test that scans `lib.rs`, extracts every `name_lower`
   comparison literal inside `call_function`, and asserts each is declared. Appending a new
   intercept without declaring it then FAILS THE BUILD. This is the thing the architecture
   has never had.
3. Codegen derives lowering from it — **delete the hand-curated `DIRECT_BUILTINS`**; lower
   anything registered and not intercepted. Removes the sandbox-bypass footgun and picks up
   the long tail for free.

## P2 — Collapse `call_function` — 🟡 **PARTLY DONE, TARGET REVISED, NOT RESUMING**

**7,497 → 6,888 lines.** Extracted, each verified green:

| slice | domain | lines |
|---|---|---|
| 1 | realtime / WebSocket | 304 |
| 2 | output (writeOutput/writeDump) | 123 |
| 3 | query / directory | 62 |
| 4 | **a 151-line `match` arm whose entire body was a comment** — 144 names listed only to block a fall-through | 151 → 1 |

⚠️ **The original "<1,000 lines" target was wrong and is withdrawn.** Inspecting what
remains: the bulk of `call_function` after the intercepts is the **core dispatch pipeline**
(S3 → `resolve_file_bif_paths` → sandbox → `builtin_lookup_ci` → invoke → GH#284 post-write
cache flush). That is legitimate sequential logic, not accumulated clutter, and extracting
it would obfuscate rather than tidy. The clutter is largely gone; further slices are
small-value (individual `arraysort`/`arrayfind` arms).

### The pattern (reusable; `/tmp/extract_slice.py`)
`intercepts_<domain>.rs` with `handles(name)` next to the implementations + a `dispatch_*`
holding bodies moved **verbatim** (owned `Vec<CfmlValue>` exactly as the original scope had,
so `?`, `return Ok(..)`, `&args` and `into_iter()` all still work — a pure move, reviewable
as such).

### Two traps this hit (both now encoded in the code)
1. **A lifted guard must reproduce every condition under which the original chain
   CONTINUED, not just the name.** `cfdirectory` returns for `action="list"` and falls
   through otherwise; an args-blind `handles()` turned that into a hard error.
2. **Fall-through needs a sentinel, not a cleverer predicate.** `intercepts_common::unhandled()`
   — encoding inner conditions in `handles()` duplicates them where they drift.
3. ⚠️ **Extraction silently weakened P1's guard**: the scanner used
   `include_str!("../src/lib.rs")`, so every moved intercept left its view while the test
   kept passing. It now discovers `intercepts_*.rs` from disk.

## P3 — Split `lib.rs` — ⛔ **STRUCK. It contradicts a decision already on the record.**

Part 4's Qwen triage already REJECTED "big-bang lib.rs/builtins.rs splits (churn vs pending
PRs and the stale-base problem; **the incremental `ops/` extraction is the sanctioned
route**)". P2 IS that incremental route, so P3 was me re-proposing something the doc had
already ruled out — exactly the kind of re-litigation the user called a halt to.
`lib.rs` shrinks as a CONSEQUENCE of P2 (36,515 → 35,950 so far), not as its own project.

## P4 — Scope model (Part 3A S2/S3) — 🔴 **NOT STARTED. This is ledger ranks 1 and 4.**

Two pieces, and they are the SAME machinery as Part 4's top perf lever:
- **S2 lazy structural keys** — seeded 16,167x/render, read 487x (33:1 never read).
  ⚠️ Part 4 item 2 already warns its value DECAYED post-interned-`Key` (seeding now clones
  a `Key` = refcount bump), so ~1-2%, and it needs 101 read sites touched.
- **S3 simplify the return-time diff** — 9,980 entries scanned to move 26 (384:1).
  ⭐ **This is also the blocker for the default-parameter lever (Part 4 item 0,
  −0.77 ms / −3.55% MEASURED)**: that prototype was reverted because the diff relies on the
  arguments-scope guard to keep a defaulted param out of the caller. So simplifying the
  diff and banking the biggest remaining perf lever are ONE job, not two.

## P5 — Rewrite CLAUDE.md's recipe — ✅ **DONE**
The recipe now forbids appending to `call_function`, states why (7,497 lines; the doc's own
"~line 1718" was 11,000 lines stale), and directs to `VM_INTERCEPTED` + an `intercepts_*`
module + the `unhandled()` sentinel. This is what stops P1-P3 regrowing.

---

# PART -0.5 — REVIEW: everything above, judged by the simplification lens — 📖 **ANALYSIS**

## ⭐⭐⭐⭐ The finding that should drive what happens next
**The tidy-up produced a bigger measured win than any of the performance campaigns.**

| lever | measured | came from |
|---|---|---|
| **compile-time builtin binding (P1's declaration unlocked ~446 builtins)** | **−6.81% admin** | tidy-up |
| default parameters (reverted, blocked) | −3.55% | Part 2 |
| slot-indexed member access (est.) | ~2% | Part 2.7 |
| 3A scope model, BOTH halves | ~1% | Part 3A |
| cycle-GC removal | **0%** | Part 2.7 |

Parts 1 / 2 / 2.5 / 2.6 / 2.7 are all COMPLETE and their honest joint conclusion is that
**no individual perf lever they identified exceeds ~3.5%**. The 6.81% came from making
interception declarative — i.e. from fixing the structure, with the speed as a side effect.
⇒ **Prefer the tidy-up track over resuming lever-hunting.**

## ⚠️ Every % in Parts 0-3 is a FLOOR
All of it was measured on a scratch Preside with placeholder content (a ~60 ms admin
render). A production site with real content and data volumes is heavier and these levers
scale with it. Do NOT quote these numbers as ceilings, and do not use "it's only N%" to
drop a stage — that mistake was made on 2026-08-21 and called out.

## Status of every part
**Moved — see THE LEDGER at the top of this doc.** It is the single master status
table; this section used to carry a second, drifting copy of it.

## What the simplification lens changes about the open perf items
- **Item 0 (default params, −3.55%) and P4-S3 are ONE job.** The prototype was reverted
  because the return-time diff relies on the arguments-scope guard. Simplifying the diff
  IS unblocking the lever.
- **Item 2 (structural keys) decayed** post-interned-`Key` to ~1-2% and needs 101 read
  sites. Only worth doing as P4-S2, never standalone — the doc already said so.
- **Item 1 (`LineInfo` = 10.89% of all executed ops, 23,304/render, pure bookkeeping)** is
  the most interesting item under this lens: the 2nd most-executed opcode buys nothing at
  runtime. Sized at only ~0.13 ms, so it is a CLARITY item, not a speed one — but it
  inflates every op-count estimate in this document.
- **Item 6 (String-literal op allocates per execution)** — sub-1%, only as part of a wider
  allocation sweep. Unchanged.

# PART 0 — State of play — ✅ **CAMPAIGN COMPLETE** (baseline; no code owed)

## The baseline (measured 2026-08-20 on v0.612.0 `--profile release-pgo`)

| | TTFB median | p10–p90 |
|---|---|---|
| RustCFML v0.612 PGO | **20.7 ms** | 19.8–21.6 |
| Lucee 7.0.4 | **9.3 ms** | 8.4–11.3 |

**2.22×, gap 11.4 ms.** Same site (readyintelligencewebsite), same 5,230-byte warm
homepage, `?cb=` cache-bust, keep-alive, debug footer OFF on both, run twice. We are
**91% CPU-bound in-request** (18.9 ms CPU / 20.7 ms wall) ⇒ a compute gap, no hidden I/O
stall. v0.613.0 shipped after this measurement (worth ~0.2% on Preside; bare UDF calls
523→209 ns = **1.39× Lucee** — procedural CFML is near parity now; the gap concentrates
in component-heavy execution).

⚠️ A local `cargo build --release` is NOT PGO'd (~4.8% slower). Cross-engine parity
numbers must come from `--profile release-pgo` +
`RUSTFLAGS="-Cprofile-use=$(pwd)/pgo/rustcfml.profdata"`. RustCFML-vs-RustCFML A/Bs on
plain release builds are fine — deltas transfer.

## Where attribution stands — ⚠️ SUPERSEDED BY PART 1's VERDICT (2026-08-21)

*The framing this section used to carry — "~7 ms is unattributed and sits inside BODIES
(ph6 ≈ 5.7 ms + BIF bodies ≈ 1.6 ms)" — was tested and is WRONG on both halves. BIF
bodies are not the cost (the whole BIF gap vs Lucee is 0.97 ms, and the worst ratios are
on body-free type predicates); and pricing every counted op at its measured cost
explains only 49% of the render. Read PART 1 before using any number here.*

What survives as measured fact:

| | per warm render |
|---|---|
| bytecode ops executed | 213,966 |
| BIF calls | 8,343 (measured cost 1.50 ms; Lucee 0.52 ms) |
| UDF/method frames | 8,193 — of which **66% run UNSLOTTED** |
| struct allocations | 5,427 (3,902 tracked + 1,525 untracked) |
| component instances created | **2** (5,367 at boot) |
| implied real cost per op | **88.3 ns** vs 16.6 ns microbenchmarked ⇒ **5.3×** |

Corroborating data point, now consistent with the verdict:
`presideWireBoxDIIssue.md` (v0.609, debug footer self-time) — WireBox DI machinery alone
is ~3.9 ms (18%) of a warm request across ~575 component-method calls. Read with Part 1,
that is not "slow bodies" either: it is ordinary CFML paying the 5.3× real-frame
multiplier.

## Qualitative shape (older measurements, directionally valid)

- Engine+HTTP floor ~0.6 ms · +Preside framework ~2.9 ms · the rest is page/content
  execution ⇒ the floor and the framework are NOT the gap.
- Per-primitive vs Lucee (2026-08-14): struct/data reads 1.3× (parity) · BIF calls 3–6× ·
  CFC method call 3.2×.
- Boot ≈ parity (~8 s vs 7 s); boot executes 38 M ops through the same call path, so every
  call-path lever is a boot lever. No boot-specific work is currently justified.

---

# PART 1 — THE CAMPAIGN — ✅ **COMPLETE (2026-08-21). MEASUREMENT ONLY; VERDICT BELOW.**

*Executed 2026-08-21 at v0.613.0. Instruments re-added, both benches built and run
cross-engine against Lucee 7.1.0.204 on the same webroot. Raw artefacts and the harnesses
are listed at the end of this part. Read the verdict before proposing any lever.*

## ⭐ THE VERDICT

**H1 (BIF bodies are slower) — RETIRED as a track, but RE-ATTRIBUTED.**
The whole measured BIF gap is **0.97 ms/render = 8.5% of the 11.4 ms gap**, below the
1 ms bar the rule set, so no body-optimisation track opens. But the *shape* refutes the
premise: the ratios are LARGEST on the BIFs with no body at all — `isNumeric` 12.5x,
`isBoolean` 8.5x, `isArray` 7.5x, `isStruct` 7.3x. Lucee answers a type predicate in
9–20 ns; we take 105–150 ns. **What H1 called "slow bodies" is a ~100 ns per-call floor
at the builtin boundary.** Proof, in-source: `isNull(ident)` is lowered by codegen to
`TryLoadLocal` + `IsNull` and never enters the builtin path — it costs **34 ns**, while
same-weight predicates that do enter it cost 105–150 ns. Available if driven to zero:
~0.83 ms (8,343 calls x ~100 ns). This is NOT the ~2% "BIF dispatch machinery" already
in the dead list — that measured the intercept chain and lookup; this is the whole
call boundary including argument marshalling, which the phase census mis-attributed to
"body" (74.9% of a call).

**H2 (per-op interpreter cost uniformly ~2x worse) — REFUTED.**
Not uniform, and on several ops we are FASTER than Lucee:

| operation (net of empty loop) | RustCFML | Lucee | |
|---|---|---|---|
| empty loop iteration | **43.7** | ~66–70 | we win |
| array read / write | **59.0 / 56.4** | 72.6 / 73.2 | we win |
| struct read / write | 31.6 / 59.9 | 15.8 / 20.2 | 2.0x / 3.0x |
| global (page-scope) read | 58.7 | 23.1 | 2.5x |
| CFC method call | 268 | 99.9 | 2.7x |
| int add / compare | 30.3 / 43.7 | 3.6 / 6.2 | 8.4x / 7.0x, but tiny absolutely |

**H3 (the gap is diffuse) — SURVIVES, and Step 2's reconciliation says something
sharper than "diffuse".**

## ⭐⭐ THE RECONCILIATION FAILS — and that IS the finding

`scripts/perf/reconcile_render.py` prices all 213,966 census-counted ops at their
measured `--no-jit` cost, adds the directly-measured BIF total and the measured frame
costs:

| | ms |
|---|---|
| ordinary bytecode ops (all 196k of them) | 3.25 |
| BIF calls + bodies (8,343, measured directly) | 1.50 |
| CFC method frames (4,432 x 268 ns) | 1.19 |
| other UDF frames (3,761 x 870 ns) | 3.27 |
| **modelled total** | **9.21** |
| measured render CPU | 18.90 |
| **UNACCOUNTED** | **9.69 ms (51%)** |

Implied cost per op in the real render: **88.3 ns**. Microbenchmarked cost over the same
op mix: **16.6 ns**. ⇒ **the same ops cost ~5.3x more inside a real Preside frame than in
a tight loop.** The gap is therefore not in the ops, not in the BIF bodies, and not in
any single call phase — it is in the CONTEXT those ops execute in. The measured
candidates for that context, all already counted per render: 5,425 of 8,216 frame
entries are UNSLOTTED (66%, so name-based lookup not slot indexing), 5,427 struct
allocations, and scope widths far past anything a microbench builds.

**Do not open a track against a microbenchmarked op cost.** Any future lever must be
sized in the real frame context or it will measure the tight-loop number and deliver
the 5.3x-smaller one. This supersedes the old "bodies are fine, data reads are at
parity" assumption AND the "~7 ms is inside bodies" framing at the top of this doc.

## Three findings no hypothesis predicted

1. **`LineInfo` is 10.89% of every op executed** — 23,304 per warm render, pure
   bookkeeping, the 2nd most-executed opcode after `LoadLocal`. Never sized. At the
   cheap-op floor it is only ~0.13 ms, but it is the single largest *count* that buys
   nothing at runtime, and it inflates every op-count-based estimate ever made here.
2. **A page-level UDF call costs 3.2x an unqualified sibling method call** — 870 ns vs
   268 ns for the same trivial callee, identical empty-loop baselines, `--no-jit`,
   reproducible. `this.method()` sits between at 420 ns. Preside is overwhelmingly
   sibling/method calls so it CANNOT see this; it is exactly the procedural-CFML
   workload Part 4 item 6 insists on keeping. Not yet root-caused.
3. **The JIT is 6.5x on a function it admits** (page UDF call 870 → 134 ns) while
   admitting ~0% of Preside. The dead-list entry "Tier-0 ceiling 1.4%" is a statement
   about ADMISSION, not capability. If H2's branch (a) is ever revisited, the question
   to ask is "why is nothing admitted", not "is a JIT worth it".

## Step 0.5 — component instantiation: **3B is GATED OFF**

| | value |
|---|---|
| instances created per BOOT | 5,367 |
| instances created per WARM RENDER | **2** |
| mean data members per instance | **0.6 public + 1.7 private = 2.3** |
| live Preside server RSS | 532 MB |

Instances average 2.3 data members. 3B's headline payoff — "per-instance key storage and
hash tables vanish, ~32 B per declared member" — is therefore 5,367 x 2.3 x 32 B ≈
**395 KB, or 0.07% of RSS**. Its second payoff (instantiation becomes a memcpy) applies
to 5,367 boot instantiations, and boot is already at parity. Its third (warm `variables.x`
becomes a slot index) cannot pay: a warm render creates 2 instances and struct reads are
already at 2.0x with a 16 ns absolute difference. ~~**Part 3B is retired on its own
numbers.**~~ ⚠️ **FINAL (2026-08-22): 3B was un-retired, BUILT (class-keyed IC + op fusion), measured
ZERO on all four real workloads, and re-closed on every axis.** This step's footprint and
instantiation verdicts stand; the read-speed axis it never measured is now measured too.
See the 3B final verdict.

⚠️ **The method the roadmap prescribed for this step does not work.** "RSS delta per
1,000 singletons" measures allocation churn, not retention: a control that CREATES AND
DISCARDS every instance shows the identical slope (2,670 vs 2,693 B/instance over 50,000
instances). Peak RSS cannot size a retained footprint under mimalloc. A real retained
number needs a heap profiler (`--memprofile` / `dhat-heap`), and the gate above does not
depend on one.

## What was actually built (all in-tree, uncommitted)

- **`--features bif-census`** re-added, but as a NAME + ARGUMENT-SHAPE census, not the
  4-phase timer whose verdict is already banked. Per builtin: call count, arity
  histogram, and per-argument-position type + size statistics. Records NO timings
  (`cfml-common/src/perf_counters.rs::bif_census`). ⚠️ It takes a Mutex per builtin
  call: on an instrumented build every BIF measured ~2.4 us and they all looked
  identical. **Never bench on a census build.**
- **`bif_census::report_raw()`** — machine-readable dump. The human report truncates and
  shows CUMULATIVE shares, and boot dominates them: `compareNoCase` is 19% of the
  cumulative table and absent from the warm top 12. Only a diff of two dumps is a
  warm-render mix.
- **Instance counters** (`INSTANCES_CREATED` / `INSTANCE_THIS_KEYS` /
  `INSTANCE_VARS_KEYS`) at `make_instance_value`, unconditional like `STRUCT_NEW`.
- `scripts/perf/bif_census_diff.py` · `gen_bifbench.py` + `bifbench_xengine.py` ·
  `gen_opbench.py` · `reconcile_render.py`.

## Method traps this campaign hit (each cost a wrong number)

1. **Benching on the instrumented binary** — every BIF came out at ~3,100 ns and
   identical, because the census Mutex dwarfed them.
2. **`--production` served a STALE bench** from the bytecode cache after every
   regeneration. Restart the server after editing a bench (the bench dir has its own
   `Application.cfc`, so it does not boot Preside and restart is ~2 s).
3. **A per-iteration closure** to dispatch bench cases added a UDF frame to every
   sample, so a 200 ns BIF became a difference between two ~3,000 ns numbers — and
   leaked the frame-cost gap into the BIF column. Inline the operation.
4. **Lucee's baseline measured first** clamped all 18 of its cases to 0.000: the empty
   loop paid class-load + JIT warm-up and came out slower than every case it was
   subtracted from. Measure the baseline first AND last, take the min.
5. **Our own JIT compiled the microbench callees** while admitting ~0% of Preside. Every
   call-shaped measurement must be taken `--no-jit` to be Preside-representative.
   (Non-call ops were unaffected — identical with and without.)
6. **`invoke( "", ... )` is not portable** — RustCFML raises "Component '' not found".
   Dispatch bench cases by first-class function reference.
7. **`structAppend` returns a boolean on Lucee and the struct on RustCFML** — an
   unrelated engine divergence found by the bench blowing up. Not yet filed.
8. ⚠️ **Running the Lucee arm against the same MySQL made the next RustCFML Preside boot
   fail** on join-table alterations (`saved_export__join__security_user` et al). It
   SELF-HEALS: each boot attempt converges more of the schema, and the third booted
   clean (homepage 5,221 bytes, 30/30 warm renders OK). Budget three boots after any
   cross-engine session, or give the Lucee arm its own database.

# PART 2 — the follow-up campaign — ✅ **COMPLETE.** *The multiplier is NOT what anyone thought* (⚠️ its one survivor, defaults, is 🔴 TO DO — Part 4 item 1)

*Part 1 ended by naming the unslotted frame as the prime suspect for the 5.3× real-frame
multiplier and made sizing it the next action. That was done. **The hypothesis is refuted
by its own measurement**, and so are three others. What survived is a different lever
entirely.*

## The next action, executed: unslotted frames are NOT the multiplier

Method: identical function bodies with an identical runtime op stream, differing only in
`if ( false ) { evaluate( "1" ); }` — the codegen slot-admission scan (compiler.rs
`REFLECTIVE_BUILTINS`) walks the whole instruction list and disqualifies the function on
sight, so the branch costs nothing at runtime while flipping the frame to unslotted.
Verified by the op census: the twin emits `LoadLocal` where the original emits `LoadSlot`.

| | |
|---|---|
| by-name local access vs a slot read | **+33 ns** |
| scaling with locals-map width (1 → 64) | **NONE** — flat 29–34 ns |
| bare-name lookups per warm render | 21,212 |
| .. **hit `locals` on the first probe (slottable)** | **17,681 = 83.4%** |
| .. hit `__variables` (never slottable — no local to slot) | 3,450 = 16.3% |
| **ceiling if every slottable access became a slot** | **0.58 ms/render = 5.1% of the gap** |

**5.1% is the whole prize, and that is a CEILING assuming perfect slotting.** Unslotted
frames cannot be the 9.69 ms. This also retroactively explains why v0.586's
slot-coverage *widening* measured dead: widening captures only a fraction of a 5% prize.

## ⭐⭐ What DID survive: default parameters — ✅ **BUILT AND SHIPPED v0.616.0**

**−3.87% on uncached Preside render traffic, exact permutation p = 0.034**
(8 legs per arm × 900 renders, `scripts/perf/ab_suites.py`). That independently
reproduces the 2026-08-21 prototype's −3.55% / p = 0.018 on a *different* rig, and the
same build shows no regression on the other three real workloads (Preside TestBox
+0.09%, Wheels −1.06%, TestBox −0.16%).

### The mechanism (unchanged — this part of the 2026-08-21 analysis was right)

The defaulted-parameter preamble emitted, per defaulted param:

```
JumpIfArgPresent(n, end); <default expr>; Dup; StoreLocal(n);
LoadLocal("arguments"); Swap; SetProperty(n); StoreLocal("arguments"); [ValidateParamType]
```

`function_needs_arguments_scope` returns true for ANY `LoadLocal("arguments")`, so **one
defaulted parameter forced every call of that function onto the eager `arguments` path,
opting it out of Lever A (lazy `arguments`, −7.7%) whether or not the default ever
fired.** Per warm render: 8,216 bound frames, 4,072 eager (49.6%), **1,740 of those with
a defaulted callee**.

### What shipped

1. **`BytecodeOp::SeedArgumentKey(Name)`** replaces the four-op arguments round-trip in
   ALL THREE default preambles (named functions, closures, arrows). `Dup; StoreLocal(n)`
   is deliberately left alone so slot behaviour is untouched. The op pops the applied
   default and seeds the frame's OWN `arguments` scope; on a lazy frame it is a plain
   pop, which is safe by construction — any body that can observe `arguments` at all
   (by name, `argumentCollection`, an include, a custom tag, or a *string* mentioning
   it) is put back on the eager path by `function_needs_arguments_scope` itself.
2. **The `arguments`-scope ownership invariant** (below) — the actual blocker.

Verified engaged, not just present: with `RUSTCFML_COUNTERS=1` a defaulted callee called
1,001 times now reports **lazy**, and the only frames left eager are the ones that
genuinely read `arguments`.

### ⭐ The blocker was a PRE-EXISTING LUCEE DIVERGENCE, not a hazard the lever created

The 2026-08-21 write-up said the fix "belongs in the writeback diff's `func.params`
guard — work out why it does not already catch this key". **It does catch it.** Both
writeback loops skip `n` correctly; instrumenting them proved the leaked key never
passes through either. The leak was somewhere else entirely, and it was reachable on
v0.615.0 **with no new opcode at all**:

```cfml
function a1( numeric n ){ n = 7; return 1; }     // already lazy today
function a2( numeric n ){ return isNull( n ) ? "absent" : n; }
a1(); a2()                                        // Lucee 7.0.5: "absent". We: 7
```

A frame on the lazy path **inherits the CALLER's `__arguments_scope` through the parent
copy-in**. `CfmlStruct` is an Arc handle, so `StoreLocal`'s param→arguments sync was
writing the callee's own parameter into *the caller's* scope — where the caller could
bare-read it (Lucee throws `variable [N] doesn't exist`) and the next callee inherited
it as its own parameter. The defaulted case was merely *masked*: it was always eager, so
its arguments scope was its own.

**The fix is one line plus its rationale**: the lazy branch of the frame-setup drops any
inherited handle, establishing the invariant **`locals[__arguments_scope]` is always
THIS frame's own scope**. That is what makes every downstream site correct at once
rather than gating them one by one — including the *other* documented blocker:

- **Old cause 1** (`op_jump_if_arg_present` preferring `locals[__arguments_scope]` over
  `arguments_supplied`, so an inherited struct made one function's params look supplied
  to the next) **needed no code at all.** With the inherited key gone the match arm
  falls through to `arguments_supplied` on its own.

Pinned in `tests/functions/test_param_scope_ownership.cfm` (8 assertions), each verified
byte-identical against Lucee 7.0.5 — including the **control that must NOT change**: a
plain unscoped write to a non-param name in a classic-localMode page function still
propagates to the caller (`q = 99` → caller sees 99). That control is the whole reason
the guard has to distinguish a parameter from an ordinary unscoped write.

### Measured cost of a default, for reference (identical call shapes, `--no-jit`)

| | ns |
|---|---|
| declaring a default, arg SUPPLIED (`d1(9)` − `p1(9)`) | **+102** |
| default FIRES vs same callee supplied | +235 |
| default FIRES vs a no-default callee omitted | **+366** → +107 with the fused op |
| declared-type validation (`string`/`numeric`/`struct`) | +22 — cheap, NOT the cost |

### Two traps this hit, both worth keeping

1. **A "blocker" inherited from a previous session's notes is a hypothesis, not a fact.**
   Three source-reading passes went into "why doesn't `func.params` catch this key"
   before a 20-line probe showed the guard was innocent and the shape reproduced on the
   shipped binary. **Reproduce the failure on the CURRENT build before believing an
   account of it** — the cheapest instrument here was four lines of CFML.
2. **The JIT-admission hazard did not apply, and checking cost nothing.** A new opcode
   normally makes both translators decline the whole function (that is what killed 3B's
   op fusion). Here `JumpIfArgPresent` and `ValidateParamType` are absent from
   `jit/translate.rs` and `jit/osr.rs`, so a defaulted function was **never** admissible
   — a new op in that preamble cannot shrink admission. One grep.

## Four hypotheses eliminated — do not re-derive

1. **Unslotted frames** — 5.1% ceiling (above).
2. **Frame cost scaling with `variables` width** — **flat 260–276 ns from 6 to 3,006
   keys** on the component's `variables` scope. A 500× width increase costs nothing.
   ⚠️ This undercuts **3A's premise as a CPU lever**: if copy-in/diff-out were paying per
   parent key, this curve would climb. It does not. (First run of this test seeded every
   width up front and silently measured the same 740-key scope five times — the numbers
   above are from the corrected incremental sweep.)
3. **Unslotted penalty scaling with locals width** — flat 1 → 64.
4. **Memory locality / heap size** — the identical bench, in the identical process,
   before Preside booted (71 MB RSS) and after boot + 8 renders (602 MB RSS):
   **0.99× mean over 18 cases.** Heap size alone is not the multiplier. (Caveat: this
   rules out resident-heap size, NOT working-set size — the bench touches ~5 objects
   where a render touches thousands.)

## Revised reconciliation — still open, but four doors closed

| | ms |
|---|---|
| ordinary bytecode ops | 3.25 |
| BIF calls + bodies | 1.50 |
| frames, now fully decomposed (base + args + types + defaults) | 3.99 |
| **modelled total** | **8.74** |
| measured render CPU | 18.90 |
| **STILL UNACCOUNTED** | **~10.2 ms (54%)** |

**The honest position: roughly half of a warm render's CPU is in something none of these
instruments counts.** It is not op dispatch, not BIF bodies, not frame setup, not
slotting, not scope width, and not heap size. What remains uninstrumented and plausible:
per-frame scope machinery that the op census cannot see (seeding, writeback, the
`arguments` scope), view/template rendering and output buffering, query result
marshalling, and cycle-GC bookkeeping over 5,427 struct allocations per render.
**Instrument one of those next — do not build against the microbenchmarked op costs.**

## Smaller finding, recorded not actioned

`variables.x` costs **1.20× the bare `x`** (86.0 vs 71.5 ns) because it lowers to
`LoadLocal(variables)` + `GetProperty` (5 ops) where the bare name is a single chain-walk
op (4 ops). `is_reserved_scope_name` deliberately excludes scope names from the
`LoadLocalProperty` fusion so the VM keeps its fallback lookups. Preside's house style
prefixes `variables.` everywhere, but a fused op would save ~15 ns/site — instruction
class, which Rule 2 says measures zero. Recorded, not proposed.

# PART 2.5 — ⭐ THE WORKLOAD WAS TOO THIN — ✅ **CAMPAIGN COMPLETE** (2026-08-21, after user challenge)

Every number in Parts 0-3 came from a **bare `http.client` GET of `/`** — no cookie, no
browser headers. That request renders **5,224 bytes and 8,208 frames**. The SAME URL with
an admin session cookie renders **42,736 bytes and 27,296 frames — 3.3x heavier** (Preside
injects its admin edit-mode overlay). Browser headers alone change nothing; the cookie is
the whole difference. The 5 KB render is genuine for this site (scratch Preside, no theme
assets, zero `<script>`/`<link>`) — it is just **thin**.

## ⭐⭐⭐⭐ The cost model — four workloads, uninstrumented, CPU time, footer OFF

| workload | CPU/req | frames | ns/frame | frame-proportional |
|---|---|---|---|---|
| homepage anon (**the campaign baseline**) | 20.0 ms | 8,208 | 2,437 | 68% |
| homepage + admin overlay | 53.0 ms | 27,296 | 1,942 | 85% |
| admin datamanager | 50.0 ms | 26,380 | 1,895 | 87% |
| admin sitetree | 62.0 ms | 34,056 | 1,821 | **90%** |

**CPU/req = 6.77 ms fixed + 1.646 us x frames**, residuals ≤1.3 ms over a 4.1x span.
(Instrumented build: 6.51 + 1.638 — the counter report is not the fixed term.)

## What it changes
1. **The frame path is 90% of a realistic page, not 68%.** The thin baseline understated
   the only thing worth optimising, because its 6.77 ms fixed cost was 34% of the request
   instead of 11%.
2. **Part 1's "5.3x multiplier" is now MEASURED, not inferred from a failed
   reconciliation.** A frame costs **1,646 ns marginal in situ** vs 268 ns (CFC method) /
   870 ns (page UDF) microbenchmarked. Part 1's phase table accounts for only ~593 ns
   (ph4 243 + ph8 156 + ph2 92 + ph20 54 + ph16 48) ⇒ **~1,050 ns/frame unattributed —
   64% of the frame, 58% of a realistic request, at ONE well-defined boundary.**
3. **3A's verdict is unchanged**: 2.8% (thin) → 3.9% (admin) = 69 ns of the 1,646 ns frame.

## Method (all four cost a wrong number first)
- **Playwright to LOG IN, then replay with the cookie.** curl cannot log in. Cookies are
  HttpOnly so `document.cookie` is empty — use `page.context().cookies()`. `psid` and
  `MXP_TRACKINGID` are the two that matter; DB-backed, so they survive a restart and work
  across ports.
- **Debug footer OFF** — it times all 8,432 executed files, inflating the denominator AND
  the frame counts (the footer is rendered CFML). Restore it after; the site runs with it on.
- **CPU time** via `ps -o utime=,stime=` deltas over N requests — admin issues ~10 queries
  and wall would bank DB wait.
- **One admin page view = 2 requests** (main render + AJAX). Bucket on the frame-count
  delta and keep the main one.

---

# PART 2.6 — ⭐⭐⭐ THE ATTRIBUTION IS SOLVED: 99% — ✅ **CAMPAIGN COMPLETE** (⚠️ its "NEXT" is 🔴 TO DO — ledger rank 3)

Part 1's bottom-up reconciliation reached **49%** and stalled for three campaigns. This
reaches **99%**.

```
CPU/request = 5.51 ms fixed + 1,276 ns x FRAMES + 13.9 ns x OPS      (Preside)
            =                   780 ns x FRAMES + 13.9 ns x OPS      (plain CFML)
```

| workload | CPU | model | err | frames% | ops% | fixed% |
|---|---|---|---|---|---|---|
| bench loop (op-heavy, 2 frames) | 10.0 ms | 10.0 | −0.0 | 0% | **100%** | 0% |
| bench calls (frame-heavy, 60k frames) | 61.0 ms | 61.0 | −0.0 | **77%** | 23% | 0% |
| homepage anon (**the old baseline**) | 19.2 ms | 19.0 | +0.2 | 55% | 15% | **29%** |
| homepage + admin overlay | 51.0 ms | 50.1 | +0.9 | 68% | 19% | 11% |
| admin datamanager | 47.0 ms | 48.5 | −1.5 | 72% | 20% | 12% |
| admin sitetree | 61.0 ms | 60.6 | +0.4 | **71%** | 19% | 9% |

## How the coefficients were identified — reuse this
Across the four real workloads **ops/frame is 24.4-26.0, only 6.6% spread** ⇒ frames and
ops are COLLINEAR and no regression on real traffic can separate them. Two synthetic
DESIGN POINTS break it:
- `loop.cfm` (2 frames / 720,063 ops) ⇒ **13.89 ns/op** — corroborates Part 1's 16.6 ns.
- `calls.cfm` (60,002 frames / 1,020,067 ops) ⇒ **780.5 ns/frame** — corroborates Part 1's
  870 ns page-UDF frame.

The four Preside points' residual against those then fits **5.51 ms + 496 ns/frame**
(residuals ≤1.5 ms). ⚠️ Everything `RUSTCFML_JIT=0` — Preside admits ~0% so it is
unaffected, but the JIT compiles the synthetics and destroys the coefficient.

## ⭐⭐⭐⭐ THE CONCLUSIONS
1. **Frames are 71% of a warm admin render; ops are 19%.** Op dispatch is not the lever and
   never was — now proven, not inferred.
2. **A Preside frame costs 1,276 ns vs 780 ns for a synthetic one: a +496 ns surcharge,
   39% of the frame, = 16.9 ms = 28% of a warm admin render.** Biggest addressable item
   found in four campaigns. It is a LOWER bound (the synthetic uses an 870 ns page-UDF
   frame; Preside is mostly 268 ns CFC methods).
3. **"54% unaccounted" is dissolved**: fixed per-request cost (29% of the thin homepage,
   9% of an admin page — the thin workload inflated it) + the per-frame surcharge + op
   cost the census priced but never summed.
4. 3A (0.61 ms) and default params (0.77 ms) are ~1% and ~1.3% of an admin render.

## NEXT: decompose the 496 ns/frame surcharge
What a Preside frame does that a synthetic one does not. Part 1's phase table
(ph4 243 + ph8 156 + ph2 92 + ph20 54 + ph16 48 = 593 ns) was taken on the THIN page —
**re-measure on admin and check whether it sums toward 1,276 or 780.**

## Traps (each produced a wrong table first)
- **An expired session makes every admin URL 302 to login** — 2,839 frames instead of
  34,056, indistinguishable from a cheap workload. **Assert HTTP 200 AND a minimum body
  size per workload.**
- **Warm history is part of the workload**: the anon homepage measured 8,208 frames on a
  server that had served only anon traffic and **2,506** once admin traffic had warmed
  Preside's caches. Warm EVERY workload before measuring ANY, one pass, one server.
- **The Preside login button is inert under Playwright** — `presidecore.min.js` throws
  `l(...).popover is not a function` before wiring the handler. Use
  `HTMLFormElement.prototype.submit.call(document.querySelector('form'))`.
- `op-census` costs <6% CPU here (19.2 vs 20.0 ms), so one pass can collect CPU and counts.

---

# PART 2.7 — INSIDE THE FRAME — ✅ **CAMPAIGN COMPLETE** (⚠️ one lever it named has since SHIPPED; ~30% still dark)

Frames are 71% of a warm admin render (Part 2.6). This asks what a frame spends. All on
the ADMIN workload, `RUSTCFML_JIT=0`, footer off.

## The call prologue is NOT the surcharge
Paired `call-phases` run, BODY excluded (phase 6 recursively contains nested frames and
reads 9,141 ns/frame — summing it double-counts):

| phase | synthetic page-UDF | Preside | delta |
|---|---|---|---|
| **arguments scope + param binding** | 101.9 | **201.2** | **+99.3** |
| CALLER pre-call | 277.7 | 239.8 | −37.9 |
| **parent-scope seed copy** | 413.0 | **74.6** | **−338.4** |
| **CALL MACHINERY TOTAL** | **872.1** | **617.5** | **−254.6** |

**Preside's frames are CHEAPER in call machinery than a plain page UDF.** The phase 3A
rewrites is Preside's best one. Only `arguments + param binding` is worse: **+99 ns/frame
= 3.4 ms = 5.6%**.

## ⛔ Three component-model hypotheses killed
1. **cycle-GC logging (3C)** — `RUSTCFML_NO_CYCLE_GC=1` ABBA on admin: OFF 62.5/61.7 vs
   ON 60.8/60.0 ms ⇒ **zero or negative**, on the workload allocating 15,384 structs/render.
2. **Inheritance depth** — 3-level CFC 2,842 ns vs flat 2,859 ns ⇒ **zero**.
3. **Component shape** — CFC calls are CHEAPER than page UDFs (2,859 vs 3,837).

## What IS real
**BIF boundaries** — ✅ **THIS LEVER HAS SINCE BEEN BUILT AND SPENT (P1, −6.81%).**
33,484 calls/render; `structKeyExists` 9,501 (28.4%), `isNull` 3,419, `len` 3,245,
`isArray` 3,032, `isStruct` 2,209, `isBoolean` 1,635. **62% are type/existence predicates**
= 3.5-5.0 ms (5.8-8.2%). **`isNull` already proves the fix** (codegen lowers it to
`TryLoadLocal`+`IsNull`, 34 ns vs 105-150). Lowering the rest was sized here at
**1.5-2.4 ms** — P1's declarative interception then made it possible to lower ~446
builtins at once, and the live-admin A/B came in at **−4.20 ms / −6.81%**, above this
estimate. **Do not re-propose it.**

**Map-backed access vs slots** (marginal ns/statement): slot local **14.0** · arithmetic
24.7 · `variables.x` 32.2 · `s.k` 35.5 · `obj.a.b.c` 63.5 · strcat 81.7 ⇒ **2.3-4.5x**.
30.7% of ops are map-backed accesses vs 10.2% slotted; converting all ≈ **1.3 ms (2%)**.
⚠️ per-OP delta ~5 ns — do NOT apply the per-STATEMENT delta of 21 ns per op (3x overcount).
**This, not footprint, is 3B's real case** — and 3B was retired on FOOTPRINT, so this ~2%
read-speed case has never actually been ruled out. It is the one dead-list entry with a
live argument against it. See the reconsideration note in Part 3B.

## ⚠️ THE HONEST TOTAL
Accounted: frames x 618 = 21.1 ms (35%) · ops x 13.9 = 11.6 (19%) · fixed 5.5 (9%) ·
BIF boundaries 3.5-5.0 (6-8%) ⇒ **68-71%**. Named levers summed: predicates 1.5-2.4 +
slots 1.3 + args/binding 3.4 + 3A 0.61 + defaults 0.77 = **7.6-8.5 ms = 12-14%**.
**~30% of an admin render is still unattributed.**

**Verdict on Part 3:** directionally right (frames ARE 71%), wrong in its specifics
(3A targets Preside's cheapest phase; 3B's footprint case is 0.07%, its read-speed case
~2%; 3C ablates to zero). Don't claim the dark 30% for a redesign until it is identified.

---

# PART 3 — Rust-native component architecture — ⛔ **CLOSED: 3B built-and-measured-zero (2026-08-22)**; 3A retired except S2/S3 (now P4); 3C dead

*Premise: Lucee/BoxLang are designed around a generational GC (short-lived allocation
nearly free) and free pointer-sharing. We have neither — but we have layout control and
arena/region ownership, which the JVM lacks. Stop paying Rust prices for a Java design.
Modern CFML workloads are component-dominated, so these tracks target components
specifically. Current representations, verified in-tree: `CfmlValue` is 32 B with Arc
handles (clone = refcount bump); interned `Key` (Arc<str> + precomputed CI hash) keys
every `ValueMap`; flyweight `Instance` = `Arc<RwLock<Instance>>` holding
`Arc<ClassBlueprint>` + two `CfmlStruct` data maps + per-instance
`RwLock<HashSet<String>>` accessor set; `StructInner` already carries a `shape_id`.*

## 3A — Frame arena + u32 handles — ⛔ **ARENA RETIRED. S2/S3 survive as P4 — NOT STARTED.**

*(An earlier revision of this heading said "ACTIVE. S2 IN PROGRESS." — that was wrong; no
S-stage code has ever been written. Corrected 2026-08-22.)*

⚠️ **The measurements below are FLOORS, not ceilings.** They were taken on a scratch
Preside with placeholder content — a 60 ms admin render. A production site with real
content and real data volumes is a heavier workload, and these levers scale with it. Do
NOT use the numbers below as a reason to shrink or drop a stage; the user has decided the
direction and removing waste is worth doing on its own merits. (I made exactly that
mistake on 2026-08-21 and was rightly called out for it.)

**Gate met, and it FAILED.** 3A's own text demanded "re-derive the ~2 ms from a
measurement, not from the design difference, before starting." Done — both halves timed
directly on the live Preside warm homepage, one binary, v0.613, `--production`,
`RCFML_WB_COUNTERS=1`, five consecutive warm renders (byte-identical counts):

| 3A half | frames/render | raw | floor | **net** | % of 18.9 ms render |
|---|---|---|---|---|---|
| copy-in (parent-scope seeding) | 8,115 | 0.61 ms | 0.14 | **0.47 ms** | 2.5% |
| diff-out (return-time parent diff) | 1,462 | 0.17 ms | 0.02 | **0.14 ms** | 0.8% |
| **3A TOTAL CEILING** | | | | **0.61 ms** | **3.2%** |

**0.61 ms = 5.4% of the 11.4 ms Lucee gap**, for a multi-week migration across 101
structural read sites with silent failure modes (a write stops propagating, nothing
thrown). **Do not start it.** Sizing history: 3–5 ms claimed → ~2 ms "honest" → **0.61 ms
measured**.

⚠️ **Copy-in has been eroded by other shipped work while 3A sat parked**: 0.88 ms on
2026-08-14, **0.47 ms now**. Interned `Key` (v0.599), the ph4 OnceLocks, ph8, and the
v0.600 futile-writeback skip already took half of it. Any parked structural lever must be
RE-MEASURED before it is costed — its number decays.

**Diff-out futility is spectacular and still not a lever:** 9,980 locals entries scanned
per render to propagate **26 values** (384:1) — and it costs 0.14 ms. That is an argument
about code simplicity, not performance.

### Two instruments ruled out along the way (both recorded so nobody retries them)

1. **Ablation is not viable here.** `RCFML_ABLATE_WRITEBACK=1` (skip the diff at both exit
   paths) reds 14 CFML assertions and **stops Preside booting outright** — `COLDBOX_APP_MAPPING`
   is one of the 26 values the diff propagates. The subsystem is load-bearing and nearly
   free at the same time. Switch has been removed from the tree.
2. **Sampling profiles cannot size anything on this box.** The SIGPROF sampler saw ~157 s
   of ~532 s of process CPU (multi-threaded; the signal lands on whichever thread takes
   it), which is how macOS `stat` at 23% and the mimalloc arena at 27% both turned out to
   be artifacts. **Direct timing at a LOW-COUNT site is the instrument that works** — at
   8,115 and 1,462 samples/render the ~17 ns floor costs 0.16 ms against a 0.6 ms signal,
   whereas per-op timing (213,966 ops) would cost more than the render.

### What survives of the S-stages
S2 (the four structural keys off the map) is still the only piece with a real number
behind it, and the 2026-08-14 analysis already decided its design from data: structural
keys are seeded 16,167x/render and READ 487x (**33:1, 97% never read**) ⇒ the fix is
**LAZINESS, like Lever A** — four known keys, one fallback, no arena and no chain rewrite.
Prize is now **≤0.47 ms (2.5%)**, days not weeks. See
[[project_structural_key_copy_per_frame_lever]]. S1/S3/S4/S5 are retired with the arena.

## 3B — Shape-based (hidden-class) instances — ⛔ **CLOSED 2026-08-22, EVERY AXIS NOW MEASURED**

> **Reconsideration note (2026-08-22).** 3B was retired on FOOTPRINT: instances average
> 2.3 data members ⇒ ~395 KB of a 532 MB server (0.07%), and a warm render creates 2
> instances. That verdict is sound *for footprint*. But **the read-speed case was never
> the basis of the retirement**, and Part 2.7 has since measured map-backed access at
> **2.3–4.5× a slot read** (slot local 14.0 ns · `variables.x` 32.2 · `s.k` 35.5 ·
> `obj.a.b.c` 63.5) across **30.7% of an admin render's ops** ⇒ **~1.3 ms, ~2%**. The
> design doc below told the reader to "sell this on allocation + footprint, NOT read
> CPU" — that instruction was accepted uncritically, and it is what steered the sizing
> at the only number that was going to fail. **This is the one dead-list entry with a
> live argument against it.** Not scheduled — ~2% for a large change — but it is not
> settled either, and if it is ever revived the case is READ SPEED, not memory.
>
> ⭐⭐ **THE PRECEDENT THAT UNDERCUTS THE RETIREMENT (added 2026-08-22).** We have already
> built 3B one scope level up. **Slot-locals (v0.584/585)** is the identical move —
> replace a name-keyed map with a direct-indexed `Vec` of slots — applied to function
> locals instead of component instances. It measured **−8% on the Preside warm homepage**
> (37.5 → 34.0 ms, repeatable, RSS-neutral to ~1 MB). A read-speed model would have
> predicted ~2% for slot-locals too. It delivered **4× that**, because slotting removes
> more than read cost: the map insert on write, the per-frame seeding, and the allocation.
> ⇒ **The method used to size (and retire) 3B undercounts what slotting delivers, and we
> have the counter-example in our own tag history.** Treat 3B's ~2% as a FLOOR with a
> known-bad instrument behind it, not as a verdict. 30.7% of an admin render's ops are
> map-backed vs 10.2% slotted; 3B converts the component-instance share.
>
> ⚠️ Also note 3B is the ONLY item in Part 3 where Rust's layout control is a structural
> advantage the JVM engines cannot match (Lucee = `HashMap` per instance, BoxLang =
> `ConcurrentHashMap`). 3A's premise inverted under measurement — Lucee/BoxLang walk a
> REFERENCE CHAIN and never copy into a frame, so our copy-in/diff-out was never "the Java
> way"; it is a design they do not have, and it measured cheap (0.61 ms).


**Gate: Step 0.5's instantiation sizing — RUN, AND IT FAILED.** Instances average 2.3
data members; the headline payoff is ~395 KB of a 532 MB server (0.07%), and a warm
render creates 2 instances. Full numbers in Part 1 Step 0.5. The design below is kept
only as a record of what was costed and why it was dropped. **Do not revive without a
new workload whose instances are wide.**

### ⛔⛔ FINAL VERDICT (2026-08-22) — 3B is closed by construction-and-measurement, not by argument

Both remaining claims were BUILT and A/B'd on the four real workloads (Preside TestBox
suite, uncached Preside traffic, Wheels core, TestBox own; 8 legs, one binary per A/B via
an env switch, exact permutation test):

| 3B claim | built as | evidence | verdict |
|---|---|---|---|
| member **lookup** cost | class-keyed member inline cache (`method_table` Arc as class id) | **98.6% hit rate on live Preside, 0 stale** — vs 38% for the v0.577 shape_id attempt — and **+0.1…+2.6%, all p≥0.11** (never faster) | mechanism perfect, **prize zero — REVERTED** |
| member access **op count** | fused `LoadVariablesProperty` (in-function `variables.x`, 2 dispatches → 1) | −0.08…+1.34%, **all p≥0.31**, within-arm sd 0.4% ⇒ true effect within ±0.5%. Also **shrinks JIT admission** (translators handle the LoadLocal+GetProperty pair via member-IC shims; the fused op would decline the whole function) | **zero and net-harmful — REVERTED** |

With the earlier measurements this closes every axis 3B ever claimed:
footprint **0.07% of RSS** · cycle-GC ablates to **zero** · instantiation **n=2/warm render**
· lookup **zero** (above) · op dispatch **zero** (above).

**The component read path is not where the time goes. Do not reopen 3B on any of these
axes.** The one-line generalisation, third confirmation this session: *instruction-class
levers measure zero on this engine* (Rule 2) — the IC removed a hash probe, the fusion
removed a dispatch, both invisible at ±0.5% resolution.

**What SURVIVES:** S0 (encapsulation, compiler-enforced `pub(crate)` boundary) and S1 (the
per-class `Shape` + tests) — committed; good structure regardless. A lowering-independent
CFML test pinning in-function `variables.x` semantics incl. a **Lucee-verified parity**
note (extracted method values invoked at page scope throw on BOTH engines — canonical, not
a bug). And the class-identity technique (`method_table` Arc pointer) is proven at 98.6%
if a future lever ever needs per-class caching for something that DOES convert.

**Where the time actually is (unchanged by any of this):** frames — 71% of a warm admin
render, with the +496 ns/frame Preside surcharge (ledger rank 3) and the default-parameter
lever (rank 1, −3.55% measured) as the open items.

### ⭐ 3B IMPLEMENTATION PLAN — SCHEDULED 2026-08-22 (user decision)

**Recon done 2026-08-22. Three of the four pieces 3B needs ALREADY EXIST in the tree —
this is far less greenfield than the design below implies.**

| asset already in tree | where | what it gives 3B |
|---|---|---|
| `Arc<ClassBlueprint>` flyweight (v0.519, default-ON) | `component.rs:51` | the per-class home for the Shape. Already shared by every instance |
| index-based get/set — `get_ci_indexed`, `get_at_index`, `set_at_index` | `dynamic.rs` (v0.99.4–0.100.0) | slot-style access **already implemented**; built for the JIT IC, and the JIT admits ~0% of Preside, so it is dead code on the interpreter path |
| shared-table fall-through on a per-instance map MISS (`method_values` / `set_method_table`) | `component.rs:70`, `dynamic.rs:648` | the exact overflow pattern 3B needs, already proven for methods |
| **slot-locals (v0.584/585, −8%)** | codegen `finalize()` + 16 `*Slot*` ops | the working precedent, AND its hard-won invariant (below) |

**The missing piece is exactly one thing: `shape_id` is not a shape.** `next_shape_id()`
(`dynamic.rs:665`) draws from a GLOBAL counter **per struct instance at construction**, so
two instances of the same class have different `shape_id`s. It is an object identity, not a
hidden class.

### ⭐⭐ This is why the "interpreter inline caches" dead entry does NOT block 3B — it DEPENDS on it
Fable §3.3 was sized and measured dead at v0.577 with this root cause, quoted verbatim from
[[project_fable_33_interpreter_ics_measured_dead]]:

> "`shape_id` … is drawn from a global counter **per struct INSTANCE at construction**, not
> per key-set. It is an object identity, NOT a hidden class. … So a `(shape → index)` IC can
> only hit when a site revisits the *same object*, which in Preside is **38% of reads**. On
> writes it is **1.6%**, because a new-key insert bumps the shape the site just cached."

**3B removes that root cause.** With a real per-class shape, every instance of a class shares
it, and a write to a DECLARED member does not change it at all. The 38% / 1.6% hit rates are
a measurement of the missing shape, not of the technique. ⇒ §3.3 is not dead; it is BLOCKED
ON 3B, and it comes almost free once the shape exists (the index ops are already written).

⚠️ **The one counter-argument that survives, and it must be answered by measurement, not
argument:** §3.3 also found that 94.5% of `GetProperty` reads already resolve on the FIRST
exact-case IndexMap probe — one FxHash lookup, zero allocation — so replacing that with a
shape compare is an *instruction-class* lever, and Rule 2 says those measure zero. **The
rebuttal is that 3B is not an IC**: it removes the map, not the hash. Part 2.7 measured
slot local **14.0 ns** vs `variables.x` **32.2** / `s.k` **35.5** — an 18–21 ns MEASURED gap
per access, plus the per-instance key storage and its construction allocation. That is
allocation-class, which Rule 2 says DOES convert. Slot-locals is the proof: same move, one
scope level up, **−8%** where a read-speed model predicted ~2%.

### Blast radius — measured, not guessed
`this_members` / `variables_members` are touched **122 times**: 59 inside `component.rs`
(the implementation, fine) and **63 outside it** — 30 in `cfml-vm/src/lib.rs`, 24 in
`dynamic.rs`, 4 in `cycle_gc.rs`, 2 in `ops/access.rs`, 2 in `dump.rs`, 1 in `ops/frame.rs`.

✅ **The access VOCABULARY is tiny, so S0 is mechanical, not a 63-way judgement call:** the
only operations used are `clone` (11), `insert` (6), `snapshot` (7), `remove_ci` (4),
`with_write` (2) and `contains_key_ci` (1) — plus **two Instance CONSTRUCTION sites**
(`dynamic.rs:2418` and `:2574`, the `duplicate()`/rebuild paths) which become "build from
the shape" in S2. A large share of the remaining grep hits are comments or LOCAL variables
that merely share the name, not field accesses.

### 🚨 THE INVARIANT, inherited from slot-locals — this is what cost two infinite-loop hunts
**Any code that reads or writes instance members BY NAME cannot see slot storage.** Every
such channel must either be excluded or spill-and-deactivate slots first. Slot-locals found
these the hard way (`spill_slots_for_writeback`); the instance equivalents to audit are the
`this_alias` / `this_instance_alias` `Weak` upgrades, `StructAppend` onto `variables.this`,
cycle-GC's 4 touches, `dump.rs`, and every `duplicate()` / serialization path.

### Stages — S0 first, and S0 is worth shipping on its own
| stage | what | risk | gate |
|---|---|---|---|
| **S0** | **Encapsulate.** Replace the 63 external field touches with methods on `Instance`. Pure refactor, zero behaviour change. Makes every later stage a local edit instead of a 63-site edit — and it is tidy-up, the track that has been out-performing. | low | all suites byte-identical |
| **S1** | Real **`Shape`** on `ClassBlueprint`: `IndexMap<Key, u32>` + a class-level id. ✅ **Its seed already exists** — `ClassBlueprint.properties` (`component.rs:107`) is the declared `property name=…` list **including inherited**, built at class load. Extend it with the `this`/`variables` keys `init` sets. Instances of a class SHARE the shape. | med | suites green; add a counter proving every instance of one class reports ONE shape id |
| **S2** | **Slot storage**: `Vec<CfmlValue>` for shaped members + overflow `CfmlStruct` for dynamic keys. ⚠️ **Overflow is MANDATORY** — WireBox mixins inject data AND UDFs into `variables` at runtime. | high | suites + Wheels/TestBox/Preside byte-identical |
| **S3** | Wire the **interpreter IC** to the now-class-level shape using the EXISTING `get_at_index`/`set_at_index`. Go/no-go = the hit rate moving off 38%/1.6%. ⚠️ **`RCFML_IC_COUNTERS` is NOT in the tree** — verified 2026-08-22; it was carried uncommitted at v0.577 and never landed, so S3 must REBUILD it (`cfml_vm::ic_counters`, env-gated, per-request line on the serve path). Budget for that. | low | counter-first, per Rule 1 |
| **S4** | Codegen-bind `variables.x` in method bodies to a slot index (codegen already builds compile-time `Key`s). | med | JIT tests — ⚠️ a new opcode makes the JIT DECLINE the whole function unless BOTH `jit/translate.rs` and `jit/osr.rs` learn it |

### ⚠️ STAGE ORDERING CORRECTED 2026-08-22 (found while building, after S0+S1 shipped)

**A slot `Vec` the frame scope cannot see would break instance state.** A CFC method frame
is handed the instance's private map DIRECTLY — `lib.rs:28016` does
`method_locals.insert("__variables", CfmlValue::Struct(variables_members))` — and that
sharing is *why* instance mutations persist across method calls. Move declared members into
a parallel slot vector and every `variables.x` in a method body stops seeing them. This is
the slot-locals invariant, restated for instances.

**And storage alone cannot pay anyway.** If the access site has to ask
`shape.slot_of("x")` at runtime, that is a `Key` hash probe — the SAME operation the
IndexMap already does in one probe (94.5% first-try, per §3.3). Swapping a hash for a hash
wins nothing. Slot-locals paid because **codegen** resolved the index at compile time.

**Why compile-time indices are not directly available here:** the shape is built from the
`__properties` array *after* the inheritance merge, which happens at RUNTIME. Codegen
compiles one `.cfc` and cannot know how many inherited properties precede the class's own,
so it cannot emit a constant index. Indices ARE stable per class (parent-first assignment
means a property declared in a parent keeps its index in every subclass — that is what the
`entry().or_insert()` rule guarantees); they are simply known at CLASS-LOAD time, not
file-compile time.

⇒ **Revised S2 is a class-keyed inline cache, not slot storage.** Key the per-site cache on
the blueprint pointer (`Arc::as_ptr`), not on `StructInner::shape_id`. This matters because
the two hit rates are already measured and are nothing alike:

| IC keyed on | measured hit rate | source |
|---|---|---|
| `shape_id` (per-INSTANCE identity) | **38%** reads / **1.6%** writes | Fable §3.3, v0.577 |
| **blueprint pointer (per-CLASS)** | **93%** | Fable §3.4, same run |

§3.4 measured that 93% and then dismissed it because *method lookup* was only 0.08% cum.
Member access is not method lookup — it is the 30.7% of admin-render ops that are
map-backed. Same mechanism, different and much larger population.

⚠️ **Honest expectation, recorded BEFORE building:** this removes a hash probe but NOT the
op count (`variables.x` is still `LoadLocal` + `GetProperty`). That is instruction-class,
and Rule 2 says instruction-class levers measure zero here. **So S2 is now a counter-first
probe, not a committed build** — the deliverable is the hit-rate number and an A/B, and a
zero is a legitimate result that retires the read-speed case honestly (unlike the footprint
retirement, which never measured the right axis). Real slot storage only becomes worth
costing if the IC converts, and it would then need class-load-time index resolution plus a
frame model that stops handing out the raw map.

**Measurement policy (user directive, 2026-08-22):** S0–S2 are structural — do NOT A/B them,
do NOT prove the win first; the bar is "all suites still green". Counter-first applies to S3
only, where a cheap existing counter answers the question.

### Fold in while here
- `accessor_private` is class-invariant in practice ⇒ move the per-instance
  `RwLock<HashSet<String>>` to the blueprint with a lazy per-instance override.
- ⚠️ GH#330's injected-UDF promotion rules (Lucee-verified) must survive the move.

<details><summary>original 3B design, kept as a record (retired)</summary>

*Its own framing said: sell this on allocation + footprint, NOT read CPU. Part 1
re-measured struct reads at 2.0× Lucee (31.6 vs 15.8 ns) rather than the 1.3× quoted
here — still a 16 ns absolute difference, still not a read-CPU lever.*

Today each instance carries two `Arc<PlRwLock<StructInner>>` maps, each with an IndexMap,
shape_id, alias slots and method-table option, plus a per-instance `RwLock<HashSet>` —
several hundred bytes of fixed overhead before a single data member, ×2 cycle-GC log
entries. Preside/WireBox hold thousands of long-lived singletons; it multiplies.

The design (V8's move, Rust-cheap): a per-class **shape** on the `ClassBlueprint`
(interned `Key` → slot index) + a bare `Vec<CfmlValue>` of slots per instance, overflow
map only for dynamic keys. Payoffs: (a) per-instance key storage and hash tables vanish —
~32 B per declared member, which Java engines cannot match (Lucee = HashMap per instance,
BoxLang = ConcurrentHashMap); (b) instantiation becomes "clone the default-slot template"
— memcpy-shaped; (c) `variables.x` in a method body compiles to a slot index (codegen
already builds compile-time `Key`s; `shape_id` and the blueprint exist).

⚠️ CFML dynamism: WireBox mixins inject data AND UDFs into `variables` at runtime, so the
overflow map is mandatory; full V8-style shape *transitions* (same class + same injection
order ⇒ shapes converge) are the stretch goal, static-shape + overflow the pragmatic v1.
⚠️ Injected-UDF promotion rules (GH#330, Lucee-verified) must survive the move.
Micro-item to fold in: `accessor_private` is class-invariant in practice — move to the
blueprint with a lazy per-instance override.

</details>

## 3C — explicitly NOT revived
RwLock-per-struct-read and per-alloc cycle-GC logging were dropped 2026-08-20 as sub-1%
each and structurally invasive. They stay dead. (If 3B ships, instances stop being two
CfmlStructs and both shrink as a side effect — that is the only sanctioned route.)

---

# PART 4 — independent open levers — 🔴 **ALL OPEN, NONE STARTED**

*Small, interleave freely. Renumbered 2026-08-22 and reordered to match THE LEDGER —
the list previously ran `1,2,3,0,1,2,3,4,5,6,7,8` with duplicate numbers, so "item 2"
was ambiguous everywhere it was cited. Items 1–9 below are the original items; 10–12 are
the QwenReview survivors.*

1. ✅ **DONE — shipped v0.616.0.** ⭐⭐ **default parameters — −3.87% on uncached Preside render
   traffic, p=0.034** (and −3.55%/p=0.018 on the earlier prototype rig). `SeedArgumentKey`
   removes the `LoadLocal("arguments")` that opted every defaulted function out of Lever A.
   The blocker turned out to be a **pre-existing Lucee divergence** — a lazy frame
   inheriting the caller's `__arguments_scope` Arc handle — not the writeback guard the
   old notes blamed. Full account, repro and both traps in Part 2.
2. 🔴 **TO DO — rank 2 (prerequisite, not a lever).** **Re-measure the phase table on v0.615 before picking any frame lever.** The last
   table predates the v0.600–v0.613 fixes (ph16 futile-diff skip, ph8 arg_sources defer,
   ph4 OnceLocks, CI-fallback deletion). What plausibly remains: ph4's binding loop
   (genuine work — its alloc fix measured zero), ph8 residue, ph2 seed.
3. 🔴 **TO DO — rank 4.** **Structural-key residue (ph2)** — re-measure post-interned-Key: seeding now clones a
   `Key` (refcount bump), so only the inserts + 38k caller scans remain (was 3.7%, likely
   ~1–2% now). The `[Option<CfmlValue>;4]` design needs 101 read sites touched — only
   worth it as S2 of Part 3A, not standalone. If done: worktree, one key at a time
   (`super` first), `RCFML_FUSED_COUNTERS=1` verifying `struct_keys` falls per key while
   `struct_reads` holds. Full-session change; never start late in one.
4. 🔴 **TO DO — rank 5 (root-cause before sizing).** **⭐NEW — page-level UDF calls cost 3.2× a sibling method call** (870 ns vs 268 ns,
   `--no-jit`, same trivial callee). Not root-caused. Invisible to Preside (which is
   nearly all method calls) and therefore exactly the procedural-CFML lever item 6
   exists to protect. Root-cause it before sizing: the suspicion is that a page-level
   UDF drags closure/captured-scope machinery (the phase 2 seed + phase 16 diff) that a
   CFC method does not.
5. 🔴 **TO DO — rank 6 (clarity, not speed).** **⭐NEW — `LineInfo` is 10.89% of executed ops** (23,304/warm render, 2nd most-executed
   opcode). Pure bookkeeping. At the cheap-op floor only ~0.13 ms, so it is NOT
   obviously a lever — but it has never been sized, it is the largest count buying
   nothing at runtime, and it silently inflates every op-count-based estimate in this
   doc. Counter-first: measure what a `LineInfo` actually costs before proposing to
   thin them out, and check what error reporting/the debug footer lose.
6. 🔴 **TO DO — below the resolution floor; needs its own sub-split first.** **`callee_reflects_on_caller_scope`**: 7 `eq_ignore_ascii_case` on 77% of calls to fire
   on 0.02% — but below the resolution floor; needs its own sub-split before building.
7. 🗄️ **BACKLOG — not scheduled.** T2.4 per-variable closure capture (10–30% on tight HOF loops; medium risk
   vs Lucee write-back semantics).
8. 🔴 **TO DO — rank 7.** **Toolchain (W3), never systematically A/B'd:** BOLT (stacks on PGO) · `-Zbuild-std` ·
   `opt-level`/`target-cpu` sweep. ⛔ `panic="abort"` ruled out on correctness (tokio
   panic isolation needs unwinding, else one panicking request kills the server).
9. 📏 **STANDING RULE, not a lever — applies to every A/B below and above.** ⭐⭐ **Keep a second workload — but NOT Wheels or TestBox for a per-frame lever.**
   Preside cannot see wins for procedural CFML (v0.613's bare-UDF 2.5× win was 0.2%
   on Preside), so a second workload is mandatory. ⚠️ **Measured 2026-08-23: `wheels`
   and `testbox` are the WRONG second workload for anything per-frame.** Their
   frames-per-CPU-second is 9–17× lower than Preside's — Wheels spends **~35 µs of
   CPU per frame** and TestBox **~62 µs** against Preside's **~4 µs**; the rest is
   SQL, ORM and IO. The v0.616 default-params lever reaches **16% of Wheels frames
   and 17% of TestBox frames** vs 21% on Preside — the same reach — yet its predicted
   effect is **0.18% on Wheels** against 2–4% on Preside, purely from dilution. It
   measured **−3.87% on Preside, nothing on Wheels/TestBox, and −22.97% (p=0.002) on
   a frame-dense procedural workload**. ⇒ `scripts/perf/ab_suites.py` now carries a
   **`procfd`** workload (plain page-level UDFs, defaulted params, no DB, no IO) so
   the rule is served by something that can actually SEE a frame lever.
   **Before concluding a per-frame lever "does nothing" on a workload, divide its
   frame count by its CPU seconds.**

### From QwenReview.md triage (2026-08-21) — the three that survived verification

10. 🔴 **TO DO — rank 8 (sweep only).** **String-literal op allocates per execution** (Qwen 4.12, re-derived and verified):
   `BytecodeOp::String(String)` + `ops/value.rs:op_string` does `s.to_string()` +
   `Arc::new` on EVERY execution of every string-literal op, even though
   `CfmlValue::String` is an `Arc<String>` — the op payload could carry a pre-built
   `Arc<String>` and push a refcount bump. Allocation-class (the only class that
   converts), one-op fix (unlike §3.5's BIF-wide ripple, which is why §3.5's zero does
   not automatically kill this). **Now sized from Part 1's census: 7,106 String ops/warm render**, measured at 23.3 ns
   each = ~0.17 ms. Removing the allocation cannot take all of that. Sub-1% — build it
   only as part of a wider allocation sweep, never on its own.
11. 🟡 **DEMOTED — hot-path fix already shipped (v0.565).** **`snapshot()` site census** (Qwen 1.1, DEMOTED from its "critical"): the hot-path fix
   already shipped — `with_map()` landed v0.565 and the −10% is banked; the doc comment
   Qwen quoted is the historical rationale, not a live cost. 129 `.snapshot()` sites
   remain (101 lib.rs / 24 builtins.rs / 4 component.rs) but many are REQUIRED —
   by-value semantics or re-entrancy (⚠️ `with_map` holds the read lock: converting a
   site whose closure runs user code = self-deadlock, memory
   `bug_parking_lot_iflet_read_guard_deadlock`). If touched at all: `#[track_caller]`
   site census behind a feature (the §3.5 technique — note track_caller does NOT
   propagate through closures), convert only hot PURE-read sites.
12. 🟡 **HYGIENE — no perf claim; fix opportunistically.** (Qwen 1.2, demoted): `instance_public_members`'s
   `out.keys().any(eq_ignore_ascii_case)` is O(n×m) where `out.contains_key(name)` is an
   O(1) CI probe (`Key` is case-insensitive by construction); same pattern in
   `all_entries()`/`all_keys()` (`dynamic.rs`). Cold paths (getMetadata output is cached
   per class; for-in over instances is rare) — fix opportunistically, claim nothing.

**Qwen items REJECTED on the record, do not re-import:** compiler/include `to_lowercase`
(compile-time only; the runtime per-call `to_lowercase` removal measured 4 ns) · VFS path
normalization (embedded-FS path, not the serve hot path; existence caching shipped
v0.598; the `stat` share is a profiler artifact) · XML serialization + SmallString (cold
path; SSO strings are measured-dead §3.5) · CfmlValue-size advice (already enforced by a
compile-time assertion) · constant folding / jump threading (instruction-class levers —
164 ns/op is not dispatch-bound) · output-buffer cap "Lucee defaults 512KB for
savecontent" (no such Lucee default — a cap would DIVERGE from the canonical engine;
threat model is trusted app code; the real adjacent gap is `<cfflush>`, which fails
LOUDLY as unimplemented — a compat feature to ask the user about, not a security fix) ·
blanket reject of `__`-prefixed struct writes (legal CFML keys; would diverge from Lucee
— at most add adversarial tests comparing `variables["__variables"]=x` behaviour vs
Lucee) · NativeObject reentrancy deadlock (defused: `CfmlNative::call_method` has no VM
handle so CFML re-entry is impossible; Rust self-calls don't reacquire the lock — a doc
note on the trait at most) · big-bang lib.rs/builtins.rs splits (churn vs pending PRs and
the stale-base problem; the incremental `ops/` extraction is the sanctioned route) ·
path-traversal items (BY DESIGN per user, 2026-08-21).

---

# PART 5 — app-level track — 🔴 **OPEN, NOT STARTED** (largest per-hour wins; **NOT engine work**)

- `isFeatureEnabled()`: 917 calls/admin page (~7.4%), measured 9.6 → 1.5 µs/call when
  memoised. `RequestContextDecorator`: 1,438 hits. Both resolve to a few dozen
  request-invariant answers. Preside-side memoisation beats anything left in the engine's
  file layer.
- **WireBox DI cluster: ~3.9 ms (18%) of a warm request** across ~575 component-method
  calls — full analysis with self-time methodology in `presideWireBoxDIIssue.md` (KEPT).
- `HandlerService.getHandlerBean()` never caches unresolved events and re-scans per call —
  upstream-Preside issues, written up in `PRESIDE_UPSTREAM_HANDLERSERVICE_ISSUES.md` (KEPT).

---

# PART 6 — THE RULES — 📏 **NON-NEGOTIABLE** (each bought with a wasted build or a wrong number)

1. **Counter-first.** Never size a lever from a profile share. Counter or live-page A/B
   BEFORE writing the implementation.
2. **Chase ALLOCATION (count × size), not instructions.** Every lever that ever paid
   removed allocations; a 2.8% instruction lever measured zero; small-key allocs don't
   convert (§3.5: 8.9k removed allocs = −0.05%).
3. **Size on the REAL workload.** A microbench said the BIF intercept chain was 44% of a
   call; live Preside said 6.8%.
4. **CPU time is INVALID cross-engine.** Lucee's process CPU (16.06 ms) exceeds its own
   wall latency (9.3 ms) — JVM GC/JIT threads. Wall/TTFB for RustCFML-vs-Lucee; CPU
   seconds for RustCFML-vs-RustCFML.
5. **Defeat HotSpot.** Accumulate results into a sink and compare sink values, or the
   loop is deleted and a constant-arg BIF measures 0.000 ns on Lucee.
6. **ONE curl process, keep-alive, `%{time_starttransfer}`.** Per-request curl spawn adds
   ~10 ms and fakes an I/O stall. zsh globs `?` in bare URLs — build argv from Python.
   Use `scripts/perf/xengine_ab.py` / `parity_run.sh`.
7. **Debug footer OFF.** The site's `.cfconfig.json` sets `debugging.enabled: true`
   (worth 1.18 ms/render) and CommandBox applies the same file to Lucee — back up, toggle,
   restart BOTH, restore under a `trap`.
8. **Interleaved ABBA, adjacent-pair medians, report the A-to-A spread.** Noise floor
   ~1.9% best, 7–11% on a loud box. A delta smaller than the A-to-A spread is not a
   result. Never build while an A/B runs (a concurrent `cargo check` once WAS the delta).
9. **Instruments:** `call-phases`/`bif-census` have a ~17 ns resolution floor — a phase
   under ~34 ns measures the instrument; sub-phases inflate the parent. Verify a split's
   arithmetic against the wall clock (262 ms of "BIF time" inside a 20 ms render was the
   tell). Prefer env-gated runtime counters (zero perturbation). For any cache, instrument
   the INVALIDATION path — over-invalidation is invisible to correctness tests.
10. **Read the reference engines' source first** (checked out at
    `~/Repos/opensource/CFMLs/{Lucee,BoxLang}`) and **read the switch, not prose about
    it** (a stale comment + stale memory both said the flyweight was off; it's been ON
    since v0.519.0). Last session 4 of 6 reasoned theories were refuted by measurement.

---

# PART 7 — MEASURED DEAD — ⛔ **DO NOT REDO** without NEW evidence (each cost a build)

## ⛔⛔ THREE ENTRIES BELOW WERE WRONG — CORRECTED 2026-08-22
The tidy-up built one of them and measured **−6.81% on the live admin (p=0.0022)**. The
dead-list entries had it at ~1.6-2% and "below noise floor". Retracted:

1. ~~"**Compile-time resolved BIF call sites** (Lucee's trick) — subsumed by interned keys:
   CI lookups 27,589 → 8,052/render, remainder ~1.6% = below noise floor"~~ ⇒ **WRONG,
   off by ~4×.** It scoped the win as "the remaining hash lookups". Compile-time binding
   actually removes the whole **call boundary**: the `LoadGlobal` op, the
   locals→`__variables`→globals chain walk, a per-call `to_lowercase` **heap allocation**,
   and the ~825-line intercept chain. Measured **152 ns saved per call** × 27,687 calls.
2. ~~"**BIF dispatch machinery** ~2% total (body is 74.9% of a BIF call)"~~ ⇒ **WRONG.**
   Same measurement: dispatch was 6.81%, not 2%.
3. ~~"**per-call `to_lowercase` removal** (4 ns)"~~ ⇒ true in isolation, **misleading as a
   verdict** — it was one component of the 152 ns, and killing the component killed the
   appetite for the whole.

**The lesson, and it generalises:** each of these priced a *part* of a boundary and
declared the *whole* dead. **Before writing an entry here, state what the change actually
removes end-to-end** — not the one component you happened to measure.



**Unslotted frames as the real-frame multiplier** (2026-08-21: +33 ns per by-name access,
flat across locals width; 83.4% of 21,212 lookups/render are slottable ⇒ 0.58 ms = 5.1%
CEILING. Also explains why v0.586 coverage-widening measured dead). · **Frame cost scaling
with `variables` width** (flat 260–276 ns from 6 to 3,006 keys — a 500× width increase
costs nothing; this undercuts 3A's premise as a CPU lever). · **Memory locality / resident
heap size** (identical bench, identical process, 71 MB vs 602 MB RSS = **0.99×**; rules
out heap size, not working-set size). · **Declared-type validation as a cost** (+22 ns per
param — cheap; the expensive part of a "typed" signature is its DEFAULTS). ·
**H1 per-BIF body track** (2026-08-21: whole BIF gap 0.97 ms = 8.5% of the request gap,
under the 1 ms bar; the worst ratios are on body-free predicates ⇒ it is a call-boundary
cost, not bodies). · **H2 uniform per-op slowness** (refuted: we BEAT Lucee on the empty
loop 43.7 vs ~66-70 ns and on array read/write; the spread is 0.8×–8×, not uniform). ·
~~**Part 3B shape-based instances**~~ **RETRACTED 2026-08-22 — UN-RETIRED AND SCHEDULED.**
(The footprint finding stands — 2.3 data members ⇒ ~395 KB = 0.07% of RSS, 2 instances per
warm render — but footprint was never 3B's case; READ SPEED is, and it was never measured
here. Same error as the three retractions at the top of this part: priced a part, killed the
whole.) · **"RSS delta per N instances" as a footprint
method** (a discard-everything control shows the identical slope — it measures churn,
not retention). · **JIT (P4/P5):** Tier-0 ceiling 1.4% — 164 ns/op is not dispatch-bound (match dispatch
~1.5 ns); inlinable ops are 39.8% of count, 0.85% of time; ~0% of Preside admitted. The
JIT measured −0.25% on Preside. Only reopen via Part 2's H2 branch, which changes the
claim being tested. · ~~**BIF dispatch machinery** ~2%~~ **RETRACTED — see the top of this
part; it was 6.81%.** · ~~**Compile-time resolved BIF call sites** (Lucee's trick),
"remainder ~1.6% = below noise floor"~~ **RETRACTED — BUILT AS P1 AND MEASURED −6.81%.**
· **More interned-key probe conversion** — census went 72%→85.7% hash-free and CPU stopped tracking it. ·
**Slot-coverage widening** (v0.586) · **D2 args-pool / D1 locals pooling as CPU levers**
(zero under the copy model — retest ONLY at 3A-S5) · **§3.5 as_string/SSO string churn**
(−0.05% ± 0.28%; ceiling ~1%) · **§3.3 interpreter inline caches** · **Lever 1 parent-
scope double clone** (1.14%) · **Lever B scope-view caching** (codegen peepholes already
do it) · **existence probing as CPU** (macOS `stat` share is a profiler artifact, off by
~400×) · **per-function memo Vec hoist** (−0.46% vs 1.94% floor) · ~~**per-call
`to_lowercase` removal** (4 ns)~~ **RETRACTED as a verdict — true in isolation, but it
was one component of P1's 152 ns/call; killing the component killed appetite for the
whole.** · **type-predicate bodies** · **`len()` ASCII fast path**
(`Chars::count()` is already vectorized) · **RwLock-per-struct-read + per-alloc cycle-GC
logging** (sub-1% each, structurally invasive — dropped 2026-08-20) · **DB round-trips**
(identical: 2.1/render both engines) · **macOS thread QoS** (no effect) · **NaN-boxing /
shrinking CfmlValue below 32 B** (pure memory-traffic lever; the class measures zero
here) · **`panic="abort"`** (correctness).

---

# PART 8 — SHIPPED — ✅ **DONE, TAGGED, IN MAIN** (one-liners; detail in project memory)

**v0.613** delete CI-fallback scans the interned key made redundant (bare UDF 523→209 ns)
· **v0.612** arguments-scope memos → compile-time OnceLocks (−3.4% warm) · **v0.611**
base64 decode 7.9×, evaluate() compile cache · **v0.603** PGO retrain on v0.602 ·
**v0.602** arg_sources deferred out of the call prologue (−2.2%, ph24 42→1.6 ns) ·
**v0.601** executed-template metadata cache (−7.9% boot) · **v0.600** futile classic-
localMode writeback diff skipped (−1.7%; diffs 998k→165k) + allocation-free arguments
scope · **v0.599** interned case-insensitive keys (−2.8% + −4.5% more from PGO retrain) ·
**v0.598** cross-request existence cache (probes 16→1/req) · **v0.596** miscased-BIF O(1)
· **v0.595** PGO release builds (~4.8% verified) · **v0.593** live shared application
scope · **v0.590** mimalloc (−15.6% warm / −18.9% boot — the biggest single lever ever;
*swap the subsystem before grinding its call sites*) · **v0.589** P6.1 writeback HashSet
drop (−4%) · **v0.585/584** slot-locals st.1+1.5 (−8%) · **v0.581** MySQL stmt cache ·
**v0.577** InheritedKeys (−4.2%) · **v0.568** regex recompile (−5.7%) · **v0.525**
custom-tag caller rework (69×) · **v0.519** flyweight components default-ON · **v0.512–
517** Lever A lazy `arguments` (−7.7%), Lever C untracked frame-confined scopes.

---

# PART 9 — measurement protocol & process — 📏 **REFERENCE**

**Preside site:** `/Users/alexskinner/Projects/Websites/readyintelligencewebsite/website`
— `rustcfml --serve <path> --port <p> --production`. DB `localhost:3306/pcms_ritest`
(container `busy_shtern`). `website/.env` must have `handlerCaching=true` or you measure
app config. Warm renders need `?cb=N`. Admin = `sysadmin`/`password`; curl CANNOT log in —
use Playwright (`/admin/general/` starts the session).

**Lucee arm:** pre-existing CommandBox server `website2` (**lucee@7 — NEVER `@be`**),
port 8585, same webroot, same MySQL: `box server start name=website2`. Boots in ~7 s.

**Harnesses (in-tree):** `scripts/perf/ab_warm.py` + `ab_admin.py` (interleaved ABBA on
server CPU seconds; SIGINT to stop, never SIGTERM — a SIGTERM'd Preside holds the startup
lock and poisons every later leg) · `scripts/perf/xengine_ab.py`, `parity_run.sh`,
`dbcount.sh` (cross-engine TTFB + DB-round-trip counting; rescued from scratchpad
2026-08-21). ≥3 legs × 150 renders; quote mean, adjacent-pair median, A-to-A spread.

**Instruments:** env-gated, compiled in: `RUSTCFML_COUNTERS=1` (struct allocs, resolve,
exists, BIF lookups, slots, **and as of 2026-08-21: instances created + their data-key
counts · bare-name scope-chain resolution DEPTH (locals / arguments / web scope /
`__variables` / globals / miss) · param-binding SHAPE (bound frames, params declared,
args supplied, type validations)**) · `RCFML_FUSED_COUNTERS=1` (frame seeding). Cargo features, probe builds only:
`op-census` · **`bif-census` (BACK IN TREE 2026-08-21** — per-builtin call counts, arity
histogram and per-argument type/size stats; no timings) · `call-phases` (mind the 17 ns
floor) · `probe-sites` · `alloc-sizing` · `obs-pprof` (`--profile` flamegraph; parsers
`pprof_top.py`, `pprof_callers.py`, `memprof_report.py` at repo root).

🚨 **NEVER measure time on a `bif-census` build** — it takes a Mutex per builtin call and
made every BIF measure ~2.4 µs and look identical. Census builds answer "what runs, with
what arguments"; a separate CLEAN build answers "how long".

**Part 1 harnesses (in-tree, untracked):** `scripts/perf/bif_census_diff.py` (diffs two
per-request census dumps into ONE warm render's BIF + op mix — the raw report is
cumulative and boot-dominated, so the diff IS the measurement) · `gen_bifbench.py` +
`bifbench_xengine.py` (per-BIF cross-engine ratio table) · `gen_opbench.py` (per-operation
cross-engine costs) · `reconcile_render.py` (op counts × op costs vs the measured render) ·
`gen_slotbench.py` (slotted-vs-unslotted A/B — flips slotting with a never-executed
`evaluate()` that the codegen admission scan sees statically; verify the flip with the op
census before trusting a run).
The two generators write into a `bench/` directory in the Preside webroot **with its own
`Application.cfc`** — without that, Preside's Application.cfc swallows the request and
returns 200 with an empty body. Delete `bench/` when done.

**Gate before tagging (ALL green, no skips):** `cargo test --workspace` (⚠️ `-j 4`, debug)
· `cargo run -- tests/runner.cfm` (grep `ERROR|FAIL \|`, never trust SUMMARY) · serve dev
AND `--production`, cold + warm · wasm32 members build · `wasm-pack build crates/wasm
--target web`. For engine-semantics changes add the third-party harnesses, byte-identical
to baseline: TestBox **410/0/0/22** · Wheels **2740/0/0/16** · Preside TestBox
**1556p/17f/50e/2skip** (the bar is identical numbers, not zeros).

**Process (standing user rules):** narrate long gates (announce → report → scoreboard) ·
cap cargo at `-j 4` · NEVER no-op/stub without approval · never park a red test · ask
before ANY `git push`, never chain it · release = bump/verify/commit/tag/push then ASK
about `install-local.sh` · commit fixes direct to main · `.md` commits blocked without
`ALLOW_MD=1` (working docs like this one are never committed at all) · review subagents
need worktree isolation · **real-app verification is the USER's job** — when gates are
green, install the binary and hand over; don't go hunting for apps to boot.
