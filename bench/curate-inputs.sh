#!/usr/bin/env bash
# Gather the weekly curation inputs into one JSON document for the
# /wick-curate agent to reason over. Read-only and creds-free: public stats +
# GET /v1/site-rules + the local probe traces. The agent (see
# agent-skill/wick-curate/SKILL.md) consumes this to decide what to re-probe
# and what new methods to try.
#
# Output (stdout): { failing: [...], hard: [...], published_rules: {...},
#                    generated_at: <stats.generated_at> }
#   failing  — site-side failing hosts (low success, failures NOT mostly
#              user-offline), with a per-cause breakdown so the agent can see
#              WHY each fails (reset/refused/timeout/403…).
#   hard     — hosts from the latest probe sweep where EVERY tested cell failed
#              (cronet / cronet+residential / cef) — the ones that need a method
#              the harness hasn't tried yet.
#   published_rules — what clients are currently being served.

set -u

STATS_URL="${WICK_STATS_URL:-https://releases.getwick.dev/v1/stats/summary}"
RULES_URL="${WICK_RULES_URL:-https://releases.getwick.dev/v1/site-rules}"
PROBE_DIR="${WICK_PROBE_OUT_DIR:-$HOME/.wick/probe}"
MIN_FETCHES="${WICK_CURATE_MIN_FETCHES:-4}"
MAX_SR="${WICK_CURATE_MAX_SR:-0.5}"

command -v jq >/dev/null   || { echo "ERROR: jq required" >&2; exit 1; }
command -v curl >/dev/null || { echo "ERROR: curl required" >&2; exit 1; }

fetch() { curl -s --max-time 30 --retry 2 "$1"; }

stats="$(fetch "$STATS_URL")"
printf '%s' "$stats" | jq -e '.rows' >/dev/null 2>&1 || { echo "ERROR: bad stats from $STATS_URL" >&2; exit 1; }

rules="$(fetch "$RULES_URL")"
printf '%s' "$rules" | jq -e '.' >/dev/null 2>&1 || rules='{"rules":{}}'

latest_trace="$(ls -1t "$PROBE_DIR"/probe-*.jsonl 2>/dev/null | head -1)"
traces='[]'
[ -n "$latest_trace" ] && traces="$(jq -s '.' "$latest_trace" 2>/dev/null || echo '[]')"

jq -n \
  --argjson stats "$stats" \
  --argjson rules "$rules" \
  --argjson traces "$traces" \
  --argjson minf "$MIN_FETCHES" \
  --argjson maxsr "$MAX_SR" '
  {
    generated_at: ($stats.generated_at // null),
    failing: (
      $stats.rows
      | group_by(.host)
      | map({
          host: .[0].host,
          fetches:   (map(.fetches)   | add),
          successes: (map(.successes) | add),
          offline:   (map((.error_kind_dist // {}).offline // 0) | add),
          causes: (
            reduce .[] as $r ({};
              reduce (($r.error_kind_dist // {}) | to_entries[]) as $e (.;
                .[$e.key] = ((.[$e.key] // 0) + $e.value)))
          ),
        }
        | . + { sr: (if .fetches > 0 then (.successes / .fetches) else 1 end),
                failures: (.fetches - .successes) })
      | map(select(.fetches >= $minf and .sr < $maxsr
                   and (.failures <= 0 or (.offline / .failures) < 0.5)))
      | sort_by(.sr, (-.fetches))
    ),
    hard: (
      $traces
      | map(select((.cells | to_entries | any(.value | startswith("ok"))) | not))
    ),
    published_rules: ($rules.rules // {}),
  }'
