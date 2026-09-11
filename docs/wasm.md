# WebAssembly

[← Back to README](../README.md)

RustCFML compiles to WebAssembly via `wasm-bindgen`, so the same engine runs in the browser and on Cloudflare Workers.

## Browser

```bash
cargo install wasm-pack
wasm-pack build crates/wasm --target web
```

```javascript
import init, { CfmlEngine } from './pkg/rustcfml_wasm.js';
await init();
const output = CfmlEngine.new().execute('writeOutput("Hello from WASM!");');
```

The [interactive demo](https://rustcfml.github.io/RustCFML/demo/) is this WASM build running entirely in the browser.

## Cloudflare Workers

RustCFML runs on Cloudflare Workers at the edge. Database access uses [Hyperdrive](https://developers.cloudflare.com/hyperdrive/) bindings (PostgreSQL via `postgres.js`, MySQL via `mysql2`), and application/session state can use KV, R2, or Durable Objects. The Worker host integration lives in a separate repo:

- **[RustCFML-Cloudflare-worker](https://github.com/RustCFML/RustCFML-Cloudflare-worker)**

You deploy from that repo with `wrangler`, which drives `worker-build` to produce the WASM module:

```bash
wrangler deploy
```

The `rustcfml` CLI has **no `--wasm` flag** and takes no part in an edge deploy. See **[Deployment → Cloudflare Workers](deployment.md#cloudflare-workers)** for the surrounding setup.

See **[Database](database.md#postgresql-on-cloudflare-workers)** for how `queryExecute` behaves against Hyperdrive datasources.

## Notes & limits

- The `wasm32-unknown-unknown` target does **not** build the host database drivers (the `postgres`/`mysql` crates use tokio transports that don't compile to wasm) — datasource access on Workers goes through Hyperdrive instead.
- **`<cfhttp>` works on Workers** via the platform `fetch` API, through a JSPI suspending import (`cfml_jspi_http_fetch`). The native client is ureq, which has no `wasm32` target, so `cfml-stdlib` is built here without its `http` feature and `cfml-worker` registers a fetch-backed `cfhttp` builtin instead. The result struct matches the native one key for key — `statusCode`, `status_code`, `statusText`, `fileContent`, `mimeType`, `charset`, `responseHeader` (including `status_code` and `explanation`), `errorDetail`, `HTTP_Version` — so a template that calls an API runs unchanged on a binary and at the edge. All `<cfhttpparam>` types work, including `file`: because a Worker has no filesystem, a file part takes its content from `value=` and uses `file=` only as the filename to present, and the multipart body is assembled in memory. The native client accepts that same form (falling back to reading `file=` as a path when no `value=` is given), so an upload written for the edge also runs on a binary. In the **browser** WASM build `cfhttp` is still absent: that target has no host to supply a transport.
- **S3 is not yet available** in the WASM/Worker build — the AWS SDK uses tokio/hyper transport that doesn't compile on `wasm32-unknown-unknown`. For Workers, use the native R2 binding via the Worker host config. A `fetch()`-backed S3 transport for the WASM target is on the roadmap. See **[Object Storage](s3.md#wasm--cloudflare-workers)**.
