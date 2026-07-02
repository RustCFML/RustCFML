# Observability — operations guide (tail sampling)

This is the *ops* companion to [debugging.md](debugging.md) (which covers the
in-engine features: the debug footer, sampling profiler, and OpenTelemetry
export). Here we cover **what runs outside the RustCFML process** to keep
observability cheap and useful in production: an OpenTelemetry Collector doing
**tail sampling**, plus a ready-to-run local stack.

## Why tail sampling

RustCFML's `obs-otel` build does **head sampling** — it decides whether to record
a trace *when the request starts*, before it knows whether the request will be
slow or error. That keeps the hot path cheap, but it means a low sample ratio can
miss the exact traces you care about.

**Tail sampling** makes the keep/drop decision *after* all of a trace's spans have
arrived at a collector — so it can keep a trace **because it was slow or errored**,
which is precisely FusionReactor's "threshold capture" behaviour. Crucially this
happens **off the application host**, so the RustCFML process pays nothing for it.

The recommended split:

- **In the engine (head sampling):** keep the app-side ratio modest but non-zero
  (`observability.otel.sampleRatio`, e.g. `0.05`–`1.0`). Because the sampler is
  `ParentBased`, a trace the collector will want is still fully recorded when the
  app is set to a high enough ratio — set `sampleRatio: 1.0` and let the collector
  do all the filtering if per-request span volume is acceptable.
- **At the collector (tail sampling):** keep everything slow or errored, plus a
  small probabilistic baseline for context.

> Trade-off: tail sampling requires the collector to buffer all spans of a trace
> until the decision window (`decision_wait`) elapses, so it uses memory
> proportional to in-flight trace volume. Size `num_traces` accordingly and watch
> `otelcol_processor_tail_sampling_sampling_trace_dropped_too_early`.

## The reference config

[`examples/observability/otel-collector-config.yaml`](../examples/observability/otel-collector-config.yaml)
ships a `tail_sampling` processor with three policies (OR'd — a trace is kept if
*any* match):

| Policy | Keeps | Why |
|---|---|---|
| `errors` | traces with an ERROR-status span | every failure is interesting |
| `slow` | traces with total latency > 3000 ms | matches the profiler threshold |
| `baseline` | 5% of everything else | context + SLO/rate math |

`decision_wait: 30s` must comfortably exceed your slowest realistic request; a
request still in flight when the window closes is decided on the spans seen so far.

## Run the whole stack locally

[`examples/observability/docker-compose.yml`](../examples/observability/docker-compose.yml)
brings up Collector → Tempo → Grafana in one command:

```bash
# 1. Run RustCFML with OTel on, pointed at the collector:
cargo run --release --features obs-otel -- --serve ./www
#    (.cfconfig.json → observability.otel.enabled: true,
#     endpoint: "http://localhost:4318", sampleRatio: 1.0)

# 2. Bring up the stack:
docker compose -f examples/observability/docker-compose.yml up

# 3. Drive traffic — some fast, some slow (a page that sleeps > 3s), some errored.

# 4. Open Grafana → Explore → Tempo (http://localhost:3000).
#    Only the slow + errored traces (plus ~5% baseline) are retained.
```

Metrics are separate: RustCFML exposes them on `GET /__rustcfml/metrics` for
Prometheus to scrape directly (no collector needed) — see
[debugging.md](debugging.md#opentelemetry-traces--metrics).

## Multi-instance topology

Tail sampling needs **all spans of a trace on the same collector instance** to
make a correct decision. With one collector (the default above) that's automatic.
When you scale to a fleet of collectors, use a **two-tier** deployment:

```
RustCFML instances ──OTLP──▶ tier-1 collectors ──▶ tier-2 collectors ──▶ Tempo
                              (loadbalancingexporter,   (tail_sampling)
                               routes by trace-id)
```

The tier-1 collectors run only a `loadbalancingexporter` keyed by `traceID`, which
guarantees every span of a given trace lands on the same tier-2 collector, where
the `tail_sampling` processor then sees the complete trace. Start with the
single-collector config above and adopt the two-tier layout only when one
collector can't keep up.

## Production notes

- The demo stack has **no auth and ephemeral storage** — it is for local
  development only. Use your managed backend (Grafana Cloud, Tempo/Mimir, Datadog,
  Honeycomb, …) in production; RustCFML just needs an OTLP/HTTP endpoint.
- The debug footer and `/__rustcfml/profiler` / `/__rustcfml/metrics` endpoints
  can leak SQL, file paths and timing — keep them behind your ingress/authn in
  production (the footer already gates on an IP whitelist; the admin endpoints do
  not, so restrict them at the proxy).
