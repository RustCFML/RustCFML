/*
 * cfml-worker JSPI snippet — wasm-bindgen-managed.
 *
 * This file is pulled into wasm-bindgen's output `snippets/` directory and
 * imported by the generated `index_bg.js`. It supplies suspending callbacks
 * the wasm side declares in `src/jspi.rs`.
 *
 * Responsibilities:
 *
 *   1. Export `cfml_jspi_hyperdrive_query` as a `WebAssembly.Suspending` so
 *      calls from wasm can `await` a Hyperdrive query (Postgres or MySQL)
 *      internally and resume sync-from-wasm.
 *
 *   2. Export `cfml_jspi_do_fetch` as a `WebAssembly.Suspending` for
 *      Durable-Object-backed application scope.
 *
 *   2a. Export `cfml_jspi_http_fetch` as a `WebAssembly.Suspending` so
 *      `<cfhttp>` can reach the Workers `fetch` API. The engine's native
 *      HTTP client is ureq, which has no wasm32 target, so on a Worker the
 *      platform's own fetch is the transport.
 *
 *   3. Cache the wasm `memory` object once (handed in by the wasm-bindgen
 *      `start` function) so the suspending callbacks can build Uint8Array
 *      views over linear memory.
 *
 *   4. Expose `globalThis.__cfmlJspi.setEnv` for the host worker entry
 *      point to register the per-request bindings env. Suspending callbacks
 *      look up bindings by datasource name against this env.
 *
 * The host entry point must additionally wrap the wasm-exported fetch with
 * `WebAssembly.promising(...)`; without that wrap, V8 refuses to call a
 * Suspending import. See `cfml-worker/jspi/README.md`.
 *
 * Database drivers (`postgres`, `mysql2`) are dynamically imported on first
 * use so projects that never run a query don't pay the bundling cost.
 */

import postgres from "postgres";
import mysql from "mysql2/promise";

let wasmMemory = null;
let activeEnv = null;

export function __cfml_jspi_set_memory(memory) {
  wasmMemory = memory;
  if (!globalThis.__cfmlJspi) {
    globalThis.__cfmlJspi = {};
  }
  globalThis.__cfmlJspi.setEnv = (env) => {
    activeEnv = env;
  };
  globalThis.__cfmlJspi.clearEnv = () => {
    activeEnv = null;
  };
}

function lookupBinding(env, name) {
  return (
    env[name] || env[name.toUpperCase()] || env[name.toLowerCase()] || null
  );
}

function writeResponse(responseObj, respPtr, respCap) {
  const bytes = new TextEncoder().encode(JSON.stringify(responseObj));
  if (bytes.length > respCap) {
    return -bytes.length;
  }
  new Uint8Array(wasmMemory.buffer, respPtr, bytes.length).set(bytes);
  return bytes.length;
}

/**
 * Postgres path — postgres.js client. One connection per query; Hyperdrive's
 * pooler is the actual pool. We disable prepare to avoid extra round-trips
 * and turn off fetch_types because Workers can't introspect pg_type lazily.
 */
async function runPostgresQuery(connectionString, statements) {
  const sql_client = postgres(connectionString, {
    max: 1,
    fetch_types: false,
    prepare: false,
  });
  try {
    const started = Date.now();
    // Statements are pre-split and rewritten to `$n` placeholders on the Rust
    // side. postgres.js `unsafe()` runs one parameterized command per call, so
    // we run each in order on this single connection and sum affected rows. The
    // last statement's rows are returned as the result set (for SELECTs that is
    // the only statement; for multi-statement mutations they're empty).
    let results = [];
    let rows_affected = 0;
    for (const stmt of statements) {
      const rows = await sql_client.unsafe(stmt.sql, stmt.params || []);
      results = rows.map((r) => ({ ...r }));
      rows_affected += typeof rows.count === "number" ? rows.count : results.length;
    }
    const duration = Date.now() - started;
    return {
      success: true,
      results,
      meta: { duration, rows_affected, last_insert_id: 0 },
    };
  } finally {
    try {
      await sql_client.end({ timeout: 1 });
    } catch (_) {
      /* swallow */
    }
  }
}

/**
 * MySQL path — mysql2/promise. `disableEval: true` is required in Workers
 * because mysql2 otherwise uses `new Function(...)` for row parsing.
 */
async function runMysqlQuery(binding, statements) {
  const connection = await mysql.createConnection({
    host: binding.host,
    user: binding.user,
    password: binding.password,
    database: binding.database,
    port: binding.port,
    disableEval: true,
  });
  try {
    const started = Date.now();
    // MySQL keeps native `?` placeholders and is not split on the Rust side, so
    // `statements` is normally a single entry; loop defensively and keep the
    // last result set / summed affected rows either way.
    let results = [];
    let rows_affected = 0;
    let last_insert_id = 0;
    for (const stmt of statements) {
      const [rows] = await connection.query(stmt.sql, stmt.params || []);
      if (Array.isArray(rows)) {
        // SELECT: rows is RowDataPacket[]; serialize to plain objects.
        results = rows.map((r) => ({ ...r }));
        rows_affected += results.length;
      } else {
        // INSERT/UPDATE/DELETE: rows is ResultSetHeader.
        rows_affected += rows.affectedRows || 0;
        last_insert_id = rows.insertId || last_insert_id;
      }
    }
    const duration = Date.now() - started;
    return {
      success: true,
      results,
      meta: { duration, rows_affected, last_insert_id },
    };
  } finally {
    try {
      await connection.end();
    } catch (_) {
      /* swallow */
    }
  }
}

async function runHyperdriveQuery(req) {
  if (!activeEnv) {
    return {
      success: false,
      error:
        "cfml-jspi: no active env — host did not call globalThis.__cfmlJspi.setEnv(env) before fetch",
    };
  }
  const binding = lookupBinding(activeEnv, req.datasource);
  if (!binding || typeof binding.connectionString !== "string") {
    return {
      success: false,
      error: `cfml-jspi: no Hyperdrive binding named "${req.datasource}" on env`,
    };
  }
  const cs = binding.connectionString;
  const statements = req.statements || [];
  try {
    if (cs.startsWith("postgres://") || cs.startsWith("postgresql://")) {
      return await runPostgresQuery(cs, statements);
    }
    if (cs.startsWith("mysql://") || cs.startsWith("mysql2://")) {
      return await runMysqlQuery(binding, statements);
    }
    return {
      success: false,
      error: `cfml-jspi: Hyperdrive binding "${req.datasource}" has unrecognised connectionString scheme`,
    };
  } catch (e) {
    return { success: false, error: String(e?.message ?? e) };
  }
}

export const cfml_jspi_hyperdrive_query = new WebAssembly.Suspending(
  async (reqPtr, reqLen, respPtr, respCap) => {
    if (!wasmMemory) {
      return 0;
    }
    let request;
    try {
      const bytes = new Uint8Array(wasmMemory.buffer, reqPtr, reqLen);
      request = JSON.parse(new TextDecoder().decode(bytes));
    } catch (e) {
      return writeResponse(
        { success: false, error: `cfml-jspi: bad request JSON: ${e.message}` },
        respPtr,
        respCap,
      );
    }
    const response = await runHyperdriveQuery(request);
    return writeResponse(response, respPtr, respCap);
  },
);

/**
 * Look up a Durable Object Namespace binding by name (case-insensitive),
 * mint or rehydrate the named instance, and fetch a small JSON RPC at
 * `req.path` (defaults to `/`). Wire shape:
 *
 *   { binding, instance, path?, method?, body? }
 *
 * Response:
 *
 *   { success: bool, status: int, body: string, error?: string }
 */
async function runDoFetch(req) {
  if (!activeEnv) {
    return {
      success: false,
      error:
        "cfml-jspi: no active env — host did not call globalThis.__cfmlJspi.setEnv(env) before fetch",
    };
  }
  const binding = lookupBinding(activeEnv, req.binding);
  if (!binding || typeof binding.idFromName !== "function") {
    return {
      success: false,
      error: `cfml-jspi: no Durable Object namespace binding named "${req.binding}" on env`,
    };
  }
  try {
    const id = binding.idFromName(req.instance);
    const stub = binding.get(id);
    const url = `https://do.invalid${req.path || "/"}`;
    const init = { method: req.method || "GET" };
    if (typeof req.body === "string" && req.body.length > 0) {
      init.body = req.body;
      init.method = init.method === "GET" ? "POST" : init.method;
    }
    const resp = await stub.fetch(url, init);
    const body = await resp.text();
    return { success: resp.ok, status: resp.status, body };
  } catch (e) {
    return { success: false, error: String(e?.message ?? e) };
  }
}

/**
 * Dispatch the sync VM activation. The post-build patch installs
 * `globalThis.__cfmlJspi.runSync` as a `WebAssembly.promising` wrapper
 * around the wasm-exported sync runner. This helper awaits that wrapper —
 * the Rust side (`jspi::invoke_run_sync`) imports this function as a
 * normal async JS call, so wasm-bindgen-futures drives the await while a
 * separate contiguous wasm activation runs the VM under JSPI.
 */
export async function __cfml_invoke_run_sync() {
  const fn = globalThis.__cfmlJspi && globalThis.__cfmlJspi.runSync;
  if (typeof fn !== "function") {
    throw new Error(
      "cfml-jspi: globalThis.__cfmlJspi.runSync not installed (build patch missing)",
    );
  }
  await fn();
}

/**
 * `<cfhttp>` transport. Wire shape in:
 *
 *   { url, method, headers?, body?, timeoutMs?, followRedirects?, getAsBinary? }
 *
 * and out:
 *
 *   { success, status, statusText, headers, body, bodyBase64?, error? }
 *
 * A non-2xx is still `success: true` — it is a real HTTP response, and CFML
 * decides what to do with the status via `throwOnError`. Only a transport
 * failure (DNS, TLS, abort, timeout) is `success: false`, which mirrors the
 * native client's split between `Error::Status` and `Error::Transport`.
 */
async function runHttpFetch(req) {
  try {
    const init = {
      method: req.method || "GET",
      headers: req.headers || {},
      // Workers follows redirects by default; `manual` surfaces the 30x itself
      // so `redirect="false"` can see it, as the native client does.
      redirect: req.followRedirects === false ? "manual" : "follow",
    };
    // A text body arrives as `body`; anything that is not valid UTF-8 (a
    // multipart upload carrying binary parts, say) arrives base64-encoded in
    // `bodyBase64`, because the bridge itself is JSON.
    if (typeof req.bodyBase64 === "string" && req.bodyBase64.length > 0) {
      const ascii = atob(req.bodyBase64);
      const bytes = new Uint8Array(ascii.length);
      for (let i = 0; i < ascii.length; i++) {
        bytes[i] = ascii.charCodeAt(i);
      }
      init.body = bytes;
    } else if (typeof req.body === "string" && req.body.length > 0) {
      init.body = req.body;
    }
    if (typeof req.timeoutMs === "number" && req.timeoutMs > 0) {
      init.signal = AbortSignal.timeout(req.timeoutMs);
    }

    const resp = await fetch(req.url, init);

    const headers = {};
    resp.headers.forEach((value, key) => {
      headers[key] = value;
    });

    let body = "";
    let bodyBase64 = null;
    if (req.getAsBinary) {
      const bytes = new Uint8Array(await resp.arrayBuffer());
      // Chunked: String.fromCharCode(...bytes) overflows the argument limit
      // on a body of any size, and a per-byte string concat is O(n^2).
      let ascii = "";
      const CHUNK = 0x8000;
      for (let i = 0; i < bytes.length; i += CHUNK) {
        ascii += String.fromCharCode.apply(
          null,
          bytes.subarray(i, i + CHUNK),
        );
      }
      bodyBase64 = btoa(ascii);
    } else {
      body = await resp.text();
    }

    return {
      success: true,
      status: resp.status,
      statusText: resp.statusText || "",
      headers,
      body,
      bodyBase64,
    };
  } catch (e) {
    // AbortSignal.timeout rejects with a TimeoutError DOMException; give it a
    // message a CFML author can act on rather than a bare "AbortError".
    const name = e && e.name ? e.name : "";
    const msg =
      name === "TimeoutError" || name === "AbortError"
        ? `request timed out after ${req.timeoutMs} ms`
        : String(e?.message ?? e);
    return { success: false, error: msg };
  }
}

export const cfml_jspi_http_fetch = new WebAssembly.Suspending(
  async (reqPtr, reqLen, respPtr, respCap) => {
    if (!wasmMemory) {
      return 0;
    }
    let request;
    try {
      const bytes = new Uint8Array(wasmMemory.buffer, reqPtr, reqLen);
      request = JSON.parse(new TextDecoder().decode(bytes));
    } catch (e) {
      return writeResponse(
        { success: false, error: `cfml-jspi: bad request JSON: ${e.message}` },
        respPtr,
        respCap,
      );
    }
    const response = await runHttpFetch(request);
    return writeResponse(response, respPtr, respCap);
  },
);

export const cfml_jspi_do_fetch = new WebAssembly.Suspending(
  async (reqPtr, reqLen, respPtr, respCap) => {
    if (!wasmMemory) {
      return 0;
    }
    let request;
    try {
      const bytes = new Uint8Array(wasmMemory.buffer, reqPtr, reqLen);
      request = JSON.parse(new TextDecoder().decode(bytes));
    } catch (e) {
      return writeResponse(
        { success: false, error: `cfml-jspi: bad request JSON: ${e.message}` },
        respPtr,
        respCap,
      );
    }
    const response = await runDoFetch(request);
    return writeResponse(response, respPtr, respCap);
  },
);
