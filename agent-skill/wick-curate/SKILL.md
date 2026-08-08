---
name: wick-curate
description: Weekly self-improvement curation pass for Wick's site-rules. Reads the public stats (with per-cause failure breakdown), the residential probe traces, and the currently-published rules, then reasons about anomalies the deterministic harness can't resolve on its own — sites that fail EVERY tested strategy, high user-offline noise, seed/measured conflicts — and proposes + tests genuinely NEW access methods (different residential country, wait-for-selector, URL rewrites, CEF+residential). Use weekly, or after a stats regression on a high-volume host. Operator-facing: needs prod Vault creds (via /residential-proxy) and a Worker publish key.
---

# Wick site-rules curation (the weekly "invent new methods" pass)

The deterministic harness (`bench/probe.sh`) measures a fixed matrix —
`cronet | cronet+residential | cef` — and corrects the rules from that. This
skill is the **judgment layer on top**: it looks at what the matrix *couldn't*
crack and what the stats reveal, then reasons about methods the matrix doesn't
try. It is the "H reviews anomalies and invents new methods" box in the
self-improvement loop.

Run it **weekly** (rules change slowly), or when a high-volume host regresses.

## Step 1 — gather + triage

```bash
bash bench/curate-inputs.sh > /tmp/curate.json     # read-only, no creds
jq '{failing: (.failing|length), hard: (.hard|length)}' /tmp/curate.json
```

`curate-inputs.sh` returns three buckets. Read them and sort hosts into:

- **`hard`** — the probe's latest sweep tried `cronet`, `cronet+residential`,
  and `cef` and *every cell failed*. These are the real targets for this pass:
  the matrix has nothing left to offer, so a new method is needed.
- **`failing`** — site-side failing hosts (from stats) with a `causes`
  breakdown. Use the cause to reason about *why*:
  - mostly `reset` / `refused` → active blocking (anti-bot edge or RST). CEF +
    a clean residential IP is the usual answer; if already tried, see "new
    methods" below.
  - mostly `403` in `status_dist` → datacenter-IP block or bot fingerprint →
    `needs_residential` and/or CEF.
  - **mostly `offline`** → user-side noise, NOT a hard site. Do not propose a
    rule; note it and move on. (The probe already excludes these, but the stats
    list still surfaces them — don't be fooled.)
  - `dns` with the site clearly up → likely the *reporting users'* resolver, not
    the site; low priority.
- **seed/measured conflicts** — where `published_rules` shows a `source:"seed"`
  entry that the latest probe contradicts. The publish merge already lets
  measured win, but flag low-confidence ones for a re-probe.

## Step 2 — reason about NEW methods (the part the matrix can't do)

For each `hard` host, form a hypothesis the harness hasn't tested. In rough
priority order:

1. **Different residential country.** The probe defaults to `us`. A site may
   geo-block US residential but allow its home country. Probe its likely region
   (a `.fr`/`.de`/`.jp` TLD, a known regional service). Confirm availability
   first: `bash <skills>/scripts/residential-probe.sh <CC>`.
2. **`wait_for_selector`.** A `cef` cell that returns a 200 with tiny
   `content_bytes` is an SPA that hadn't hydrated when the DOM was dumped.
   Open it interactively to find the content selector, then encode it in the
   rule (`wait_for_selector`), so CEF waits for real content.
   ```bash
   wick fetch --render cef --wait-for-selector 'article' --json <url>
   ```
3. **URL rewrite.** Some sites have a lighter endpoint that isn't bot-walled
   (the built-in `www.reddit.com → old.reddit.com` rewrite is the canonical
   example). If you find one, it belongs in `fetch.rs`'s rewrite list, not a
   rule — open a PR.
4. **CEF + residential.** A DataDome-class site (e.g. `apkpure` — failed every
   cell) typically needs *both* a real browser and a clean IP. The SOCKS
   harness can't test this combination (`--proxy` routes Cronet, not CEF — CEF
   residential is a WireGuard preload). Verify it from a tunneled Linux box, or
   record the hypothesis and flag for that environment.
5. **Nothing plausible** → open a GitHub issue documenting the host, the cells
   tried, and the causes, so a human can investigate. Don't invent a rule that
   isn't backed by a passing probe.

## Step 3 — test the hypothesis

Re-probe just the candidates with the tuned parameters, e.g. a different
country:

```bash
source <skills>/scripts/residential-proxy-env.sh
# Probe a specific region for geo-blocked hosts (probe.sh sweeps the failing
# set; for a one-off host use wick fetch directly through the proxy):
PX=$(bash bench/proxy-providers.sh --provider=oxylabs --country=fr)
wick fetch --json --no-robots --render cef --proxy "$PX" https://<host>/
```

A cell counts as a win only at `status 200` with `content_bytes` above the
block threshold (~1000) — a 200 with near-zero content is a challenge shell,
not success (same rule the harness uses).

## Step 4 — act

- **A new method worked** → add/adjust the measured rule and republish:
  ```bash
  WICK_PUBLISH_KEY=<key> bash bench/publish-rules.sh        # --dry-run first
  ```
  (If the win needs a param the rule schema can't express yet — e.g. a per-host
  residential country — open a PR extending the schema; don't fake it.)
- **A seed is wrong** (probe contradicts it and the probe is trustworthy) → the
  merge already corrects it on publish; just confirm it's published.
- **Nothing worked** → file the issue from Step 2.5 and leave the host's
  existing rule/seed untouched.

## Step 5 — log

Append a short curation note (date, hosts reviewed, methods tried, outcomes) to
`~/.wick/probe/curation-log.md` so the next pass sees what's already been tried
and doesn't re-litigate dead ends.

## Scheduling

```
# Sundays 05:00 — after the Sunday 04:00 probe sweep has refreshed the traces
0 5 * * 0  cd /abs/path/wick && bash bench/curate-inputs.sh > /tmp/curate.json && \
           claude -p "/wick-curate review /tmp/curate.json and act per the skill"
```

## Guardrails

- **Never publish a rule not backed by a passing probe.** A hypothesis is not a
  rule. The whole point of the loop is that rules are *measured*.
- **Respect the offline signal.** A high `offline` fraction means the failures
  are users' networks, not the site — excluding these is the difference between
  improving and chasing ghosts.
- **Residential is for reachability testing of our own routing**, per
  `/residential-proxy` — not general scraping.
- Methodology caveats (operator-vantage baseline, SOCKS-vs-CEF) live in
  `bench/PROBE.md`; read them before trusting a single probe.
