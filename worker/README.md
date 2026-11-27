# wick-releases Worker

Cloudflare Worker backing `releases.getwick.dev`. Handles:

- **Release distribution** — signed downloads of prebuilt binaries from R2.
- **Usage telemetry ingest** — two endpoints:
  - `POST /ping` (legacy) — daily usage pings + failure reports, aggregated into KV.
  - `POST /v1/events` — per-fetch telemetry `{host, strategy, ok, status, timing_ms, …}` stored in KV.
  - `POST /v1/feedback` — capability requests agents send through the `wick_request_capability` MCP tool, one KV record each.
- **Public stats** — `GET /v1/stats/summary` aggregates 7 days of KV-stored events. The renderer (`site/stats.html`) ships in a follow-up PR; this endpoint is usable on its own in the meantime. Cached 5 min in KV.
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
- `SUBSCRIPTIONS` — KV namespace (holds Pro keys, legacy ping counters, per-country ping counters under `pingc:`, per-fetch event counters under `evt:`, agent feedback under `fb:`, and the 5-min stats cache)
- `FEEDBACK_LIMITER` — rate-limit binding, 10 feedback posts per minute per caller IP (kept in memory by Cloudflare, never stored). Optional: without it the endpoint relies on the daily cap alone.

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
  "tool": "wick_fetch",
  "retry": false,
  "version": "0.9.2",
  "os": "macos"
}
```

`tool` (0.13+) is the MCP tool that made the fetch, or `cli` / `api`; values outside a fixed list are ignored. `retry` (0.13+) is true when the same host failed for this client in the last five minutes.

Stored in `SUBSCRIPTIONS` KV as one key per `(date, host, strategy)`:

| Field | Format |
|---|---|
| Key | `evt:YYYY-MM-DD:{host}:{strategy}` |
| Value | `{"fetches", "successes", "total_ms", "status_dist", "error_kind_dist", "country_dist", "tool_dist", "retries", "retry_successes"}` (JSON) |
| TTL | 30 days |

Increments are read-modify-write, same pattern as the legacy `/ping` counters. Under heavy concurrency a small number of increments may be lost; this is fine for telemetry.

`country_dist` counts the caller's country as Cloudflare geolocates it (`request.cf.country`: ISO code, `T1` for Tor, `XX` unknown). Only the code is stored. It is kept out of the public `/v1/stats/summary`, since per-host country counts can single out one user.

`/ping` also bumps `pingc:YYYY-MM-DD:{event}:{country}`, giving daily active installs by country (90-day TTL).

### Feedback

`POST /v1/feedback` accepts `{"kind": "capability_request", "task", "tried", "failure", "host", "tool", "version", "os"}` and stores one record at `fb:YYYY-MM-DD:{uuid}` with the caller's country, for 30 days. Free text is scrubbed by the client and again here: URLs keep only their host, emails, key-like tokens and runs of 7+ digits are replaced, and each field is capped at 600 characters. Both scrubbers are tested against `test/scrub-cases.json`. About 2000 records per day are kept at most.

What's **not** stored: URL paths or query strings, request/response bodies, page titles, caller IP (Cloudflare sees it at ingest, but it's never persisted as a data point), user identifier, machine ID.

## Querying

`GET /v1/stats/summary` does the aggregation and returns shaped JSON. The public renderer (`site/stats.html`) ships in a follow-up PR.

For ad-hoc debugging you can list KV keys directly:

```bash
npx wrangler kv key list --binding=SUBSCRIPTIONS --prefix='evt:' | head -20
npx wrangler kv key get --binding=SUBSCRIPTIONS 'evt:2026-04-21:example.com:cronet'
npx wrangler kv key list --binding=SUBSCRIPTIONS --prefix='fb:'      # agent feedback
```

## Tests

```bash
node test/smoke.mjs   # telemetry + feedback endpoints against an in-memory KV
```
