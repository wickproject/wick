#!/usr/bin/env bash
# Wick self-improvement probe harness (PR2).
#
# Closes the loop: read which sites Wick is FAILING on (from the public stats
# endpoint), then empirically test access methods against each through a
# residential proxy, and emit measured per-site rules in the site-rules.json
# schema that fetch.rs consumes.
#
#   stats → candidates → strategy matrix → winner → site-rules.measured.json
#
# Pipeline:
#   1. Pull releases.getwick.dev/v1/stats/summary, aggregate per host, and
#      select genuinely SITE-SIDE failing hosts — explicitly dropping hosts
#      whose failures are mostly error_kind="offline" (the user's own network),
#      so we never chase phantom "this site is hard" signals.
#   2. For each candidate, run a strategy matrix via `wick fetch --json`:
#        - cronet            (--render cronet)                 [direct]
#        - cronet+residential(--render cronet --proxy <url>)   [datacenter-block test]
#        - cef               (--render cef)                    [JS / bot-managed test]
#      (cef+residential is NOT tested here: --proxy routes only the Cronet/
#      reqwest engine, not CEF, whose residential path is a WireGuard preload
#      on tunneled servers. The rule still combines render:cef + needs_residential
#      when both independent signals fire; PR4's agent refines.)
#   3. Decide the winner and derive the rule:
#        render            = "cef"  if cef succeeds AND cronet-direct fails
#        needs_residential = true   if cronet+residential succeeds AND cronet-direct fails
#   4. Emit measured rules (source:"measured", with sample count + date) and a
#      per-host JSONL trace.
#
# Residential creds come from the env (same convention as run.sh /
# proxy-providers.sh). Source the residential-proxy skill's env first:
#   source <skills>/scripts/residential-proxy-env.sh   # exports OXY_USER, ...
#   bash bench/probe.sh --provider=oxylabs --country=us
#
# Safe under cron/launchd: serial, per-request timeout, polite sleep.

set -u  # NOT -e: a single failed probe must never kill the sweep.

REPO_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROXY_BUILDER="$REPO_DIR/proxy-providers.sh"
STATS_URL="${WICK_STATS_URL:-https://releases.getwick.dev/v1/stats/summary}"

OUT_DIR="${WICK_PROBE_OUT_DIR:-$HOME/.wick/probe}"
TS="$(date -u +'%Y-%m-%dT%H:%M:%SZ')"
DAY="$(date -u +'%Y-%m-%d')"
RESULTS="$OUT_DIR/probe-$TS.jsonl"
RULES_OUT="$OUT_DIR/site-rules.measured.json"

# Tunables.
PROVIDER="${WICK_PROBE_PROVIDER:-}"
COUNTRY="${WICK_PROBE_COUNTRY:-us}"
MAX_HOSTS="${WICK_PROBE_MAX_HOSTS:-25}"
MIN_FETCHES="${WICK_PROBE_MIN_FETCHES:-4}"     # ignore low-volume noise
MAX_SUCCESS_RATE="${WICK_PROBE_MAX_SR:-0.5}"   # candidate if site-side SR below this
MIN_OK_BYTES="${WICK_PROBE_MIN_BYTES:-1000}"   # a 200 with < this many bytes of extracted content = block/shell
PER_REQUEST_TIMEOUT="${WICK_PROBE_TIMEOUT:-40}"
SLEEP_BETWEEN="${WICK_PROBE_SLEEP:-2}"
DRY_RUN=0

for arg in "$@"; do
    case $arg in
        --provider=*) PROVIDER="${arg#*=}" ;;
        --country=*)  COUNTRY="${arg#*=}" ;;
        --max-hosts=*) MAX_HOSTS="${arg#*=}" ;;
        --dry-run)    DRY_RUN=1 ;;
        *) echo "WARN: unknown arg ignored: $arg" >&2 ;;
    esac
done

command -v jq >/dev/null   || { echo "ERROR: jq required" >&2; exit 1; }
command -v curl >/dev/null || { echo "ERROR: curl required" >&2; exit 1; }
WICK_BIN="${WICK_BIN:-$(command -v wick)}"
if [[ -z "$WICK_BIN" && "$DRY_RUN" -eq 0 ]]; then
    echo "ERROR: wick not found on PATH (set WICK_BIN), or pass --dry-run" >&2
    exit 1
fi
mkdir -p "$OUT_DIR"

# Resolve a timeout command: GNU coreutils ships `timeout`; macOS only has it
# as `gtimeout` (after `brew install coreutils`). Without either, run with no
# per-request timeout (and warn) rather than failing the whole sweep — the
# README documents macOS/launchd usage, so a hard dependency on `timeout`
# would break the default Mac.
TIMEOUT_BIN="$(command -v timeout 2>/dev/null || command -v gtimeout 2>/dev/null || true)"
if [[ -z "$TIMEOUT_BIN" && "$DRY_RUN" -eq 0 ]]; then
    echo "WARN: no 'timeout'/'gtimeout' on PATH — running without a per-request timeout (macOS: brew install coreutils)" >&2
fi

# ── Step 1: candidate selection ─────────────────────────────────────────────
# Aggregate the per-(host,strategy) rows into per-host totals. A host is a
# candidate when it has real volume, a low overall success rate, AND its
# failures are predominantly site-side (offline fraction < 0.5). error_kind_dist
# may be absent until the worker is deployed + clients ship; treat missing as
# offline=0 (so we don't accidentally exclude everything in the meantime).
echo "[$TS] fetching stats: $STATS_URL" >&2
STATS_JSON=""
for attempt in 1 2 3 4; do
    # --retry handles curl's own transient transport errors; the outer loop
    # also retries an empty/non-JSON body (a transient edge blip we've seen).
    if STATS_JSON="$(curl -s --max-time 30 --retry 2 "$STATS_URL")" \
        && printf '%s' "$STATS_JSON" | jq -e '.rows' >/dev/null 2>&1; then
        break
    fi
    echo "  stats fetch attempt $attempt failed; retrying in 3s…" >&2
    STATS_JSON=""
    sleep 3
done
[[ -n "$STATS_JSON" ]] || { echo "ERROR: stats fetch failed after retries" >&2; exit 1; }

CANDIDATES="$(printf '%s' "$STATS_JSON" | jq -r --argjson minf "$MIN_FETCHES" --argjson maxsr "$MAX_SUCCESS_RATE" '
  [ .rows[]
    | { host, fetches, successes,
        offline: ((.error_kind_dist // {}).offline // 0) }
  ]
  | group_by(.host)
  | map({
      host: .[0].host,
      fetches:   (map(.fetches)   | add),
      successes: (map(.successes) | add),
      offline:   (map(.offline)   | add),
    })
  | map(. + {
      failures: (.fetches - .successes),
      sr: (if .fetches > 0 then (.successes / .fetches) else 1 end),
    })
  # real volume, low success, and failures that are mostly NOT user-offline
  | map(select(.fetches >= $minf and .sr < $maxsr
               and (.failures <= 0 or (.offline / .failures) < 0.5)))
  | sort_by(.sr, (-.fetches))
  | .[].host
')"

mapfile -t HOSTS < <(printf '%s\n' "$CANDIDATES" | grep -v '^\s*$' | head -n "$MAX_HOSTS")

echo "[$TS] ${#HOSTS[@]} site-side failing candidate host(s) (max=$MAX_HOSTS):" >&2
printf '  %s\n' "${HOSTS[@]}" >&2

if [[ "$DRY_RUN" -eq 1 ]]; then
    echo "[$TS] --dry-run: stopping before probing. Matrix per host would be: cronet | cronet+residential | cef" >&2
    exit 0
fi

# Residential proxy is required to test the needs_residential signal.
if [[ -z "$PROVIDER" ]]; then
    echo "WARN: no --provider set; testing cronet-direct and cef-direct only (cannot derive needs_residential)." >&2
fi

# Build a fresh residential proxy URL (new session → new exit IP) per call.
# Scheme is provider-specific (oxylabs = HTTP CONNECT, others = SOCKS5).
build_proxy() {
    [[ -z "$PROVIDER" ]] && return 1
    "$PROXY_BUILDER" --provider="$PROVIDER" --country="$COUNTRY" 2>>"$RESULTS.err"
}

# Run one matrix cell. Echoes "ok <status> <bytes>" or "fail <rc>".
probe_cell() {
    local url="$1" render="$2" proxy="$3"
    local args=(fetch --json --no-robots --render "$render")
    [[ -n "$proxy" ]] && args+=(--proxy "$proxy")
    args+=("$url")
    local out rc
    if [[ -n "$TIMEOUT_BIN" ]]; then
        out="$(WICK_AUTO_INSTALL_CEF=1 "$TIMEOUT_BIN" "$PER_REQUEST_TIMEOUT" "$WICK_BIN" "${args[@]}" 2>/dev/null)"; rc=$?
    else
        out="$(WICK_AUTO_INSTALL_CEF=1 "$WICK_BIN" "${args[@]}" 2>/dev/null)"; rc=$?
    fi
    if [[ $rc -ne 0 ]]; then
        echo "fail $rc"
        return
    fi
    local status bytes
    status="$(printf '%s' "$out" | jq -r '.status_code // 0' 2>/dev/null)"
    # content_bytes = extracted-content size; a challenge/JS shell extracts to
    # near nothing, so a small value below means a block (not bytes-on-wire).
    bytes="$(printf '%s' "$out" | jq -r '.content_bytes // 0' 2>/dev/null)"
    if [[ "$status" == "200" && "${bytes:-0}" -ge "$MIN_OK_BYTES" ]]; then
        echo "ok $status $bytes"
    else
        echo "fail-block ${status:-0} ${bytes:-0}"
    fi
}

# ── Step 2 + 3: matrix + decision ───────────────────────────────────────────
: > "$RESULTS"
for host in "${HOSTS[@]}"; do
    url="https://$host/"
    cronet="$(probe_cell "$url" cronet "")";          sleep "$SLEEP_BETWEEN"
    cef="$(probe_cell "$url" cef "")";                sleep "$SLEEP_BETWEEN"
    cronet_res="n/a"
    if [[ -n "$PROVIDER" ]]; then
        if px="$(build_proxy)"; then
            cronet_res="$(probe_cell "$url" cronet "$px")"; sleep "$SLEEP_BETWEEN"
        fi
    fi

    cronet_ok=0;     [[ "$cronet"     == ok* ]] && cronet_ok=1
    cef_ok=0;        [[ "$cef"        == ok* ]] && cef_ok=1
    cronet_res_ok=0; [[ "$cronet_res" == ok* ]] && cronet_res_ok=1

    # render: cef only when cef rescues a cronet-direct failure.
    render="cronet"
    [[ "$cronet_ok" -eq 0 && "$cef_ok" -eq 1 ]] && render="cef"
    # needs_residential: residential rescues a cronet-direct failure.
    needs_res="false"
    [[ "$cronet_ok" -eq 0 && "$cronet_res_ok" -eq 1 ]] && needs_res="true"

    jq -nc \
        --arg host "$host" --arg render "$render" --argjson needs_res "$needs_res" \
        --arg cronet "$cronet" --arg cef "$cef" --arg cronet_res "$cronet_res" \
        --arg ts "$TS" \
        '{host:$host, render:$render, needs_residential:$needs_res,
          cells:{cronet:$cronet, cef:$cef, cronet_residential:$cronet_res}, probed_at:$ts}' \
        | tee -a "$RESULTS" >&2
done

# ── Step 4: emit measured rules ─────────────────────────────────────────────
# Emit the measured verdict for every host where SOME strategy worked — including
# render:cronet. That's deliberate: a measurement of "cronet works here" must be
# able to CORRECT an over-aggressive hand-seed (the published overlay overrides
# the bundled seed per host). A host where every cell failed (e.g. apkpure, hard
# even via residential) emits nothing, so its seed stays until we learn a method
# that works. confidence is modest for a single sweep; repeated sweeps / PR4's
# agent raise it.
jq -s --arg day "$DAY" '
  { version: 1, updated_at: $day,
    note: "measured by bench/probe.sh",
    rules: (
      [ .[]
        | select(.cells | to_entries | any(.value | startswith("ok")))
        # Key on the bare host (strip leading www.) to match the seed
        # convention plus the client parent-domain walk, so a measurement
        # OVERRIDES a same-host seed instead of sitting beside it.
        | { key: (.host | sub("^www\\."; "")),
            value: { render: .render, needs_residential: .needs_residential,
                     vendor: "measured", confidence: 0.7, source: "measured",
                     updated_at: $day } }
      ] | from_entries)
  }' "$RESULTS" > "$RULES_OUT"

echo "[$TS] wrote $(jq '.rules | length' "$RULES_OUT") measured rule(s) → $RULES_OUT" >&2
echo "[$TS] per-host trace → $RESULTS" >&2
