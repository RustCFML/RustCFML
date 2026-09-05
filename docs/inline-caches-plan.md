# Inline caches for member reads — plan

Status: **not started, and gated on a pre-flight measurement that may kill it.**
Written when the Cranelift JIT was removed (v0.653.0, known-issues §77), because
member-access inline caches were the one idea worth carrying out of it.

Read this section before writing any code.

---

## 1. This has been built twice and measured zero

Both attempts were interpreter-level member ICs. Both were reverted.

| attempt | keyed by | hit rate on live Preside | A/B result |
|---|---|---|---|
| v0.577 | `shape_id` | 38% | no win |
| 2026-08-22 | class id (`method_table` `Arc` ptr) | **98.6%**, 0 stale | +0.1…+2.6%, all p≥0.11, **never faster** |

The second attempt is the important one: a 98.6% hit rate with zero staleness is
as good as an IC gets, and it still did not produce a win. The reason is not
subtle — **the probe it replaced was already cheap**. `Key` hashes and compares
case-insensitively, so a member read is one `IndexMap` probe; the IC's fast path
needs a second lock acquire to read the cached slot, and that costs about what
the probe saved.

It also sits inside a broader result recorded three separate times in this repo:
**instruction-class levers measure zero on this engine.** Interned-key probes,
the member IC, and `LoadVariablesProperty` op fusion all landed within noise.
Time is in **frames** — 71% of an admin render, ~1,276 ns/frame — not in the ops.

## 2. What changed that could make it different

Exactly one thing: **the JIT is gone.**

The variant never tested is an IC *inside a compiled loop body*, where the
per-iteration frame and dispatch overhead has already been removed, so a saved
probe is a larger share of what remains. That was the MatchBox design (inline
caches in Tier-2 loops) and the reason the idea was worth keeping.

That variant is **no longer available**. With no compiled loops, any IC we build
now is the interpreter IC that has already been measured zero, twice.

So the honest position going in is: **the prior is strongly negative.** This plan
is written so that we spend a day proving or refuting it cheaply, not a fortnight
rebuilding it on hope.

## 3. Pre-flight: the measurement that decides it

Do not build anything until this is done. It is half a day.

1. **Size the ceiling.** Instrument `op_get_property` / `op_get_index` with a
   cycle counter (env-gated, `RCFML_MEMBER_TIMING=1`) and take a live Preside
   admin render. Record total member-read time as a share of request CPU.
2. **Apply the IC's best case.** The 2026-08-22 build hit 98.6% and saved,
   optimistically, one hash probe per hit. Multiply.
3. **Compare against the noise floor of the instrument.** `scripts/perf/ab_suites.py`
   null-calibrates at **3.8–8.8%** on 4 legs — an apparent −3.77% has been
   observed from *identical binaries*.

**Kill criterion: if the modelled best case is below ~4%, stop.** A lever that
cannot clear its own instrument's null floor cannot be validated, and shipping it
means adding a cache-invalidation surface for a number we can never confirm.

Both previous attempts would have been killed here. That is the point.

## 4. If — and only if — it clears the floor

The primitives already exist and are currently unused; they were built for the
JIT's ICs and survived its removal:

- `CfmlStruct::shape_id()` — generation counter, bumped on structural change
- `CfmlStruct::get_ci_indexed()` — case-insensitive lookup returning the entry index
- `CfmlStruct::get_at_index()` / `set_at_index()` — index fast path; `set_at_index`
  deliberately does **not** bump `shape_id` (same key set, new value)

Design constraints learned the hard way:

- **Key the cache by class, not by `shape_id`.** 98.6% vs 38%. Class id = the
  `method_table` `Arc<ValueMap>` pointer, one per class; plain structs have
  `None` and opt out for free.
- **Cache location: a per-instruction `MemberICell` vec on `BytecodeFunction`,
  indexed by ip.** No new opcode, no codegen change. `Name` cannot hold it —
  it is globally interned and shared across call sites.
- ⚠️ **Member access lands on the STRUCT arm, not the Instance arm.** The frame
  holds the private map directly as `__variables`. The first wiring in the 2026-08
  attempt was on the Instance arm and executed **zero times**; only env-gated hit
  counters caught it. A flat A/B would have read as "no win" from dead code and
  we would have drawn the wrong conclusion. **Add hit/miss counters before the
  A/B, always.**
- **One lock, not two.** The second acquire is roughly the probe being saved.

## 5. Where the time actually is

If the pre-flight kills this — the likely outcome — these are the measured,
open levers, all of them in frames rather than ops:

- **The Preside frame surcharge**: +496 ns/frame over synthetic, with allocation
  volume identified as the currency (37.3 allocs/frame vs 17.0).
- **Structural key copying**: 97% of per-frame seeded keys are structural,
  ~16.6k `String` allocations per render.
- **Arity cost**: ~300 ns per declared parameter vs Lucee's ~42 ns (7×), with a
  hard 2.5× cliff at 17 parameters.
- **CFC construction**: ~1.06 µs per method vs Lucee's 0.07 µs (15×).

Each of those is larger than the entire member-read budget this plan is about.

## 6. Rules for whoever picks this up

1. Measure the ceiling before building. If it cannot clear the null floor, close it.
2. Instrument hit/miss counters before running any A/B.
3. Use `scripts/perf/ab_suites.py` — interleaved, CPU seconds, per-leg sanity
   strings, exact permutation p. Never a single wall-clock run.
4. If it measures zero a third time, delete this document rather than leaving it
   open for a fourth attempt.
