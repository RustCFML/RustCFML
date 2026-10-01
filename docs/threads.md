# Threading (`cfthread`)

[← Back to README](../README.md)

RustCFML runs `<cfthread>` bodies on **real OS threads** — they execute concurrently
on separate cores, not sequentially inline. This is on by default on native builds.

```cfml
<cfthread name="fetch">
    <cfset thread.body = cfhttp(url = "https://example.com").fileContent>
</cfthread>

<!-- ... do other work in parallel ... -->

<cfthread action="join" name="fetch" timeout="5000"/>
<cfoutput>#cfthread.fetch.body#</cfoutput>
```

## Actions

| Action | Behaviour |
|---|---|
| `run` (default) | Spawns the body on a new thread and returns immediately. |
| `join` | Blocks until the named thread(s) finish. `timeout` is in ms; `0` or omitted waits forever. **Omit `name` to join *all* outstanding threads.** A timeout leaves the thread `RUNNING` and continues without error. |
| `terminate` | Requests cooperative cancellation of the named thread (see caveats). |

The script BIFs **`threadJoin([name][, timeout])`** and **`threadTerminate(name)`**
are equivalent to the `join` / `terminate` actions (e.g. `threadJoin("t", 5000)`,
or `threadJoin()` to join all).

A thread's metadata is available at `cfthread.NAME`, with Lucee's keys: `status`
(`RUNNING`, then `COMPLETED` or `TERMINATED`), `name`, `output`, `elapsedTime` (ms),
`startTime`, `priority` (the tag's `priority`, default `NORMAL`), `interrupted`,
`stackTrace` (always empty: Lucee's is the running thread's Java stack) and
`childThreads`, plus every key the body wrote to its `thread` scope. `error` is
there only when the body threw.

```cfml
<cfthread name="t">
    <cfset thread.answer = 42>
</cfthread>
<cfthread action="join" name="t"/>
<cfoutput>#cfthread.t.status# / #cfthread.t.answer#</cfoutput>   <!-- COMPLETED / 42 -->
```

## What threads share, and what they copy

Each thread runs on its own VM constructed at spawn time. Scopes split into two groups:

| Scope | Visibility |
|---|---|
| `application`, `server`, `session`, `request` | **Shared live** — writes are visible across the parent and all its threads (CFML semantics). Guard concurrent writes with `cflock`. |
| `variables` / `local` | **Copied at spawn** — the thread gets its own copy of the page's `variables`. Re-assigning `variables.x` inside a thread does **not** affect the parent. (Nested objects remain by-reference, as everywhere in CFML — mutating a struct the parent also holds *is* visible.) |
| `attributes` | The custom attributes passed on the `<cfthread>` tag, bound from the parent context at spawn. |
| `thread` | Private to the body; surfaced afterward as `cfthread.NAME.*`. |

```cfml
<cfthread name="greet" who="world" greeting="#session.locale#">
    <!-- attributes.who / attributes.greeting are bound from the parent here -->
    <cfset thread.message = "hello #attributes.who#">
</cfthread>
<cfthread action="join" name="greet"/>
<cfoutput>#cfthread.greet.message#</cfoutput>
```

Pass data **into** a thread via `attributes` (the canonical, portable way) rather than
relying on `variables`, since `variables` is a copy.

## Errors

An error inside a thread body does **not** abort the parent request. The thread's
status becomes `TERMINATED` and `cfthread.NAME.error` holds the error, as the same
struct a `catch` would get (`message`, `type`, `detail`, `tagContext`, …):

```cfml
<cfthread name="risky"><cfthrow message="boom"></cfthread>
<cfthread action="join" name="risky"/>
<cfif structKeyExists( cfthread.risky, "error" )>
    <cfoutput>failed: #cfthread.risky.error.message#</cfoutput>
</cfif>
```

## Caveats — two deliberate differences from Lucee

These follow from doing threading *safely* in Rust; both have simple workarounds.

### 1. `terminate` is cooperative, not forceful

Rust has no memory-safe way to kill a running thread mid-instruction (forcibly stopping
a thread can leave locks held and shared memory half-written — the same reason Java
deprecated `Thread.stop()`). So `terminate` sets a cancel flag that the running body
checks **at loop back-edges** (the top of each loop iteration) and then aborts itself,
ending as `TERMINATED`.

- A thread spinning in a CFML loop stops promptly. ✅
- A thread parked in a **single long-running call with no loop** — `sleep(60000)`, a slow
  query, a big `cfhttp` — won't notice the request until it returns to a loop checkpoint.

It's a difference in *responsiveness*, never correctness. If you need a thread to be
promptly interruptible, give it a loop that does bounded work per iteration rather than
one long blocking call.

### 2. A `cftransaction` cannot span the parent↔child boundary

`<cftransaction>` holds one live database connection, which cannot be safely used from a
different thread than the one that opened it. A spawned thread therefore starts with **no
transaction**: queries it runs are not part of a transaction the parent has open.

```cfml
<cftransaction>
    <cfquery ...>INSERT ...</cfquery>       <!-- parent's transaction -->
    <cfthread name="t">
        <cfquery ...>UPDATE ...</cfquery>   <!-- NOT in the parent's transaction;
                                                 a parent rollback won't undo it -->
    </cfthread>
    <cfthread action="join" name="t"/>
</cftransaction>
```

Keep transactional work inside a single thread (parent **or** child) — don't split a
transaction across a spawn.

## Build notes

- Real threading is gated behind the default-on `real-threads` Cargo feature. Building
  `cfml-vm` with `--no-default-features` (dropping `real-threads`) reverts to the
  synchronous-inline path — the same fallback used on WebAssembly, which has no
  `std::thread`. In that mode `<cfthread>` bodies run immediately and `join`/`terminate`
  are no-ops.
- `cflock` performs real OS-level locking in `--serve` mode (via shared named locks). In
  the toolless CLI with no server context it is a no-op, so prefer per-thread keys over
  shared-counter increments when running threaded code from the CLI.
