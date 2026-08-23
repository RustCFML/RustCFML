# Testing

[← Back to README](../README.md)

RustCFML's test suite is **written in CFML**, not Rust. The runner includes every test file and uses a small harness providing `assert()`, `assertTrue()`, `assertFalse()`, `assertNull()`, `assertThrows()`, `suiteBegin()`, and `suiteEnd()`.

```bash
cargo run -- tests/runner.cfm    # Full CFML test suite
cargo test                       # Rust unit tests (tag parser, pg_sql, etc.)
```

## Writing a test

1. Create `tests/<category>/test_<feature>.cfm`.
2. Use the harness:

   ```cfml
   suiteBegin("My feature");
   assert("addition", 1 + 1, 2);
   assertTrue("truthy", isNumeric(42));
   assertThrows("bad cast", function() { parseDateTime("not a date"); });
   suiteEnd();
   ```

3. Register it in `tests/runner.cfm`:

   ```cfml
   try { include "category/test_my_feature.cfm"; } catch (any e) { /* ... */ }
   ```

## Lucee is the compatibility reference

**The bar for accepting a contribution is that the test passes on Lucee.** RustCFML targets the CFML standard ([cfdocs.org](https://cfdocs.org)) with Lucee as the primary implementation target, so the same `tests/runner.cfm` is run against Lucee to verify compatibility.

Start Lucee via CommandBox (served from the project root), then hit the runner over HTTP:

```bash
box server start cfengine=lucee@7    # use @7, not @be — bleeding edge can fail to start
curl -s http://127.0.0.1:<port>/tests/runner.cfm -o /tmp/lucee_out.txt
grep -E "^(SUMMARY|FAIL \||ERROR)" /tmp/lucee_out.txt
box server status                    # shows the assigned port
box server stop
```

A green run on **both** RustCFML and Lucee is the compatibility bar. (By rare exception, where Lucee allows something genuinely unreasonable, the project may opt not to match it.)

### Writing tests that pass on both engines

- Do **not** use `var` at page scope — Lucee rejects it. Declare without `var` at page level, or wrap the test body in a function.
- Always close `<cfscript>` blocks with `</cfscript>` — Lucee's parser is strict about this.
- `tests/runner.cfm` includes `harness.cfm` once at the top; individual test files must **not** re-include it (doing so resets the harness counters and masks the grand summary).
- HTTP-dependent tests discover the port from `cgi.server_port` at request time and skip when run from the CLI with no server — don't hardcode a port.
- Don't call a BIF via `argumentCollection` if any argument name has uppercase letters. Lucee uppercases struct keys and its BIF signature lookup is case-sensitive, so the call fails there with "missing required argument" — naming the very argument you passed. Use explicit named arguments.
- Don't leave the application altered for the tests that follow. `application action="update" mappings={...}` **replaces** the application's mapping set, so a test that registers a mapping must merge (read `getApplicationMetadata().mappings`, add, write back) or every later dotted-path test in the run loses `/oop`, `/core` and `/tags`.

### Tests that cannot run on Lucee

A file exercising a RustCFML extension, a superset, or syntax Lucee's parser rejects is marked in the runner:

```cfml
<cf_runtest file="stdlib/test_xmp.cfm" rustcfmlOnly="true">
```

It runs normally on RustCFML and is reported as `SKIP` on any other engine, with a `SKIPPED:` block in the summary listing every one. Nothing is ever skipped on RustCFML itself, so this cannot weaken the release gate.

Reach for it only when the file's *purpose* is RustCFML-only. A file that merely contains one superset assertion should guard that assertion with `isRustCFML()` instead and keep its cross-engine value — unless Lucee rejects it at parse time, in which case an in-file guard is too late and the whole file has to be marked.

Gating exists so a red Lucee run means something. A permanently-red cross-engine run cannot tell you what a change broke.

See the project [CLAUDE.md](../CLAUDE.md) for more detail on the test architecture.
