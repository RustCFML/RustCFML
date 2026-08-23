# Obscura Browser Integration Plan

Incorporate [Obscura](https://obscura.sh) (source: `~/Repos/opensource/THIRDPARTY/obscura`) as an
embedded headless-browser capability: a fluent CFML `Browser()`/Page API in the spreadsheet-builder
style, delivered in **three feature tiers** (`browser` lite/no-V8, `browser-stealth`, `browser-js`
full V8), plus MCP/CDP endpoints served by `rustcfml --serve` in the full tier.

---

## 1. What Obscura is (and why it fits)

Obscura is **not** a Chrome driver — it is a complete headless browser *engine* in Rust:

| Layer | Crate | LOC | Notes |
|---|---|---|---|
| DOM | `obscura-dom` | 2.7k | html5ever + servo selectors, own tree. **Zero V8 references.** |
| Network | `obscura-net` | 3.0k | reqwest client, cookie jar, blocklist, robots; optional `wreq` stealth client (Chrome TLS fingerprint, pulls BoringSSL/cmake, feature-gated upstream). **Zero V8 references.** |
| Page/lifecycle | `obscura-browser` | 2.3k | `Page` (navigate incl. POST, evaluate, settle, interception, preload scripts), lifecycle events. `Page.js: Option<ObscuraJsRuntime>` — only 14 `obscura_js` refs in 2 files. |
| JS | `obscura-js` | 5.9k + 8.8k JS | real V8 via `deno_core 0.350`; `bootstrap.js` provides window/document/fetch/observers/IndexedDB. **The only V8 crate.** |
| Facade | `obscura` | 0.5k | `Browser`/`Page`/`Element`/`CookieStore` embedding API |
| Servers | `obscura-cdp` (6.6k), `obscura-mcp` (2.3k) | | CDP WebSocket server (Puppeteer/Playwright-compatible); MCP server with ~35 agent tools |
| CLI | `obscura-cli` | 2.2k | not needed by us; its `worker.rs` (JSON-over-stdio page actor) validates the actor embedding shape |

Whole workspace ≈ 25.5k lines Rust + 8.8k JS. The core embedding path (dom+net+browser+facade)
≈ **8.5k lines**. All build pain (static-lib download, ~50MB binary growth) is the `v8` crate via
`deno_core` — not Obscura's own code. ~30MB RSS, no Chrome binary, instant startup, Apache-2.0.
Same "pure-Rust, JVM-free replacement for a heavyweight external dependency" story as umya for
spreadsheets — but for Selenium/Puppeteer/headless Chrome.

**Hard constraints verified in source:**

1. **Single V8 isolate per process, single-threaded, `!Send`** (`obscura_js::v8_lock`); everything
   runs on one thread under a current-thread tokio runtime + `LocalSet`
   (`obscura-cli/src/main.rs:291`).
2. The facade `Element` holds a raw `*const Page` pointer (`obscura/src/page.rs:148-151`) — cannot
   leave the owning thread.
3. `Page::evaluate()` swallows JS errors to `Null` (`obscura-browser/src/page.rs:1357`); error
   detail exists one layer down (`evaluate_for_cdp`).
4. **No crates.io releases; git dependency only** (V8 build). Obscura's markdown "feature" is a JS
   snippet run inside V8 (`obscura-js/src/markdown.rs::HTML_TO_MARKDOWN_JS`) — not extractable Rust.
5. `panic = "unwind"` required (anti-panic protocol) — RustCFML already uses the default.

## 2. Sourcing: fork, don't reference upstream

**Fork obscura under the RustCFML org** (Apache-2.0 permits; keep LICENSE/NOTICE), pin by tag.
Rationale: no crates.io releases exist, upstream is fast-moving (10k stars), and the fork carries
our one structural patch:

- Add `js = ["dep:obscura-js"]` cargo feature to `obscura-browser`, gating its ~14 `obscura_js`
  reference sites + 3 `init_js()` call sites. The seams already exist upstream —
  `Page.js: Option<ObscuraJsRuntime>`, a no-JS fallback branch in `evaluate()`, `has_js()` — JS-less
  pages are a supported *runtime* state today; only the compile-time dep is unconditional. Small
  patch, plausibly upstreamable as an obscura "lite" build.

**Fork boundary rule: nothing CFML-specific ever goes in the fork.** Every fork patch must be
upstream-shaped — a change any Obscura user could want, and a candidate PR. The full contemplated
set is exactly three:
1. the `obscura-browser` `js` feature gate above (Phase 0);
2. exposing obscura-mcp's private tool helpers (snapshot ref-tables, extract, detect_forms) as
   public `BrowserState` library functions so cfml-browser can reuse them (§7, Phase 2/3);
3. loud errors instead of silent `Null` from JS-dependent MCP tools on a JS-less page (Phase 5,
   lite-MCP only).

All CFML-specific code lives in this repo: `crates/cfml-browser` (actor thread, command enum,
CfmlValue↔JSON, `CfmlNative` fluent objects), `cfml-stdlib` (BIF registration, stubs, htmd
wiring), `crates/cli` (serve flags). This keeps fork rebases cheap — the diff is mostly
`#[cfg(feature = "js")]` attributes — and each patch that lands upstream shrinks the fork toward a
pure supply-chain pin.

## 3. Feature tiers

| | `browser` (lite) | `browser-stealth` | `browser-js` (full) |
|---|---|---|---|
| Adds | obscura-dom/net/browser (~8.5k lines; html5ever/selectors already in tree via `scraper`; reqwest) | `wreq` stealth client → BoringSSL (cmake) | `obscura-js` → deno_core → **V8** |
| Capabilities | goto incl. redirects + cookie/session persistence, blocklist, DOM + CSS selectors (`with_dom`), `text`/`links`/`extract`/`attr`/`count`, form detection + native POST submit (`navigate_with_wait_post`), `markdown()` via htmd | Chrome TLS fingerprint on navigation and subresources | everything: `evaluate`, SPA/JS-rendered content, `settle`, `click`/`fill`/`press`/`type`/`scroll` (implemented as JS), fetch/XHR interception, preload scripts, **CDP + MCP servers** |
| Default? | **default-on candidate** (spreadsheet-tier dep weight) | opt-in | opt-in |

- Lite still needs a cargo feature (it cannot be unconditional): the wasm exclusion mechanism is
  feature opt-out (`crates/wasm/Cargo.toml` re-adds stdlib features explicitly) and obscura-net's
  reqwest/tokio stack is not wasm-safe. But lite joins `default = [...]` like `http`/`spreadsheet`.
- Methods missing from a build throw the four-part-pattern stub error naming the tier that has the
  capability (e.g. `"click() requires the browser-js build (JavaScript/V8 support)"`). Name set
  identical in every build.
- Lite mode is "cfhttp with a session, a DOM, and (optionally) a stealth TLS stack" — a
  mechanize-class client for static pages. V8 is only for pages that *render* with JS.
- **Markdown uses htmd in ALL builds, never Obscura's V8 converter** (cruder, and output would
  differ between builds). `page.markdown()` serializes the page's *current* DOM → same htmd path as
  the `htmlToMarkdown()` BIF. JS support changes *what DOM you convert* — as-served vs
  script-rendered (V8 mutates the live tree during goto/settle, so this needs no mode switch) —
  not how it converts.

## 4. The crux: bridging RustCFML's sync VM to Obscura's `!Send` async world

RustCFML facts (verified in tree):

- The VM is fully synchronous; builtins are `fn(Vec<CfmlValue>) -> CfmlResult` (`cfml-vm/src/lib.rs:60`).
- Serve mode runs each CFML request on `tokio::task::spawn_blocking` threads (`cli/src/lib.rs:2267`).
- The existing async-crate bridge (s3, `cfml-stdlib/src/s3.rs:21-43`) **does not work here** — it
  requires `Send` futures; every Obscura JS-adjacent type is `!Send` (V8).
- `CfmlNative::call_method` holds the object's `RwLock` write guard for the whole call and has no
  `&mut VM` (`cfml-vm/src/lib.rs:21587-21600`, `async_kernel.rs:18-28`) — re-entrant calls deadlock
  by design; CFML closures cannot be invoked from inside it.

### Design: a dedicated browser service thread (actor model)

```
CFML request thread (sync)                 browser service thread (lazy, one per process)
──────────────────────────                 ────────────────────────────────────────────────
CfmlPage::call_method("goto", args)        tokio current_thread runtime + LocalSet
  └─ build BrowserCmd::Goto{page_id,url}     owns: Browser, HashMap<PageId, Page>,
  └─ tx.send(cmd + oneshot reply)                  per-page network log ring buffers,
  └─ reply.recv_timeout(deadline) ──────►          intercept rule tables
       ◄────────────────────────────────    executes command, replies serde_json/CfmlValue
```

- Spawned lazily on first `Browser()` call (`Lazy<BrowserService>`), `std::thread` with its own
  `new_current_thread().enable_all()` runtime + `LocalSet`. All Obscura types live and die there.
  (In a lite build nothing is `!Send`, but keeping the same actor in both tiers means one
  architecture and no per-tier divergence.)
- CFML-side `NativeObject`s (`CfmlBrowser`, `CfmlPage`) contain only `{id: u64, tx: Sender}` —
  trivially `Send + Sync + Debug`, safe in application scope and across `cfthread`.
- Every command carries a deadline; the CFML side uses `recv_timeout` so a wedged page produces a
  catchable CFML timeout exception, never a hung request thread. Obscura's own watchdogs
  (`arm_watchdog`, `OBSCURA_FETCH_TIMEOUT_MS`) bound the service side.
- One V8 isolate ⇒ concurrent CFML requests **serialize** their browser work through the service
  thread. Fine for testing/scraping; document it. Scale-out later = obscura's worker-subprocess
  pattern (N isolated processes, crash isolation for free).
- `Drop` on the CFML handle sends `ClosePage`/`CloseBrowser` (best-effort, non-blocking), plus a
  service-side idle reaper so leaked handles can't accumulate pages.
- Bind against **`obscura-browser`/`obscura-dom`/`obscura-net` directly** (as `obscura-mcp` does),
  not the `obscura` facade: gets `page.title`, `with_dom()` (selector queries without JS
  round-trips), POST navigation, `evaluate_for_cdp` (real JS error info → CFML exceptions). Skip
  the facade `Element` entirely (raw pointer); element targeting is selector/ref-based, like the
  MCP tools' `resolve_target`.

## 5. CFML API design

Follow the spreadsheet playbook exactly (`cfml-stdlib/src/spreadsheet.rs`):

- `Arc::new_cyclic` + `Weak` self-ref + `fn this(&self) -> CfmlValue` so mutators return the same
  Arc for chaining (spreadsheet.rs:76-107).
- One `CfmlNative::call_method` match with commented bands: mutators (return `this()`), sugar
  (return `this()`), terminals (return data).
- Options are plain structs, case-insensitive keys; 1-based anywhere indexy; unknown method →
  `"Browser has no method [x]"`.

### Entry points

```cfml
// One-shot convenience — the 80% scraping case, no object lifecycle:
res = BrowserFetch("https://example.com", { stealth:true, waitUntil:"networkidle2" });
// → { status, url, title, html, text, markdown, links }

// Fluent:
b = Browser({ stealth:true, proxy:"http://...", userAgent:"...", storageDir:"/tmp/profile" });
```

Procedural surface stays minimal — `Browser()`, `BrowserFetch()`, `IsBrowserObject()` — because
unlike spreadsheets there is no ACF/Lucee legacy BIF family to match. (A `bif!`-macro mirror can be
added later; dispatch is shared either way.)

### Page — fluent chain (mutators return `this`)

```cfml
page = b.newPage()
    .goto("https://news.example.com")           // waitUntil: load|domcontentloaded|networkidle2|networkidle0
    .waitForSelector(".stories", 5000)
    .fill("##q", "rustcfml")                    // browser-js tier
    .press("Enter")                             // browser-js tier
    .waitForText("results", 5000)
    .settle(2000);                              // browser-js tier: pump the JS event loop
```

Mutators/sugar: `goto`, `back`, `forward`, `reload`, `waitForSelector`, `waitForText`, `setCookie`,
`clearCookies`, `block(patterns[])`, `mock(urlPattern, {status, headers, body})`, `submitForm`
(lite-capable native POST), `close`; **browser-js tier**: `click`, `fill`, `fillForm(struct)`,
`type`, `press`, `selectOption`, `scroll`, `settle`, `addPreloadScript`.

Terminals: `url()`, `title()`, `content()`, `text([selector])`, `markdown()` (htmd), `links()`,
`extract(selector)` → array of structs, `attr(selector, name)`, `count(selector)`,
`exists(selector)`, `detectForms()`, `cookies()`, `requests()`/`responses()` (passive network log →
CFML query), `storageState()`/`setStorageState(struct)`; **browser-js tier**: `evaluate(js)`
(JSON → CfmlValue; JS errors throw a `browser`-typed CFML exception via `evaluate_for_cdp`),
`interactiveElements()`, `consoleMessages()`.

Browser object: `newPage()`, `pages()`, `cookies()`, `close()`.

### Interception: declarative rules, not closures (v1)

`onRequest`/`onResponse` CFML closures can't run inside `call_method` (no `&mut VM`) and would
re-enter across threads. v1 ships what covers ~all real use without closures:

- `page.block(["*/ads/*", "*.png"])` — obscura `set_blocked_urls`.
- `page.mock("/api/flags", {status:200, body:'{"x":1}'})` — rule table on the service thread,
  resolved via `enable_interception` + `InterceptResolution::Fulfill` (browser-js tier; JS
  fetch/XHR is what interception intercepts).
- Passive capture always-on into a bounded ring buffer; read back with `page.responses()` — the
  SPA-API-payload path people actually want `on_response` for.

CFML-closure callbacks later, if ever needed, via a VM-intercepted BIF (the `io()` precedent,
`cfml-vm/src/lib.rs:12201`).

## 6. Packaging

- **New crate `crates/cfml-browser`** (the `cfml-qoq` precedent): owns the service thread, command
  enum, and the obscura fork deps; isolates them from `cfml-stdlib`'s dep graph. Its own features:
  `stealth`, `js` — mapped through cfml-stdlib/CLI as `browser`, `browser-stealth`, `browser-js`.
- `cfml-stdlib`: `browser = ["dep:cfml-browser"]`; thin `browser.rs` module with the `CfmlNative`
  impls + BIF registration; four-part twin registrars (stub errors when off; tier-specific stub
  errors inside `call_method` when lite).
- **`browser` (lite) joins default features**; `browser-stealth` and `browser-js` opt-in (BoringSSL
  cmake build and V8 respectively are too heavy to impose on every `cargo build` and the `--build`
  cocktail path).
- wasm: excluded automatically (wasm crate re-adds stdlib features explicitly; don't add `browser`).
- The obscura fork stays a git dep pinned by tag; vendoring the 8.5k-line lite path into the repo
  is the fallback if fork maintenance ever becomes a burden.

## 7. MCP / CDP exposure (`--serve` integration, browser-js tier)

Both servers already exist in obscura as libraries — this is wiring, not building:

- **CDP**: `rustcfml --serve --browser-cdp-port 9222` → `obscura_cdp::start_with_options` on the
  service thread's runtime. Any Puppeteer/Playwright script (or Claude's Playwright MCP plugin)
  drives the same engine — *without Chrome installed*. Pages share the process's one isolate with
  the CFML-facing API.
- **MCP**: `rustcfml --serve --browser-mcp-port 3000` → `obscura_mcp::http::run(...)` (HTTP/SSE,
  `OBSCURA_MCP_ALLOWED_ORIGINS` guard). Agents get ~35 tools (`browser_navigate`,
  `browser_snapshot`, `browser_markdown`, `browser_extract`, `browser_fill_form`, tabs, storage)
  hosted by the CFML server. Loopback-only by default; document the no-auth caveat.
- **Both flags require the `browser-js` build (v1)** and error clearly at startup otherwise.
  CDP hard-requires V8 (obscura-cdp references obscura-js directly, and Puppeteer/Playwright
  implement click/fill/waitForSelector by injecting JS via `Runtime.evaluate`). MCP has zero
  compile-time V8 references, but 18 of 35 tools are `page.evaluate()` snippets and `evaluate()`
  on a JS-less page returns `Null` silently — wrong answers, not errors. A degraded lite-MCP
  profile (~17 DOM/nav/cookie tools) is feasible in Phase 5 only with a fork patch making
  JS-dependent tools error explicitly.
- The MCP tool implementations (snapshot ref-tables, extract, detect_forms) are exactly what the
  CFML terminals need but are **private fns** in obscura-mcp. Upstream a small PR exposing them on
  `BrowserState`, or re-implement in cfml-browser — most are short JS snippets + DOM walks.

## 8. The self-testing angle (worth calling out)

A CFML page can drive a browser against **its own server**: `Browser().newPage()
.goto("http://127.0.0.1:#cgi.server_port#/admin/login")...` — real-JS integration tests written in
pure CFML, in-process, no Selenium/Playwright/Chrome install. This is the browser-test story for
TestBox/Preside webflow suites (done manually with the Playwright MCP plugin until now).
Engine test suite follows the established local-echo pattern (`tests/tags/http_statements_target.cfm`):
serve-mode browser tests navigate to local fixture pages; CLI runs skip via the `cgi.server_port`
guard — no external network dependency.

## 9. Related but separate: `htmlToMarkdown()` BIF

A browser-less `htmlToMarkdown(html [,options])` BIF (for cfhttp-fetched pages) belongs in
cfml-stdlib's existing `html` feature, backed by **htmd** (turndown port; optionally
`dom_smoothie` for `{mode:"article"}` readability extraction). Pure Rust, wasm-safe, independent of
everything above — can ship first. `page.markdown()` routes through the same converter (§3).
**New-builtin approval pending.**

## 10. Phases

0. **Phase 0 — fork + seams**: fork obscura to the RustCFML org, pin a tag; add the
   `obscura-browser` `js` feature patch (+ optional PR upstream); smoke-test lite and full builds.
   Optionally ship `htmlToMarkdown()` (htmd) first — independent, immediate value.
1. **Phase 1 — lite core** (`browser`, default-on): `crates/cfml-browser` (service thread, command
   enum, timeouts, Drop cleanup), `Browser()`/`BrowserFetch()`/`IsBrowserObject()`, Page core
   (goto/back/forward/reload, waitForSelector/Text, content/text/markdown/links/extract/attr/count,
   cookies, detectForms, submitForm), feature gating + stubs, `docs/browser.md` in the
   spreadsheets.md format, serve-mode test suite against local fixtures.
2. **Phase 2 — full tier** (`browser-js`, opt-in): evaluate (via `evaluate_for_cdp`, errors →
   CFML exceptions), click/fill/fillForm/type/press/selectOption/scroll, settle, preload scripts,
   declarative `block`/`mock` interception, passive network log, storage state,
   interactiveElements, consoleMessages. `browser-stealth` pass-through.
3. **Phase 3 — servers**: `--browser-cdp-port` / `--browser-mcp-port` serve flags (browser-js
   only; + security docs); upstream-or-vendor the MCP helper fns.
4. **Phase 4 — self-testing story**: TestBox/Preside webflow recipes, docs, examples.
5. **Phase 5 — scale/nice-to-have**: worker-subprocess pool (crash isolation & parallelism),
   lite-MCP profile (with loud-error fork patch), tabs API polish, CFML closure callbacks via
   VM-intercepted BIFs if demanded.

## 11. Risks / open questions

- **Fork maintenance**: upstream moves fast; pinning + a small patch surface (one feature gate)
  keeps rebases cheap. Vendoring the lite path is the escape hatch.
- **Git-only dependency** blocks crates.io publication of rustcfml crates with `browser` on
  (RustCFML isn't on crates.io today, so acceptable).
- **Build weight (full tier)**: V8 static-lib download on first compile; CI time. Mitigated by
  keeping `browser-js` out of defaults; CI needs a cache for the v8 artifact.
- **Single isolate throughput**: all browser work in one process serializes. Fine for tests and
  modest scraping; worker pool is the escape hatch.
- **Rendering**: Obscura has no layout/paint — **no screenshots/PDF**. Don't promise them in docs.
- **Fidelity**: an independent engine, not Chromium — heavy SPA frameworks mostly work (V8 + fetch
  + observers + IndexedDB shims) but not everything will. Position as scraping/agents/testing, not
  pixel-perfect browsing.
- **Lite-tier `click`**: interactive verbs are JS-implemented; in lite builds they throw. Form
  submission is the lite-capable interaction (native POST). Make the docs table explicit per tier.
- **evaluate error fidelity**: requires binding below the facade (decided, §4) — smoke-test early.
