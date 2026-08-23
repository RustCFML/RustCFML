# Preside WireBox DI cost on a warm request

**Headline:** on a warm Preside front-end request measured at **21.719 ms total**, the
WireBox dependency-injection machinery accounts for **~3.9 ms (18%)** across
**~575 component-method calls** — more than four times the cost of the three
`selectData()` calls in the same request, and the largest single cluster in the
debug footer.

Measured on RustCFML v0.609.0 with the debug footer enabled. Timings are **self
(exclusive)** time — nested calls are subtracted and credited to the parent, so
these figures do not double-count.

> **Read the call counts as exact and the milliseconds as directional.** The
> footer instruments every frame, so the absolute ms carry some observability
> overhead. The counts are the robust signal, and they are what the argument
> below rests on.

---

## 1. Where the 3.9 ms sits

| File | Method | Calls | Self ms |
|---|---|---|---|
| `ioc/Injector.cfc` | `getInstance()` | 69 | 0.981 |
| `ioc/Injector.cfc` | `containsInstance()` | 20 | 0.082 |
| `ioc/Builder.cfc` | `buildSimpleDSL()` | 32 | 0.905 |
| `ioc/DelayedInjector.cfc` | (proxied via `onMissingMethod`) | 70 | 0.700 |
| `config/WireBox.cfc` | `mappingExists()` | 57 | 0.266 |
| `config/WireBox.cfc` | `getMapping()` | 37 | 0.170 |
| `ioc/config/Mapping.cfc` | `getScope()` | 74 | 0.115 |
| `ioc/config/Mapping.cfc` | `isDiscovered()` | 37 | 0.121 |
| `ioc/config/Mapping.cfc` | `getName()` | 37 | 0.079 |
| `ioc/scopes/Singleton.cfc` | `getFromScope()` | 37 | 0.293 |
| `FeatureDependentDsl.cfc` | `process()` | 13 | 0.208 |
| **Total** | | **~483** | **3.92** |

### The counts reconcile exactly, which is why this is trustworthy

```
69 getInstance()  =  37 by-name  +  32 by-DSL
                     ↓                ↓
   getMapping()   =  37          buildSimpleDSL() = 32
   isDiscovered() =  37
   getName()      =  37
   getFromScope() =  37
   getScope()     =  74  ( = 2 × 37, called twice per by-name resolution )
```

Every count falls out of a single number — 37 by-name resolutions and 32
by-DSL resolutions per request. `getScope()` being exactly double confirms the
read of the code path below. This is not a sampling artefact.

---

## 2. Anatomy of one `getInstance("someService")` — for a singleton that already exists

From `system/externals/coldbox/system/ioc/Injector.cfc:307`. Note that **none of
this is construction work** — the singleton was built at boot and is sitting in
the scope cache. This is the cost of *looking it up*:

```cfml
function getInstance( name, dsl, struct initArguments = structNew(), targetObject="" ){
    if( structKeyExists( arguments, "dsl" ) ){ ... }        // by-DSL route, see §3

    if( NOT variables.binder.mappingExists( arguments.name ) ){ ... }   // 1
    var mapping = variables.binder.getMapping( arguments.name );        // 2
    if( NOT mapping.isDiscovered() ){ ... }                            // 3
    if( NOT structKeyExists( variables.scopes, mapping.getScope() ) ){ ... }  // 4
    var target = variables.scopes[ mapping.getScope() ]                // 5 (getScope AGAIN)
        .getFromScope( mapping, arguments.initArguments );             // 6
    variables.eventManager.processState(                               // 7
        "afterInstanceCreation",
        { mapping=mapping, target=target, injector=this }              // 8
    );
    return target;
}
```

So a cache hit costs **six component-method calls, a seventh into the
interceptor service, and a struct literal allocated at the call site** — every
time. Two specific avoidable items:

- **`mapping.getScope()` is called twice** (lines 4 and 5) — once to validate the
  scope key exists, once to index into it. One local variable removes 37 calls
  per request.
- **`processState("afterInstanceCreation")` fires on every lookup**, not just on
  actual creation, and the `{ mapping, target, injector }` struct is built
  *before* the call so it is allocated whether or not anything listens. Preside's
  own `InterceptorService.processState()` override early-returns when no
  interceptor is registered for the state — but the struct has already been
  built, and the method call already made. The state name is also a lie on a
  cache hit: nothing was created.

At the ~0.6–0.8 µs per component-method call this engine measures, that is
roughly **5–7 µs of pure lookup overhead per resolved dependency**.

---

## 3. The `delayedInjector:` proxy

**144 usages across 30 files** in `system/`, the most-injected being
`siteService`, `presideObjectService` and `featureService` (5 each).

### It exists for correctness, not performance — do not just delete it

This matters, and Preside states it itself in
`system/coldboxModifications/services/InterceptorService.cfc:44`:

> "This issue is usually caused by injecting dependencies into your interceptor
> with wirebox and ommitting the `delayedInjector:` DSL from the beginning of
> your inject attributes. For example, in interceptors,
> `[property name="presideObjectService" inject="presideObjectService"]` should
> be `[property name="presideObjectService" inject="delayedInjector:presideObjectService"]`"

A normal `inject` resolves the dependency **during interceptor registration**,
which fires interception points before every interceptor has been registered —
Preside throws `coldbox.interceptor.panic` to catch exactly that. `delayedInjector:`
defers resolution until first use. Any proposal here has to preserve that
deferral.

### The per-call cost is the `onMissingMethod` hop

`DelayedInjector.cfc` **does** memoise the resolved instance, so `Injector.getInstance()`
is only hit once per proxy:

```cfml
public any function get() {
    var instance = _getInstance();
    if ( IsNull( local.instance ) ) {
        instance = _getInjector().getInstance( argumentCollection=_getInjectorArgs() );
        _setInstance( instance );          // memoised — good
    }
    return instance;
}

public any function onMissingMethod( required string missingMethodName, struct missingmethodArguments={} ) {
    var instance = get();
    return instance[ arguments.missingMethodName ]( argumentCollection=arguments.missingMethodArguments );
}
```

But every *call* through the proxy still pays, forever:

1. `onMissingMethod( name, args )` — one frame, plus the `missingMethodArguments` struct
2. `get()` — one frame
3. `_getInstance()` — one frame
4. `instance[ name ]( argumentCollection=... )` — dynamic dispatch by string name

So `siteService.getActiveSiteTemplate()` costs **four frames plus a string-keyed
dynamic dispatch instead of one direct call**, for the entire life of the
process — long after the deferral it exists for has served its purpose. The
footer's 70 `DelayedInjector` calls are these proxy hops (helpfully attributed
under the *requested* method name: `isFeatureEnabled` 17, `getRequestService` 15,
`match` 12, …).

**The deferral is needed once. The indirection is paid every time.**

---

## 4. `featureInjector:` resolves twice, per autowire

**69 usages.** `system/coldboxModifications/FeatureDependentDsl.cfc`:

```cfml
if ( _getInjector().getInstance( "featureService" ).isFeatureEnabled( feature ) ) {
    return _getInjector().getInstance( dsl=service );
}
```

Two `getInstance()` calls per `process()`, and `process()` ran **13 times** in
this request — so ~26 of the 69 `getInstance()` calls come from here alone. The
`getInstance("featureService")` is a fresh full lookup (all six calls from §2)
every time, to fetch the same singleton. `featureService` could be resolved once
and held on the DSL builder, which is itself a singleton.

Worth a separate look: `process()` runs during autowiring, so 13 calls means 13
objects were autowired *during a warm request*. If those are transients being
rebuilt per request, the DI cost above is being paid on the constructor path too.

---

## 5. Suggested changes, cheapest first

| # | Change | Removes / request | Risk |
|---|---|---|---|
| 1 | Hoist the double `mapping.getScope()` in `Injector.getInstance` into a local | 37 calls | None — pure refactor |
| 2 | Cache `featureService` on `FeatureDependentDsl` instead of re-resolving per `process()` | ~13 full lookups (~78 calls) | None — it is a singleton |
| 3 | Only build the `{ mapping, target, injector }` struct and call `processState` when a listener is registered for `afterInstanceCreation` | 37 struct allocs + 37 calls | Low — behaviour identical when unlistened |
| 4 | Have `DelayedInjector` collapse itself after first resolution, so callers hold the real object rather than the proxy | up to 70 × 3 frames | Medium — needs the property rewritten in the target, or a `getTargetObject()` convention at call sites |
| 5 | Memoise `buildSimpleDSL` on the DSL string (32 calls, no memoisation today) | 32 parses | Medium — must not cache transient-scoped results |

1–3 are self-contained and account for roughly **190 of the ~575 calls** with no
semantic change. 4 is the big one but needs design work, because the whole point
of the proxy is that the target is not available when the property is wired.

---

## 6. Caveats

- Single-request sample from one Preside front-end page. Confirm the shape holds
  on the admin and on a second page before acting — though prior profiling has
  found the admin to be the *same shape* at ~4× the frame count, so it is likely
  to amplify rather than change this.
- The engine-side per-call cost (~0.6–0.8 µs per component-method call) was
  measured separately with the debug footer *off*; the footer's own per-frame
  instrumentation means the ms column above is an upper bound.
- Items 1–3 are upstream ColdBox/WireBox code vendored under
  `system/externals/coldbox/`, except where Preside already overrides them in
  `system/coldboxModifications/` — which it does for `Injector`, `Builder` and
  `InterceptorService`, so there is an established place to put all three.
