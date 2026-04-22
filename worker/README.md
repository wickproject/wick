# wick-releases Worker

Cloudflare Worker backing `releases.getwick.dev`. Handles:

- **Release distribution** — signed downloads of prebuilt binaries from R2.
- **Usage telemetry ingest** — two endpoints:
  - `POST /ping` (legacy) — daily usage pings + failure reports, aggregated into KV.
  - `POST /v1/events` — per-fetch telemetry `{host, strategy, ok, status, timing_ms, …}` stored in KV.
- **Public stats** — `GET /v1/stats/summary` aggregates 7 days of KV-stored events for `https://getwick.dev/stats.html`. Cached 5 min in KV.
- **Legacy analytics dashboard** — `GET /analytics/:key` (KV-based, auth-gated).

Everything here runs on Workers Free — no Analytics Engine, no paid Workers plan required. If Wick grows past the free KV limits, the `/v1/events` handler can be swapped for Analytics Engine by flipping a binding.

## Development

```bash
cd worker
npx wrangler dev            # local preview
npx wrangler tail           # live logs from the deployed worker
```

## Deployment

```bash
npx wrangler deploy
```

### One-time setup

Bindings declared in `wrangler.toml`:

- `RELEASES` — R2 bucket `wick-releases`
- `SUBSCRIPTIONS` — KV namespace (holds Pro keys, legacy ping counters, per-fetch event counters under `evt:` prefix, and the 5-min stats cache)

Secrets (set via `wrangler secret put`):

- `API_KEYS` — JSON object of Pro customer keys (legacy, kept for existing customers).

No additional secrets needed for the telemetry endpoints.

## Telemetry schema

`POST /v1/events` accepts JSON:

```json
{
  "host": "nytimes.com",
  "strategy": "cef",
  "escalated_from": "cronet",
  "ok": true,
  "status": 200,
  "timing_ms": 1840,
  "version": "0.9.2",
  "os": "macos"
}
```

Stored in `SUBSCRIPTIONS` KV as one key per `(date, host, strategy)`:

| | |
|---|---|
| Key | `evt:YYYY-MM-DD:{host}:{strategy}` |
| Value | `{"fetches": N, "successes": M, "total_ms": T}` (JSON) |
| TTL | 30 days |

Increments are read-modify-write, same pattern as the legacy `/ping` counters. Under heavy concurrency a small number of increments may be lost; this is fine for telemetry.

What's **not** stored: URL paths or query strings, request/response bodies, page titles, caller IP (Cloudflare sees it at ingest, but it's never persisted as a data point), user identifier, machine ID.

## Querying

`GET /v1/stats/summary` does the aggregation and returns shaped JSON. See `site/stats.html` for the public renderer.

For ad-hoc debugging you can list KV keys directly:

```bash
npx wrangler kv key list --binding=SUBSCRIPTIONS --prefix='evt:' | head -20
npx wrangler kv key get --binding=SUBSCRIPTIONS 'evt:2026-04-21:example.com:cronet'
```
