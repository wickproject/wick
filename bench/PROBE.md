# Wick self-improvement probe harness

Closes the loop from the public stats page back into Wick's routing: read which
sites Wick is **failing on**, empirically test access methods through a
residential proxy, and publish measured per-site rules that every client picks
up — so the curated "known behaviors" list (`rust/data/site-rules.json` +
`GET /v1/site-rules`) constantly evolves instead of relying on hand-seeds.

```
 /v1/stats/summary ──→ probe.sh ──→ site-rules.measured.json ──→ publish-rules.sh ──→ POST /v1/site-rules
   (failing sites)      (matrix)      (measured verdicts)          (merge w/ seed)       (clients refresh)
```

## The pipeline

| stage | script | what it does |
|---|---|---|
| select | `probe.sh` step 1 | Pull `/v1/stats/summary`, aggregate per host, keep **site-side** failing hosts. Drops hosts whose failures are mostly `error_kind="offline"` (the user's own network) — so we never chase phantom "this site is hard" signals. |
| probe | `probe.sh` step 2–3 | Per host, run a matrix via `wick fetch --json`: `cronet` \| `cronet+residential` \| `cef`. Derive `render` (cef only if it beats a cronet failure) and `needs_residential` (residential beats a cronet failure). |
| emit | `probe.sh` step 4 | Write `~/.wick/probe/site-rules.measured.json` — a measured verdict for every host where *some* strategy worked (incl. `render:cronet`, so a measurement can **correct** an over-aggressive seed). Key is the host with a leading `www.` stripped, matching the seed convention. |
| publish | `publish-rules.sh` | Merge seed ∪ measured (**measured wins per host**) and `POST /v1/site-rules/:key`. |
| consume | client | `wick` refreshes `GET /v1/site-rules` into `<wick-home>/site-rules.json` daily; that overlay overrides the bundled seed (`site_rules.rs`). |

## Running it

```bash
# 1. residential creds from Vault (prod tailnet + GCP ADC required)
source <skills>/scripts/residential-proxy-env.sh         # OXY_USER/OXY_PASS, ...

# 2. ALWAYS probe availability first — it's time-varying
bash <skills>/scripts/residential-probe.sh US

# 3. sweep (oxylabs is the reliable US provider; HTTP CONNECT :443-only)
bash bench/probe.sh --provider=oxylabs --country=us --max-hosts=15
#    → ~/.wick/probe/probe-<ts>.jsonl  (per-host trace)
#    → ~/.wick/probe/site-rules.measured.json

# 4. publish (needs a Worker API key)
WICK_PUBLISH_KEY=<key> bash bench/publish-rules.sh        # or --dry-run to preview

# candidate selection alone (no creds): bash bench/probe.sh --dry-run
```

## Scheduling

Rules change slowly and residential probing has cost, so **weekly** is plenty.
Cron (sources creds at runtime — never bake Vault creds into the job):

```
# Sundays 04:00 — sweep the current failing set and republish
0 4 * * 0  source $HOME/.../scripts/residential-proxy-env.sh && \
           bash /abs/path/wick/bench/probe.sh --provider=oxylabs --country=us --max-hosts=25 && \
           WICK_PUBLISH_KEY=$WICK_PUBLISH_KEY bash /abs/path/wick/bench/publish-rules.sh
```

## Methodology caveats (read before trusting a single sweep)

- **The `cronet` baseline cell uses the operator's own IP.** If that IP is clean
  (residential / office), `cronet` succeeds and we conclude `needs_residential:false`
  — even though the *datacenter*-hosted clients that generate much of the failing
  telemetry would need residential. To detect `needs_residential` faithfully, run
  the harness **from a datacenter VM** so the baseline matches the failing
  population. (First live sweep, 2026-06-26, ran from a clean US vantage and found
  reuters/cfr/tradingview/apkmirror/apkcombo all work on plain Cronet — i.e. the
  telemetry failures were vantage-specific or user-side noise, and the hand-seeds
  for those hosts were over-aggressive. The loop corrected them to `cronet`.)
- **`--proxy` (SOCKS/HTTP) routes only Cronet, not CEF.** CEF's residential path is
  a WireGuard `LD_PRELOAD` (`bindwg.so`) that exists only on tunneled Linux servers,
  so the `cef+residential` combination is **not** tested here. `render:cef` and
  `needs_residential` are derived as independent signals; a site needing *both*
  (e.g. apkpure — DataDome, failed every testable cell) is left to its seed / PR4's
  agent.
- **Single residential IP per session, single country.** A site reachable from a
  different country/ISP won't show it. Sweep multiple `--country` values for
  geo-sensitive targets.
- A `200` under `MIN_OK_BYTES` (default 1000) is treated as a block/challenge shell,
  not success (matches `fetch.rs`'s `is_acceptable_render`).
