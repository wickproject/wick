# wick-releases Worker

Cloudflare Worker backing `releases.getwick.dev`. Handles:

- **Release distribution** — signed downloads of prebuilt binaries from R2.
- **Usage telemetry ingest** — two endpoints:
  - `POST /ping` (legacy) — daily usage pings + failure reports, aggregated into KV.
  - `POST /v1/events` — per-fetch telemetry `{host, strategy, ok, status, timing_ms, …}` written to Cloudflare Analytics Engine.
- **Public stats** — `GET /v1/stats/summary` serves shaped rows from Analytics Engine for `https://getwick.dev/stats.html`. Cached 5 min in KV.
- **Legacy analytics dashboard** — `GET /analytics/:key` (KV-based, auth-gated).

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
- `SUBSCRIPTIONS` — KV namespace (also used as the 5-min cache for stats)
- `WICK_EVENTS` — Analytics Engine dataset `wick_events`

Secrets (set via `wrangler secret put`):

- `API_KEYS` — JSON object of Pro customer keys (legacy, kept for existing customers).
- `CF_ANALYTICS_ACCOUNT_ID` — Cloudflare account ID used for the AE SQL API.
- `CF_ANALYTICS_TOKEN` — API token with `Analytics Engine:Read` and `Workers:Read` (or scoped to the `wick_events` dataset).

```bash
echo 'cc1234...' | npx wrangler secret put CF_ANALYTICS_ACCOUNT_ID
echo '<token>'  | npx wrangler secret put CF_ANALYTICS_TOKEN
```

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

Stored in `wick_events` as:

| Column | Meaning |
|---|---|
| `blob1` | host |
| `blob2` | strategy (`cronet`, `cef`, `cef-after-cronet`, `captcha-auto`, …) |
| `blob3` | escalated_from (empty if none) |
| `blob4` | wick version |
| `blob5` | OS |
| `double1` | ok (0 or 1) |
| `double2` | HTTP status |
| `double3` | timing_ms |
| `index1` | host truncated to 32 bytes (used as shard key) |

No IP, no path, no content.

## Querying

```bash
npx wrangler queues   # (unrelated — just to confirm wrangler auth)

# via SQL API (requires CF_ANALYTICS_TOKEN env var):
curl -fsSL https://api.cloudflare.com/client/v4/accounts/$ACCOUNT/analytics_engine/sql \
  -H "Authorization: Bearer $TOKEN" \
  --data-raw "SELECT blob1 AS host, blob2 AS strategy,
              SUM(_sample_interval) AS fetches,
              SUM(double1 * _sample_interval) AS successes
              FROM wick_events
              WHERE timestamp > NOW() - INTERVAL '1' DAY
              GROUP BY host, strategy
              ORDER BY fetches DESC LIMIT 50 FORMAT JSON"
```

See `site/stats.html` for the public version.
