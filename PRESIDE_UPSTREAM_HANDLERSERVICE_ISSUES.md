# `HandlerService.getHandlerBean()` — unresolved events are never cached, and re-scan on every call

**File:** `system/coldboxModifications/services/HandlerService.cfc`
**Version inspected:** Preside-CMS `main` as of 2026-08-13 (line numbers verified against that checkout)

---

## Summary

Two inefficiencies in `getHandlerBean()`:

1. **A lookup that falls through to the invalid-event path is never cached**, so every repeat request
   for the same unresolved event redoes the full handler scan, a `Duplicate()`, and a filesystem
   probe — for the lifetime of the application. This is the significant one.
2. **`arguments.handlers.len()` is re-evaluated on every iteration** of the hottest loop in the
   service (`_getHandlerIndex`), roughly doubling the member-function dispatches in a scan across
   several hundred entries.

Both are one-to-few-line changes.

---

## Issue 1 — the invalid-event path never populates the cache

### The code (`HandlerService.cfc:95-170`)

```cfml
public any function getHandlerBean( required string event ) {                       // :95
    var currentSite = controller.getRequestContext().getSite();                    // :96
    var beanKey     = arguments.event & ( currentSite.id ?: "" );                   // :97

    if ( StructKeyExists( variables.handlerBeans, beanKey ) ) {                     // :99
        return variables.handlerBeans[ beanKey ];                                   // :100
    }

    var handlerBean     = _newHandlerBean( variables.handlersInvocationPath );      // :103
    var handlerReceived = ListLast( ReReplace( arguments.event, "\.[^.]*$", "" ), ":" );
    var methodReceived  = ListLast( arguments.event, "." );
    var isModuleCall    = Find( ":", arguments.event );
    ...
    // Do View Dispatch Check Procedures
    if ( isViewDispatch( arguments.event, handlerBean ) ) {                         // :159
        variables.handlerBeans[ beanKey ] = handlerBean;                            // :160  <-- cached
        return handlerBean;                                                         // :161
    }

    // Run invalid event procedures, handler not found
    invalidEvent( arguments.event, handlerBean );                                   // :165

    // If we get here, then invalid event handler is active and we need to
    // return an event handler bean that matches it
    return getHandlerBean( handlerBean.getFullEvent() );                            // :169  <-- NOT cached
}
```

`variables.handlerBeans` is declared at **`:3`** and is never cleared or bounded.

### The defect

Every *successful* resolution writes the cache — site-template match (`:117`), plain handler match
(`:131`), module match (`:145`), view dispatch (`:160`). The path at **`:165-169`** returns via
recursion **without writing `variables.handlerBeans[ beanKey ]` for the original `beanKey`**.

So for any event that does not resolve to a handler, every call repeats the whole cost:

- `_newHandlerBean()` (`:283`), including a `Duplicate()` (`:288`)
- a `ReReplace`, two `ListLast` and a `Find` (`:103-106`)
- the linear handler scan across every registered mapping (`:111-138`) — see Issue 2
- `ListFindNoCase` / `ListGetAt` over the module handler list (`:139-156`)
- `isViewDispatch()` → `fileExists( expandPath( ... ) )` — a filesystem stat on a path that by
  construction does not exist, since this point is only reached after the scan failed
- `invalidEvent()` (`:204-215`), which throws, plus the recursive call at `:169`

Because `variables.handlerBeans` lives on the singleton, this repeats across requests too, not just
within one.

`Controller._handlerExistsCache` (`system/coldboxModifications/Controller.cfc:30-33`) mitigates this
for callers arriving via `handlerExists()`, but anything calling `getHandlerBean()` directly pays in
full, every time.

### Suggested fix

Cache the resolved bean under the original `beanKey` before returning at `:169`:

```cfml
    invalidEvent( arguments.event, handlerBean );

    var resolved = getHandlerBean( handlerBean.getFullEvent() );
    variables.handlerBeans[ beanKey ] = resolved;
    return resolved;
```

Points that need your judgement rather than ours:

- Is it safe to memoise the invalid-event outcome for the application lifetime? If the invalid-event
  handler is reconfigurable at runtime, this may need to participate in whatever invalidates the
  rest of `handlerBeans`.
- `beanKey` already includes the site id, so the cache is per-site; a negative entry should be too.
- If caching the *bean* is undesirable, storing a negative marker and short-circuiting on it would
  still remove the scan and the stat.

---

## Issue 2 — loop-invariant `.len()` re-evaluated every iteration

### The code (`HandlerService.cfc:274-281`)

```cfml
private numeric function _getHandlerIndex( required array handlers, required string handlerName, required string actionName ) {   // :274
    for( var i=1; i <= arguments.handlers.len(); i++ ){                                                                          // :275
        if ( arguments.handlers[i].name == arguments.handlerName && arguments.handlers[i].actions.findNoCase( arguments.actionName ) ) {  // :276
            return i;
        }
    }
    return 0;
}
```

`arguments.handlers.len()` at **`:275`** is a member-function invocation in the loop condition, so it
is dispatched on every iteration despite being loop-invariant.

### Why it matters here

Core Preside ships **441 handler CFCs** (`find system/handlers -name '*.cfc' | wc -l`), and
`variables.handlerMappings` (`:51`) holds core plus one entry per active extension plus the external
location, with `siteTemplateHandlerMappings` (`:52`) on top. A single unresolved event walks on the
order of several hundred to a couple of thousand array elements, paying a member dispatch per element
purely to re-read a length that cannot change.

Combined with Issue 1, this loop runs in full on every lookup of a missing event.

### Suggested fix

```cfml
    var len = arguments.handlers.len();
    for( var i=1; i <= len; i++ ){
```

No behavioural change; removes roughly half the dispatches in the method.

---

## Lower-confidence observations

Offered as context, not as bug reports — each may be a deliberate trade-off.

**a. Exception-message string matching as control flow.** `getHandler()` (`:172-184`) branches on
`e.message contains "has no accessible Member with name"`. That couples the code to an engine's exact
wording, which varies between engines and can change between versions. A typed check or explicit
existence test would be sturdier.

**b. Not-found signalled by throwing.** `invalidEvent()` (`:204-215`) throws, and
`Controller.handlerExists()` (`Controller.cfc:28-72`) catches it in two `catch` arms to answer a
boolean question. Exception construction including stack capture is not cheap on any engine.
`_handlerExistsCache` mitigates repeats, but the first occurrence per key still pays it — and under
Issue 1 the `getHandlerBean` path never benefits from that cache at all.

**c. WireBox metadata caching is off by default.**
`system/externals/coldbox/system/ioc/config/Binder.cfc:135` defaults `instance.metadataCache = ''`,
so `system/externals/coldbox/system/ioc/config/Mapping.cfc:604` calls
`getComponentMetaData( instance.path )` uncached on every mapping process. If enabling WireBox's
metadata cache is viable for Preside, it would cut the call count rather than relying on each engine
to make that call cheap.

---

## Verifying

No profiler required:

1. Add a counter or log line at `:99` distinguishing cache hit from miss, and another at `:165`.
2. Load an admin page and count how often the same `beanKey` reaches `:165` more than once. Under
   Issue 1, repeats are expected — including across requests, since `variables.handlerBeans` lives on
   the singleton.
3. For Issue 2, hoisting `.len()` at `:275` can be applied and measured directly.
