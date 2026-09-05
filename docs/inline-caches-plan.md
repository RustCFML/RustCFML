# Inline caches for member reads — plan

Status: **not started. Genuinely open — the two prior attempts were
under-powered, not refutations.**
Written when the Cranelift JIT was removed (v0.653.0, known-issues §77), because
member-access inline caches were the one idea worth carrying out of it.

Read this section before writing any code.

---

## 1. This has been built twice — and the measurements could not resolve the answer

Both attempts were interpreter-level member ICs. Both were reverted.

| attempt | keyed by | hit rate on live Preside | A/B result |
|---|---|---|---|
| v0.577 | `shape_id` | 38% | no win |
| 2026-08-22 | class id (`method_table` `Arc` ptr) | **98.6%**, 0 stale | +0.1…+2.6%, all p≥0.11 |

Those were recorded at the time as "measured zero". **That conclusion was
over-stated, and the numbers say so.** The A/B rig null-calibrates at
**3.8–8.8%** on four legs, and has produced an apparent **−3.77% from identical
binaries**. An instrument with a ±4-9% null floor cannot resolve a candidate
whose plausible effect is 1-3%: "+0.1…+2.6%, p≥0.11" is *no signal*, which is not
the same as *no effect*.

So the honest state is **unknown, not dead**. What the attempts do establish:

- the mechanism works — 98.6% hit, zero staleness, so a third attempt should
  reuse that design rather than rediscover it;
- whatever the effect is, it is **smaller than the instrument's floor**, so the
  first problem to solve is the instrument, not the cache.

There is a real prior for it being small: `Key` hashes and compares
case-insensitively, so the probe an IC replaces is already one `IndexMap`
lookup, and the 2026-08 fast path needed a second lock acquire to read the
cached slot. But "plausibly small" is a hypothesis, and it has never been
measured with an instrument that could see it.

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

## 3. Pre-flight: build the instrument first, then size the ceiling

Do not repeat the previous shape of this work — build, A/B on the suite rig,
read noise, conclude nothing. Two steps, in this order.

### 3a. An instrument that can see a 1% effect

The suite-level A/B rig cannot, and no amount of care in the cache will fix
that. Options, cheapest first:

- **Direct measurement instead of differencing.** Cycle-count
  `op_get_property` / `op_get_index` in place (env-gated, e.g.
  `RCFML_MEMBER_TIMING=1`) and report total member-read cycles per request.
  A before/after on *that* number has none of the null floor of a whole-suite
  wall-clock difference, because it is not a difference of two large numbers.
- **More legs, paired, on a quiet box.** The 3.8–8.8% floor was measured at four
  legs. Establish the floor at the leg count you actually intend to use, and
  publish it alongside the result — a result without its null calibration is
  not a result.
- **Counter-based confirmation.** Hit/miss counters and probe counts prove the
  cache is doing what you think, independent of timing.

The deliverable of 3a is a stated, calibrated resolution: "this rig can detect
X%". Everything downstream is judged against X.

### 3b. Size the ceiling

Take a live Preside admin render with the counter from 3a and record member-read
cycles as a share of request CPU. The IC's best case is that share × the hit rate
× the fraction of a probe it actually saves.

**Decision rule: proceed only if the modelled ceiling is comfortably above the
resolution X established in 3a.** If the ceiling is below what you can measure,
you cannot validate the change — and shipping an unverifiable cache-invalidation
surface is how you get a subtle staleness bug for no proven gain.

That is a different rule from "the ceiling must beat 4%". The old number was an
artefact of a blunt instrument, not a property of the lever.

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

## 5. Where the time is otherwise

If the pre-flight says the ceiling is below what you can resolve, these are the
other open levers. They are all in frames rather than ops, and they are all
*large* — which is why they survived a blunt instrument when a 1-3% member-read
effect could not:

- **The Preside frame surcharge**: +496 ns/frame over synthetic, with allocation
  volume identified as the currency (37.3 allocs/frame vs 17.0).
- **Structural key copying**: 97% of per-frame seeded keys are structural,
  ~16.6k `String` allocations per render.
- **Arity cost**: ~300 ns per declared parameter vs Lucee's ~42 ns (7×), with a
  hard 2.5× cliff at 17 parameters.
- **CFC construction**: ~1.06 µs per method vs Lucee's 0.07 µs (15×).

Each of those is larger than the member-read budget this plan is about, and each
is measurable with the rig as it stands. That is an argument about ORDER, not
about this lever being dead.

## 6. Rules for whoever picks this up

1. Measure the ceiling before building. If it cannot clear the null floor, close it.
2. Instrument hit/miss counters before running any A/B.
3. Never a single wall-clock run. If you use `scripts/perf/ab_suites.py`
   (interleaved, CPU seconds, per-leg sanity strings, exact permutation p),
   calibrate its null floor first at the leg count you are using.
4. Publish the null calibration next to any result. A number without its floor
   is what sent the first two attempts to the wrong conclusion.
5. If a properly-powered measurement says zero, record THAT — with the
   resolution it achieved — and close it for good.
