# Wick Reference Benchmark

A continuously-updated, public benchmark of Wick's per-site fetch success rate. Runs hourly against a curated list of ~110 sites spanning easy / medium / hard targets, on a single residential IP, with full transparency on methodology.

The data feeds **[getwick.dev/stats.html](https://getwick.dev/stats.html)** — the same page real-user telemetry feeds — so the public stats reflect both organic adoption and this systematic sweep.

## Why this exists

The agentic-browser space is full of "we work on every site" claims. Public, time-series, per-site success-rate data is rare. We built this so the question "does Wick work on `<site>`" is answered by data, not marketing.

## Methodology

| | |
|---|---|
| **Target list** | [`bench/sites.txt`](sites.txt) — ~110 URLs, hand-curated, public |
| **Frequency** | Every 3600s (`launchd` `StartInterval`) |
| **Concurrency** | Serial (one fetch at a time) |
| **Order** | Randomized per sweep so the same hosts don't always hit at :00 |
| **Per-request timeout** | 30 s |
| **Inter-request sleep** | 1.5 s |
| **IP source** | User's own residential IP by default; optional residential-pool rotation via `--provider=<name>` (see below) |
| **Wick version** | Whatever's on `PATH` (`wick version` to check) |
| **Robots.txt** | Bypassed for the bench (`--no-robots`); respected in normal user fetches |
| **What's reported** | `{host, strategy, ok, status, timing_ms}` per fetch — no URL paths, no content, no IP |
| **Where it lands** | `releases.getwick.dev/v1/events` → KV → `getwick.dev/stats.html` |

### Two modes

**User-IP mode** (default, no flags). Fetches go out from your machine's
own residential IP. This is what real users see, and the most honest
"per-site Wick success rate" data. The downside: a single IP burns
reputation against hard targets faster than typical usage would, so the
hard-target numbers under-report.

**Proxy-pool mode** (`--provider=<name>`). Each fetch tunnels through a
fresh session on a residential proxy provider — so each fetch from a
different exit IP. This measures Wick's *capability ceiling* on a clean
IP rotation, comparable to how TWSC's LAB shootouts test against the
same provider pools (Bright Data residential FR, Geonode residential
FR). Five providers supported: `oxylabs`, `brightdata`, `iproyal`,
`soax`, `packetstream`. Credential conventions per provider are lifted
verbatim from `getlantern/lantern-cloud/cmd/pinger` (so a single set of
env vars works for both Lantern's pinger and Wick's bench).

## Site categories

The list spans:

- **Easy** — news, reference, dev tooling. Cronet expected to win.
- **Medium** — e-commerce, travel, social, finance. Mix of CDN and bot-management.
- **Hard, DataDome** — same vendor [TWSC LAB #103](https://substack.thewebscraping.club/p/the-lab-103-bypassing-datadome-protected) benchmarked, including `leroymerlin.fr`, `vinted.com`, `mediamarkt.de`, `indeed.com`, `glassdoor.com`.
- **Hard, Cloudflare Turnstile / Bot Management** — `cloudflare.com`, `discord.com`, `patreon.com`, `chess.com`.
- **Hard, Akamai** — `linkedin.com`, `nike.com`, `adidas.com`.
- **Hard, PerimeterX / HUMAN** — `zillow.com`, `realtor.com`, `crunchbase.com`, `ticketmaster.com`.
- **International** — French / German / Spanish / Italian / Japanese press for geo + language diversity.

## Honest caveats

- **Single residential IP.** Real-world Wick users span many residential IPs; this bench burns one IP's reputation on hard targets faster than typical usage would. Reading the bench data alone *understates* the success rate a fresh user would see on their first visit.
- **Site list is public.** Anti-bot vendors who notice Wick on their radar can preferentially target this exact list. If we see a site flip from green to red overnight, that's information — but it's also the cost of being open about methodology.
- **No login flows.** This bench tests landing-page reachability, not multi-step interaction. TWSC's LAB shootouts cover the multi-step case more thoroughly.
- **Hourly is sparse for high-volatility sites.** Anti-bot rules can change faster than this bench can detect; the ground truth lags by up to an hour.

## Running it

### Prerequisites

```bash
# macOS
brew tap wickproject/wick && brew install wick

# Or
npm install -g wick-mcp
```

Verify:
```bash
wick version  # should print 0.10.x or later
```

### Install (macOS)

```bash
bash bench/install.sh
```

This:
1. Substitutes the repo path into `bench/com.wickproject.bench.plist`
2. Drops the plist into `~/Library/LaunchAgents/`
3. Loads it via `launchctl`
4. Triggers an immediate first sweep (~3–5 minutes)

### Run a single sweep manually

```bash
# User's own IP (default)
bash bench/run.sh

# Through a residential proxy pool
export OXY_USER=your_oxylabs_user
export OXY_PASS=your_oxylabs_password
bash bench/run.sh --provider=oxylabs --country=us
```

Required env vars per provider:

| Provider | Variables |
|---|---|
| `oxylabs` | `OXY_USER`, `OXY_PASS` |
| `brightdata` | `BRD_CUSTOMER_ID`, `BRD_ZONE`, `BRD_PASSWORD`, `BRD_PORT` (optional, defaults to 24000 SOCKS5) |
| `iproyal` | `IPR_USER`, `IPR_PASSWORD` |
| `soax` | `SOAX_PACKAGE_ID`, `SOAX_PASSWORD` |
| `packetstream` | `PS_USER`, `PS_AUTH_KEY` |

To run the proxied bench under launchd, add the env vars to the plist's
`EnvironmentVariables` dict and bake `--provider=<name>` into
`ProgramArguments`. Don't put credentials in the plist if your
`~/Library/LaunchAgents/` is in iCloud or otherwise synced.

### Install (Linux)

`launchd` is macOS-only. On Linux, drop this into `crontab -e`:

```
# Every hour, top of the hour
0 * * * * /bin/bash /absolute/path/to/wick/bench/run.sh >> /var/log/wick-bench.log 2>&1
```

### Stop

```bash
bash bench/uninstall.sh   # macOS
crontab -e                # Linux: remove the line
```

### Inspect logs

```bash
ls -lth ~/.wick/bench/sweep-*.log | head -3
tail -f ~/.wick/bench/sweep-$(date -u +'%Y-%m-%d')*.log
```

The log shows pass/fail per URL and overall sweep summary. The richer data — `strategy`, `status`, `timing_ms` per host — lives in the public stats endpoint:

```bash
curl -s https://releases.getwick.dev/v1/stats/summary | jq '.rows | sort_by(-.fetches) | .[0:20]'
```

## Telemetry / opt-out

The bench uses Wick's normal telemetry pipeline. If you've set `WICK_TELEMETRY=0` or `~/.wick/no-telemetry`, the bench still runs but its data won't reach the public stats page. If you want the bench data to be public (which is the whole point), make sure neither flag is set.

Privacy details: [`getwick.dev/docs.html#telemetry`](https://getwick.dev/docs.html#telemetry).

## Adding sites

PRs welcome. Keep the categorization tidy and don't pile up multiple sites behind the same vendor unless the variation is meaningful (e.g. different geos, different sub-products).
