#!/usr/bin/env bash
# Wick reference benchmark — single sweep.
#
# Iterates the curated list in sites.txt, runs `wick fetch` against each,
# and lets Wick's normal telemetry pipeline post per-fetch events to
# releases.getwick.dev/v1/events. The local log (~/.wick/bench/*.log)
# is just for the operator to spot regressions / connectivity issues.
#
# Designed to be safe under cron / launchd:
#   - serial (no parallelism), so we don't burst on rate-limited targets
#   - light per-request sleep to look less batch-y
#   - randomized site order so we don't hammer the same hosts at :00
#     every hour
#   - per-fetch timeout so a hung site can't block the whole sweep
#
# Telemetry is what populates getwick.dev/stats.html; if WICK_TELEMETRY=0
# is set in the env, this script does its work but the public stats page
# won't see any of it. We don't override that — opting out is opting out.

set -u  # don't set -e: a failed fetch shouldn't kill the sweep

REPO_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SITES_FILE="$REPO_DIR/sites.txt"
PROXY_BUILDER="$REPO_DIR/proxy-providers.sh"

LOG_DIR="${WICK_BENCH_LOG_DIR:-$HOME/.wick/bench}"
TIMESTAMP="$(date -u +'%Y-%m-%dT%H:%M:%SZ')"
LOG_FILE="$LOG_DIR/sweep-$TIMESTAMP.log"

PER_REQUEST_TIMEOUT="${WICK_BENCH_FETCH_TIMEOUT:-30}"
SLEEP_BETWEEN="${WICK_BENCH_SLEEP:-1.5}"

# Optional proxy mode. When --provider=<name> is passed (or
# WICK_BENCH_PROVIDER is set in the env), every fetch tunnels through
# that residential provider. Otherwise the bench runs from the user's
# own IP — the default, mirroring what real users see.
PROVIDER="${WICK_BENCH_PROVIDER:-}"
COUNTRY="${WICK_BENCH_COUNTRY:-us}"
for arg in "$@"; do
    case $arg in
        --provider=*) PROVIDER="${arg#*=}" ;;
        --country=*)  COUNTRY="${arg#*=}" ;;
        *) echo "WARN: unknown arg ignored: $arg" >&2 ;;
    esac
done

mkdir -p "$LOG_DIR"

WICK_BIN="${WICK_BIN:-$(command -v wick)}"
if [[ -z "$WICK_BIN" ]]; then
    echo "ERROR: wick not found on PATH (set WICK_BIN to override)" >&2
    exit 1
fi

if [[ ! -f "$SITES_FILE" ]]; then
    echo "ERROR: sites file not found: $SITES_FILE" >&2
    exit 1
fi

if [[ -n "$PROVIDER" && ! -x "$PROXY_BUILDER" ]]; then
    echo "ERROR: --provider=$PROVIDER set but $PROXY_BUILDER is missing or not executable" >&2
    exit 1
fi

# Read non-comment, non-empty lines; shuffle so batch-iness is less obvious.
mapfile -t SITES < <(grep -v '^\s*#' "$SITES_FILE" | grep -v '^\s*$')
shuffled=()
while IFS= read -r line; do
    shuffled+=("$line")
done < <(printf '%s\n' "${SITES[@]}" | awk 'BEGIN{srand()} {print rand() "\t" $0}' | sort -k1,1n | cut -f2-)

mode_label="user-IP"
if [[ -n "$PROVIDER" ]]; then
    mode_label="proxy=$PROVIDER cc=$COUNTRY"
fi

echo "[$TIMESTAMP] sweep starting · ${#shuffled[@]} sites · ${mode_label} · timeout=${PER_REQUEST_TIMEOUT}s · sleep=${SLEEP_BETWEEN}s" | tee -a "$LOG_FILE"

ok=0
fail=0

for url in "${shuffled[@]}"; do
    # Build a fresh proxy URL per fetch when in proxy mode — each call
    # generates a new session ID, which prompts most providers to give us
    # a different exit IP. That's deliberate: the bench measures per-host
    # success across many residential IPs, not the same one repeated.
    proxy_args=()
    if [[ -n "$PROVIDER" ]]; then
        if proxy_url="$("$PROXY_BUILDER" --provider="$PROVIDER" --country="$COUNTRY" 2>>"$LOG_FILE")"; then
            proxy_args=(--proxy "$proxy_url")
        else
            echo "  ! could not build proxy URL — falling back to user IP for this fetch" | tee -a "$LOG_FILE"
        fi
    fi

    start=$(date +%s)
    # Discard stdout (the page content); we only want exit code + elapsed.
    # Telemetry is posted by wick itself in the background regardless.
    if timeout "$PER_REQUEST_TIMEOUT" "$WICK_BIN" fetch --no-robots "${proxy_args[@]}" "$url" >/dev/null 2>&1; then
        rc=0
    else
        rc=$?
    fi
    elapsed=$(( $(date +%s) - start ))
    if [[ $rc -eq 0 ]]; then
        ok=$((ok+1))
        printf '  ok   %3ds  %s\n' "$elapsed" "$url" | tee -a "$LOG_FILE"
    else
        fail=$((fail+1))
        printf '  fail %3ds (rc=%d)  %s\n' "$elapsed" "$rc" "$url" | tee -a "$LOG_FILE"
    fi
    sleep "$SLEEP_BETWEEN"
done

DONE_TS="$(date -u +'%Y-%m-%dT%H:%M:%SZ')"
echo "[$DONE_TS] sweep done · ok=$ok fail=$fail (out of ${#shuffled[@]})" | tee -a "$LOG_FILE"

# Compact: keep last 14 logs, drop older.
ls -1t "$LOG_DIR"/sweep-*.log 2>/dev/null | tail -n +15 | xargs rm -f 2>/dev/null || true
