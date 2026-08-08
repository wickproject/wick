#!/usr/bin/env bash
# Publish merged site-rules to the Worker, closing the self-improvement loop:
#
#   probe.sh → site-rules.measured.json ─┐
#   bundled seed (rust/data) ────────────┴─(merge)→ POST /v1/site-rules/:key
#                                                    → clients refresh daily
#
# Merge policy: measured rules WIN per host over the seed, so a measurement can
# *correct* an over-aggressive hand-seed (e.g. the probe finding that a site
# the seed flagged render:cef actually works on cronet). The seed supplies the
# long tail the harness hasn't probed yet.
#
# Auth: set WICK_PUBLISH_KEY to a Worker API key that has publish:true in the
# API_KEYS secret (the loop's publisher identity — a plain customer key is
# rejected). Use --dry-run to print the merged doc without publishing.

set -u

REPO_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SEED="${WICK_SEED_RULES:-$REPO_DIR/../rust/data/site-rules.json}"
MEASURED="${WICK_MEASURED_RULES:-${WICK_PROBE_OUT_DIR:-$HOME/.wick/probe}/site-rules.measured.json}"
URL="${WICK_RULES_PUBLISH_URL:-https://releases.getwick.dev/v1/site-rules}"

DRY_RUN=0
for arg in "$@"; do
    case $arg in
        --dry-run) DRY_RUN=1 ;;
        *) echo "WARN: unknown arg ignored: $arg" >&2 ;;
    esac
done

command -v jq >/dev/null   || { echo "ERROR: jq required" >&2; exit 1; }
command -v curl >/dev/null || { echo "ERROR: curl required" >&2; exit 1; }
[[ -f "$SEED" ]] || { echo "ERROR: seed not found: $SEED" >&2; exit 1; }

# Measured file is optional — first run, or a sweep that learned nothing new.
measured_rules='{}'
if [[ -f "$MEASURED" ]]; then
    measured_rules="$(jq -c '.rules // {}' "$MEASURED" 2>/dev/null || echo '{}')"
fi

# Merge: seed first, measured second (so measured wins on key collision).
MERGED="$(jq -nc \
    --slurpfile seed "$SEED" \
    --argjson measured "$measured_rules" \
    '{ version: 1,
       rules: (($seed[0].rules // {}) + $measured) }')"

n_seed="$(jq '.rules | length' "$SEED")"
n_measured="$(printf '%s' "$measured_rules" | jq 'length')"
n_merged="$(printf '%s' "$MERGED" | jq '.rules | length')"
echo "merge: seed=$n_seed + measured=$n_measured → $n_merged host(s)" >&2

if [[ "$DRY_RUN" -eq 1 ]]; then
    printf '%s\n' "$MERGED" | jq .
    echo "(--dry-run: not published)" >&2
    exit 0
fi

: "${WICK_PUBLISH_KEY:?set WICK_PUBLISH_KEY to a publisher API key (publish:true in API_KEYS), or pass --dry-run}"

resp="$(curl -s -w '\n%{http_code}' -X POST "$URL/$WICK_PUBLISH_KEY" \
    -H 'content-type: application/json' --data-binary "$MERGED")"
code="$(printf '%s' "$resp" | tail -n1)"
body="$(printf '%s' "$resp" | sed '$d')"
if [[ "$code" == "200" ]]; then
    echo "published: $body" >&2
else
    echo "ERROR: publish failed (HTTP $code): $body" >&2
    exit 1
fi
