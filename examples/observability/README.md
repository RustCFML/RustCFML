# RustCFML observability stack (local)

A one-command OpenTelemetry stack for RustCFML: **Collector (tail sampling) →
Tempo (traces) → Grafana (view)**. It keeps only the *interesting* traces — slow
or errored — computed off the app host so the RustCFML process stays cheap.

## Prerequisites

- A RustCFML build with OpenTelemetry compiled in:
  `cargo build --release --features obs-otel`
- Docker + Docker Compose.

## Run

```bash
# 1. Serve an app with OTel enabled, pointed at the collector on :4318.
cargo run --release --features obs-otel -- --serve ./www
```

Your app's `.cfconfig.json`:

```jsonc
{
  "observability": {
    "enabled": true,
    "otel": {
      "enabled": true,
      "endpoint": "http://localhost:4318",
      "sampleRatio": 1.0,
      "metrics": { "enabled": true }
    }
  }
}
```

```bash
# 2. Bring up Collector + Tempo + Grafana.
docker compose up

# 3. Drive some traffic (fast, slow >3s, and errored requests).
# 4. Grafana → Explore → Tempo at http://localhost:3000
```

Metrics scrape endpoint (no collector needed): `http://localhost:8500/__rustcfml/metrics`.

## Files

| File | Purpose |
|---|---|
| `docker-compose.yml` | Collector + Tempo + Grafana |
| `otel-collector-config.yaml` | OTLP receiver + `tail_sampling` (keep slow/errored + 5% baseline) |
| `tempo.yaml` | Minimal single-binary Tempo |
| `grafana-datasources.yaml` | Auto-provisions Tempo in Grafana |

Full write-up, tuning, and the multi-instance topology:
[`docs/observability-ops.md`](../../docs/observability-ops.md).
