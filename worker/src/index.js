/**
 * Wick — Release Server + Telemetry
 *
 * Public routes:
 *   GET  /install-pro.sh, /install-pro-mac.sh, /wick-tunnel
 *   GET  /financial-data
 *   POST /ping                   → legacy daily usage ping
 *   POST /v1/events              → per-fetch telemetry
 *   GET  /v1/stats/summary       → public 7-day stats aggregate
 *   POST /v1/feedback            → agent capability requests (wick_request_capability)
 *
 * Protected (API key required, set via API_KEYS secret):
 *   GET  /releases/:key/:file    → pro tarball + CEF runtime downloads
 *   POST /solve/:key             → CAPTCHA solve proxy
 *   POST /proxy/:key             → geo-proxy
 *   GET  /analytics/:key         → per-customer metrics dashboard
 */

// ── Helpers ─────────────────────────────────────────────────

/**
 * True if `hostname` is a loopback/private/link-local address literal.
 * Used by the geo-proxy to block SSRF against internal networks. Only
 * catches IP-literal targets; DNS-based rebinding is not prevented
 * here (Workers fetch() resolves names internally).
 */
function isPrivateHost(hostname) {
  if (!hostname) return true;
  const h = hostname.toLowerCase();
  if (h === "localhost" || h === "localhost." || h.endsWith(".localhost")) return true;

  // IPv4 literal
  const v4 = h.match(/^(\d{1,3})\.(\d{1,3})\.(\d{1,3})\.(\d{1,3})$/);
  if (v4) {
    const a = Number(v4[1]), b = Number(v4[2]);
    if ([a, b, Number(v4[3]), Number(v4[4])].some(x => x > 255)) return true;
    if (a === 0) return true;                                 // 0.0.0.0/8
    if (a === 10) return true;                                // 10.0.0.0/8
    if (a === 127) return true;                               // loopback
    if (a === 169 && b === 254) return true;                  // link-local
    if (a === 172 && b >= 16 && b <= 31) return true;         // RFC1918
    if (a === 192 && b === 168) return true;                  // RFC1918
    if (a === 192 && b === 0 && Number(v4[3]) === 2) return true; // TEST-NET
    if (a === 100 && b >= 64 && b <= 127) return true;        // CGNAT
    return false;
  }

  // IPv6 literal — URL parses as `[...]`; hostname strips brackets.
  if (h.includes(":")) {
    if (h === "::" || h === "::1") return true;
    if (h.startsWith("fc") || h.startsWith("fd")) return true; // ULA fc00::/7
    if (h.startsWith("fe8") || h.startsWith("fe9") ||
        h.startsWith("fea") || h.startsWith("feb")) return true; // fe80::/10
    if (h.startsWith("::ffff:")) {
      // IPv4-mapped — recurse on the v4 portion.
      return isPrivateHost(h.slice(7));
    }
    return false;
  }

  return false;
}

/**
 * Allowlist of transport-failure causes the client emits (see
 * fetch::classify_transport_error). The /v1/events endpoint is
 * unauthenticated, so we bound `error_kind_dist` to this fixed set rather
 * than a loose regex — otherwise a buggy/malicious client could spray
 * arbitrary [a-z_]{1,20} keys and inflate a day's KV value.
 */
const ERROR_KINDS = new Set([
  "offline", "dns", "timeout", "reset", "refused",
  "unreachable", "quic", "connect", "other",
]);
function isErrorKind(k) {
  return typeof k === "string" && ERROR_KINDS.has(k);
}

/**
 * Surfaces a fetch can come from: MCP tool names, plus the process modes
 * the client falls back to outside a tool call. Bounded for the same
 * reason as ERROR_KINDS.
 */
const TOOLS = new Set([
  "wick_fetch", "wick_search", "wick_crawl", "wick_map", "wick_download",
  "wick_fetch_to_file", "wick_session", "wick_request_capability",
  "mcp", "api", "cli",
]);
function isTool(t) {
  return typeof t === "string" && TOOLS.has(t);
}

/**
 * Caller country from Cloudflare's own geolocation (request.cf.country):
 * an ISO 3166 alpha-2 code, "T1" for Tor, or "XX" when unknown. Only this
 * code is stored, never the IP it came from.
 */
function callerCountry(request) {
  const c = request.cf && request.cf.country;
  return typeof c === "string" && /^[A-Z][A-Z0-9]$/.test(c) ? c : "XX";
}

/** Copy `{key: count}` entries from `src` into `dst` where `valid(key)`. */
function mergeCounts(dst, src, valid) {
  if (!src || typeof src !== "object") return;
  for (const [k, count] of Object.entries(src)) {
    const n = Number(count);
    if (Number.isFinite(n) && valid(k)) dst[k] = (dst[k] || 0) + n;
  }
}
const isCountry = k => /^[A-Z][A-Z0-9]$/.test(k);

/**
 * Server-side pass of the client's scrub (`scrub` in rust/src/analytics.rs),
 * since old or modified clients may skip it. Same allowlist rules, checked
 * against the same cases (worker/test/scrub-cases.json): a word is kept
 * only if it is a scrub placeholder or looks like plain language; any other
 * word is replaced whole with `[url:host]`, `[email]` or `[redacted]`; then
 * runs of 7+ digits become `[number]`; length is capped. Scrubbing twice
 * gives the same result, so client output passes through unchanged.
 */
const FEEDBACK_MAX_CHARS = 600;
const MAX_PLAIN_WORD = 40;
function looksLikeSecret(word) {
  const alnum = word.replace(/[^A-Za-z0-9]/g, "");
  if (alnum.length < 20) return false;
  const mixed = /\d/.test(alnum) && /[A-Za-z]/.test(alnum);
  const innerUpper = (alnum.slice(1).match(/[A-Z]/g) || []).length;
  return mixed || (innerUpper >= 3 && /[a-z]/.test(alnum));
}
function isDomain(h) {
  return h.includes(".") && /^[A-Za-z0-9.-]+$/.test(h) && /\.[A-Za-z]{2,}$/.test(h);
}
function isPlaceholder(word) {
  const w = word.replace(/^\(+/, "").replace(/[.,;:!?)]+$/, "");
  if (["[number]", "[email]", "[redacted]", "[url]"].includes(w)) return true;
  const m = w.match(/^\[url:(.+)\]$/);
  return !!m && isDomain(m[1]);
}
function isPlainWord(word) {
  if (Array.from(word).length > MAX_PLAIN_WORD || word.includes("://")) return false;
  const core = word.replace(/[.,!?;:)]+$/, "").replace(/^\(+/, "");
  if (core.includes(":")) return false;
  return /^[\p{L}\p{M}\p{N}_'’.,!?;:()\-&+%]*$/u.test(word) && !looksLikeSecret(word);
}
function urlTag(authority) {
  if (authority.includes("@")) return "[url]";
  const host = authority.split(":")[0];
  return isDomain(host) ? `[url:${host}]` : "[url]";
}
function scrubWord(word) {
  if (isPlaceholder(word) || isPlainWord(word)) return word;
  const i = word.indexOf("://");
  if (i >= 0) {
    const after = word.slice(i + 3);
    const m = after.search(/[\/?#]/);
    return urlTag(m < 0 ? after : after.slice(0, m));
  }
  const core = word.replace(/^["'`()[\]{}<>,.;:!?]+|["'`()[\]{}<>,.;:!?]+$/g, "");
  const slash = core.indexOf("/");
  if (slash > 0 && slash < core.length - 1 && isDomain(core.slice(0, slash))) return `[url:${core.slice(0, slash)}]`;
  if (word.includes("@")) return "[email]";
  return "[redacted]";
}
function scrubText(v) {
  if (typeof v !== "string") return null;
  const words = v.split(/\s+/).filter(Boolean).map(scrubWord).join(" ");
  return Array.from(maskDigitRuns(words)).slice(0, FEEDBACK_MAX_CHARS).join("") || null;
}

/**
 * Runs of 7+ digits (with space, -, (, ), ., + between them) become
 * `[number]`, widened to whole words so the output survives a second
 * scrub unchanged. Mirrors `mask_digit_runs` in rust/src/analytics.rs.
 */
function maskDigitRuns(text) {
  const re = /\+?\d[\d ().+-]*/g;
  let out = "", emitted = 0, m;
  while ((m = re.exec(text))) {
    const core = m[0].replace(/[^\d]+$/, "");
    if ((core.match(/\d/g) || []).length < 7) {
      re.lastIndex = m.index + Math.max(core.length, 1);
      continue;
    }
    let start = m.index, end = m.index + core.length;
    while (start > emitted && !/\s/.test(text[start - 1])) start--;
    while (end < text.length && !/\s/.test(text[end])) end++;
    out += text.slice(emitted, start) + "[number]";
    emitted = end;
    re.lastIndex = end;
  }
  return out + text.slice(emitted);
}

/**
 * Ceiling on feedback records per UTC day, across all callers. Each record
 * costs two KV writes (record + counter), and Workers Free allows about
 * 1,000 KV writes a day shared with fetch telemetry, so this stays small.
 * KV has no atomic increment: concurrent requests in the same moment can
 * overshoot it a little. FEEDBACK_LIMITER bounds that per caller; a strict
 * global cap would need a Durable Object.
 */
const FEEDBACK_DAILY_CAP = 100;

export default {
  async fetch(request, env) {
    const url = new URL(request.url);
    const path = url.pathname;

    // CORS headers for browser requests
    const headers = {
      "Access-Control-Allow-Origin": "*",
      "Access-Control-Allow-Methods": "GET",
    };

    // Public: serve the install script (no key needed)
    if (path === "/install-pro.sh") {
      const script = await env.RELEASES.get("install-pro.sh");
      if (!script) {
        return new Response("Install script not found. Contact hello@getwick.dev\n", {
          status: 404,
          headers,
        });
      }
      return new Response(script.body, {
        headers: {
          ...headers,
          "Content-Type": "text/plain; charset=utf-8",
          "Cache-Control": "public, max-age=300",
        },
      });
    }

    // Private client pages (no key needed, just unlisted)
    if (path === "/financial-data") {
      const page = await env.RELEASES.get("financial-data.html");
      if (!page) {
        return new Response("Not found\n", { status: 404, headers });
      }
      return new Response(page.body, {
        headers: { ...headers, "Content-Type": "text/html; charset=utf-8", "Cache-Control": "private, no-cache" },
      });
    }

    // ── Analytics ──────────────────────────────────────────────

    // Usage ping — lightweight, no PII. Tracks installs + active users.
    // POST /ping with { "event": "install|fetch|activate", "version": "0.4.0", "os": "darwin" }
    if (request.method === "POST" && path === "/ping") {
      const body = await request.json().catch(() => ({}));
      const event = body.event || "unknown";
      const version = body.version || "unknown";
      const os = body.os || "unknown";
      const date = new Date().toISOString().split("T")[0]; // YYYY-MM-DD

      // Increment counters in KV
      const key = `ping:${date}:${event}:${os}:${version}`;
      const current = parseInt(await env.SUBSCRIPTIONS.get(key) || "0");
      await env.SUBSCRIPTIONS.put(key, String(current + 1), { expirationTtl: 90 * 86400 });

      // Daily counter per caller country, so active installs can be split
      // by where agents run. Separate prefix keeps the ping: key format.
      if (/^[a-z_]{1,20}$/.test(event)) {
        const countryKey = `pingc:${date}:${event}:${callerCountry(request)}`;
        const cCount = parseInt(await env.SUBSCRIPTIONS.get(countryKey) || "0");
        await env.SUBSCRIPTIONS.put(countryKey, String(cCount + 1), { expirationTtl: 90 * 86400 });
      }

      // Track daily totals
      const totalKey = `ping:${date}:total`;
      const total = parseInt(await env.SUBSCRIPTIONS.get(totalKey) || "0");
      await env.SUBSCRIPTIONS.put(totalKey, String(total + 1), { expirationTtl: 90 * 86400 });

      // For error events, track which domains fail most
      if (event === "error" && body.domain) {
        const domainKey = `errors:${date}:${body.domain}:${body.error || "unknown"}`;
        const domainCount = parseInt(await env.SUBSCRIPTIONS.get(domainKey) || "0");
        await env.SUBSCRIPTIONS.put(domainKey, String(domainCount + 1), { expirationTtl: 90 * 86400 });

        // Append to daily error log (last 100 errors)
        const logKey = `errorlog:${date}`;
        const log = await env.SUBSCRIPTIONS.get(logKey) || "";
        const entry = `${body.domain}|${body.status}|${body.error}|${body.version}|${body.os}|${body.pro}\n`;
        if (log.length < 50000) { // cap at ~50KB per day
          await env.SUBSCRIPTIONS.put(logKey, log + entry, { expirationTtl: 90 * 86400 });
        }
      }

      return new Response("ok\n", { status: 200, headers });
    }

    // ── Per-fetch telemetry (KV-backed) ──────────────────────────
    //
    // POST /v1/events with body:
    // { "host": "nytimes.com", "strategy": "cef", "escalated_from": null|"cronet",
    //   "ok": true, "status": 200, "timing_ms": 1840,
    //   "error_kind": "reset"|"offline"|...,   // only on transport failures
    //   "tool": "wick_fetch"|"cli"|...,         // 0.13+: surface that fetched
    //   "retry": true,                          // 0.13+: host failed < 5 min ago
    //   "version": "0.9.2", "os": "macos" }
    //
    // The stored value also counts the caller's country (country_dist, from
    // Cloudflare's request.cf.country), tools (tool_dist), retries and
    // retries that succeeded. Failures minus successful retries approximates
    // tasks the agent gave up on.
    //
    // Storage model: one KV key per (date, host, strategy) with a merged
    // JSON value `{ fetches, successes, total_ms }`. Each event is a
    // read-modify-write — matches the pattern the existing /ping counters
    // use. Eventually consistent at high concurrency (some increments may
    // be lost if two writes race in the same second), which is fine for
    // telemetry.
    //
    // Cloudflare sees the caller IP at ingest but we don't persist it.
    // Retention: 30 days via KV TTL.
    if (request.method === "POST" && path === "/v1/events") {
      let body;
      try {
        body = await request.json();
      } catch {
        return new Response("bad json\n", { status: 400, headers });
      }

      // Reject absurdly long fields — RFC 1035 max hostname is 253 chars.
      // Also validate character sets so the `:` delimiter in the KV key
      // format `evt:<date>:<host>:<strategy>` can't be injected through
      // either field. Hostnames follow RFC 952/1123 (letters, digits,
      // dots, hyphens). Strategies are ASCII word characters.
      const host = String(body.host || "").slice(0, 253);
      const strategy = String(body.strategy || "").slice(0, 32);
      if (!host || !/^[a-zA-Z0-9.-]+$/.test(host)) {
        return new Response("", { status: 204, headers });
      }
      if (!strategy || !/^[a-zA-Z0-9_-]+$/.test(strategy)) {
        return new Response("", { status: 204, headers });
      }

      // Normalize date to YYYY-MM-DD UTC to keep keys sortable.
      const date = new Date().toISOString().split("T")[0];
      const key = `evt:${date}:${host}:${strategy}`;

      // Load + coerce the current aggregate. Anything that isn't a
      // plain object with numeric fields (null, a string, something
      // manually edited in the KV dashboard) is treated as "start
      // fresh" so we never throw during increment or write back a
      // poisoned value.
      const existing = {
        fetches: 0, successes: 0, total_ms: 0, status_dist: {}, error_kind_dist: {},
        country_dist: {}, tool_dist: {}, retries: 0, retry_successes: 0,
      };
      const existingRaw = await env.SUBSCRIPTIONS.get(key);
      if (existingRaw) {
        try {
          const parsed = JSON.parse(existingRaw);
          if (parsed && typeof parsed === "object") {
            const f = Number(parsed.fetches);
            const s = Number(parsed.successes);
            const t = Number(parsed.total_ms);
            if (Number.isFinite(f)) existing.fetches = f;
            if (Number.isFinite(s)) existing.successes = s;
            if (Number.isFinite(t)) existing.total_ms = t;
            // status_dist is a {statusCode: count} object. Older KV
            // values won't have it; ignore non-objects defensively.
            if (parsed.status_dist && typeof parsed.status_dist === "object") {
              for (const [code, count] of Object.entries(parsed.status_dist)) {
                const n = Number(count);
                // Validate the status-code key too — only keep 0..999
                // ASCII digits, since the key is taken from the
                // payload and we don't want garbage in the bucket.
                if (Number.isFinite(n) && /^\d{1,3}$/.test(code)) {
                  existing.status_dist[code] = n;
                }
              }
            }
            // error_kind_dist: {cause: count} for transport failures. Older
            // KV values won't have it; ignore non-objects defensively.
            if (parsed.error_kind_dist && typeof parsed.error_kind_dist === "object") {
              for (const [kind, count] of Object.entries(parsed.error_kind_dist)) {
                const n = Number(count);
                if (Number.isFinite(n) && isErrorKind(kind)) {
                  existing.error_kind_dist[kind] = n;
                }
              }
            }
            mergeCounts(existing.country_dist, parsed.country_dist, isCountry);
            mergeCounts(existing.tool_dist, parsed.tool_dist, isTool);
            const r = Number(parsed.retries);
            const rs = Number(parsed.retry_successes);
            if (Number.isFinite(r)) existing.retries = r;
            if (Number.isFinite(rs)) existing.retry_successes = rs;
          }
        } catch { /* corrupt JSON — start fresh */ }
      }

      existing.fetches += 1;
      // Strict boolean: the endpoint is unauthenticated, so a truthy
      // check would let `"false"` (a string) count as a success and
      // skew the stats.
      if (body.ok === true) existing.successes += 1;
      const ms = Number(body.timing_ms) || 0;
      if (ms > 0) existing.total_ms += Math.min(ms, 600000); // clamp at 10 min to avoid runaway sums

      // Increment the bucket for this fetch's HTTP status. Treat unknown
      // / missing as 0 ("transport / unknown") so the bucket stays
      // populated even for cronet-transport-error events. Cap statuses
      // to a sane range so a malformed payload can't spam unique keys.
      const rawStatus = Number(body.status);
      const statusBucket = Number.isFinite(rawStatus) && rawStatus >= 0 && rawStatus < 1000
        ? String(Math.trunc(rawStatus))
        : "0";
      existing.status_dist[statusBucket] = (existing.status_dist[statusBucket] || 0) + 1;

      // Record the transport-failure cause when present (offline / dns /
      // reset / refused / timeout / unreachable / quic / connect / other).
      // Only transport-error events carry it; HTTP responses omit it, so
      // this stays empty for them. This is what lets the stats page and the
      // self-improvement harness exclude user-side "offline" failures from
      // "this site is hard" — see analytics::report_transport_error.
      // error_kind means "no HTTP response at all" (transport failure), so it
      // only counts when the event is not ok AND has no status (statusBucket
      // "0"). The endpoint is unauthenticated; this stops a client from
      // attaching a cause to an HTTP error (e.g. 403) and skewing the stats.
      if (body.ok !== true && statusBucket === "0" && isErrorKind(body.error_kind)) {
        existing.error_kind_dist[body.error_kind] =
          (existing.error_kind_dist[body.error_kind] || 0) + 1;
      }

      const country = callerCountry(request);
      existing.country_dist[country] = (existing.country_dist[country] || 0) + 1;
      if (isTool(body.tool)) {
        existing.tool_dist[body.tool] = (existing.tool_dist[body.tool] || 0) + 1;
      }
      if (body.retry === true) {
        existing.retries += 1;
        if (body.ok === true) existing.retry_successes += 1;
      }

      await env.SUBSCRIPTIONS.put(key, JSON.stringify(existing), {
        expirationTtl: 30 * 86400,
      });

      return new Response("", { status: 204, headers });
    }

    // ── Agent feedback ───────────────────────────────────────────
    //
    // POST /v1/feedback with body:
    // { "kind": "capability_request", "task": "...", "tried": "...",
    //   "failure": "...", "host": "example.com", "tool": "wick_request_capability",
    //   "version": "0.13.0", "os": "macos" }
    //
    // Sent by the wick_request_capability MCP tool when an agent can't get
    // what its user needs. Each record is stored once under
    // `fb:<date>:<uuid>` with the caller's country and a 30-day TTL; read
    // them with `wrangler kv key list --prefix fb:`. Text is scrubbed again
    // here in case the client didn't. The endpoint is unauthenticated, so
    // each caller IP is rate limited (FEEDBACK_LIMITER, in memory, never
    // stored) and a global daily cap bounds total writes.
    if (request.method === "POST" && path === "/v1/feedback") {
      if (env.FEEDBACK_LIMITER) {
        const ip = request.headers.get("cf-connecting-ip") || "unknown";
        const { success } = await env.FEEDBACK_LIMITER.limit({ key: ip });
        if (!success) return new Response("rate limited\n", { status: 429, headers });
      }
      let body;
      try {
        body = await request.json();
      } catch {
        return new Response("bad json\n", { status: 400, headers });
      }
      const task = scrubText(body && body.task);
      if (!task) return new Response("task required\n", { status: 400, headers });

      const date = new Date().toISOString().split("T")[0];
      const countKey = `fbcount:${date}`;
      const count = parseInt(await env.SUBSCRIPTIONS.get(countKey) || "0");
      if (count >= FEEDBACK_DAILY_CAP) return new Response("", { status: 204, headers });
      // Fail closed: if the counter can't be written (KV allows about one
      // write per second to a key), drop the record rather than store it
      // uncounted.
      try {
        await env.SUBSCRIPTIONS.put(countKey, String(count + 1), { expirationTtl: 2 * 86400 });
      } catch {
        return new Response("", { status: 204, headers });
      }

      const host = typeof body.host === "string" && /^[a-zA-Z0-9.-]{1,253}$/.test(body.host)
        ? body.host : null;
      const record = {
        kind: body.kind === "capability_request" ? body.kind : "other",
        task,
        tried: scrubText(body.tried),
        failure: scrubText(body.failure),
        host,
        tool: isTool(body.tool) ? body.tool : null,
        country: callerCountry(request),
        version: typeof body.version === "string" ? body.version.slice(0, 20) : null,
        os: typeof body.os === "string" ? body.os.slice(0, 20) : null,
        at: new Date().toISOString(),
      };
      await env.SUBSCRIPTIONS.put(`fb:${date}:${crypto.randomUUID()}`, JSON.stringify(record), {
        expirationTtl: 30 * 86400,
      });
      return new Response("", { status: 204, headers });
    }

    // ── Public stats summary ─────────────────────────────────────
    //
    // GET /v1/stats/summary — 7-day aggregate of the KV event counters,
    // cached 5 minutes. Public, no auth. Refreshing on a cache miss
    // scans across the 7-day window with a single global cap of 5000
    // KV keys, so keep the cache honest.
    if (request.method === "GET" && path === "/v1/stats/summary") {
      // v2 added per-row status_dist; bumping the key ensures the
      // first request after deploy rebuilds the cached payload in the
      // new shape instead of serving stale v1 JSON for up to 5 minutes.
      const cacheKey = "stats:summary:v4";
      const cached = await env.SUBSCRIPTIONS.get(cacheKey);
      if (cached) {
        return new Response(cached, {
          headers: {
            ...headers,
            "Content-Type": "application/json",
            "Cache-Control": "public, max-age=300",
          },
        });
      }

      // Aggregate across the last 7 days.
      const now = new Date();
      const dates = [];
      for (let i = 0; i < 7; i++) {
        const d = new Date(now.getTime() - i * 86400_000);
        dates.push(d.toISOString().split("T")[0]);
      }

      // Keep the accumulation small: one entry per (host, strategy).
      const agg = new Map(); // key: `${host}|${strategy}` → { host, strategy, fetches, successes, total_ms }

      // Single global cap across all 7 days — prevents a pathological
      // 7 * per_day_cap worst case. Each scanned key is a read-modify-
      // write aggregate (already de-duped per host+strategy+day at
      // ingest), so 5000 keys is comfortably more than we expect
      // and leaves headroom under Workers CPU limits.
      const SCAN_CAP = 5000;
      let scanned = 0;
      outer:
      for (const date of dates) {
        let cursor = undefined;
        do {
          const list = await env.SUBSCRIPTIONS.list({
            prefix: `evt:${date}:`,
            limit: 1000,
            cursor,
          });
          for (const k of list.keys) {
            if (scanned >= SCAN_CAP) {
              // Stop paginating further — not just the inner loop.
              break outer;
            }
            scanned++;
            const raw = await env.SUBSCRIPTIONS.get(k.name);
            if (!raw) continue;
            let v;
            try { v = JSON.parse(raw); } catch { continue; }
            // Key format: evt:YYYY-MM-DD:host:strategy
            const rest = k.name.slice(`evt:${date}:`.length);
            const lastColon = rest.lastIndexOf(":");
            if (lastColon < 0) continue;
            const host = rest.slice(0, lastColon);
            const strategy = rest.slice(lastColon + 1);
            const aggKey = `${host}|${strategy}`;
            const cur = agg.get(aggKey) || {
              host, strategy, fetches: 0, successes: 0, total_ms: 0, status_dist: {}, error_kind_dist: {},
              tool_dist: {}, retries: 0, retry_successes: 0,
            };
            // Coerce each field via Number() and ignore non-finite
            // values — a stringly-typed stored value (`"1"`) would
            // otherwise turn `cur.fetches` into a string and break
            // arithmetic + sorting downstream.
            const fetches = Number(v.fetches);
            const successes = Number(v.successes);
            const totalMs = Number(v.total_ms);
            if (Number.isFinite(fetches)) cur.fetches += fetches;
            if (Number.isFinite(successes)) cur.successes += successes;
            if (Number.isFinite(totalMs)) cur.total_ms += totalMs;
            // Merge status_dist across days. Older daily values may
            // not have it (pre-v2 ingest); just skip those.
            if (v.status_dist && typeof v.status_dist === "object") {
              for (const [code, count] of Object.entries(v.status_dist)) {
                const n = Number(count);
                if (Number.isFinite(n) && /^\d{1,3}$/.test(code)) {
                  cur.status_dist[code] = (cur.status_dist[code] || 0) + n;
                }
              }
            }
            // Merge transport-failure causes across days (same shape as
            // status_dist). Pre-error_kind events simply don't contribute.
            if (v.error_kind_dist && typeof v.error_kind_dist === "object") {
              for (const [kind, count] of Object.entries(v.error_kind_dist)) {
                const n = Number(count);
                if (Number.isFinite(n) && isErrorKind(kind)) {
                  cur.error_kind_dist[kind] = (cur.error_kind_dist[kind] || 0) + n;
                }
              }
            }
            mergeCounts(cur.tool_dist, v.tool_dist, isTool);
            const vr = Number(v.retries);
            const vrs = Number(v.retry_successes);
            if (Number.isFinite(vr)) cur.retries += vr;
            if (Number.isFinite(vrs)) cur.retry_successes += vrs;
            agg.set(aggKey, cur);
          }
          cursor = list.list_complete ? undefined : list.cursor;
        } while (cursor);
      }

      const rows = [...agg.values()]
        .map(r => ({
          host: r.host,
          strategy: r.strategy,
          fetches: r.fetches,
          successes: r.successes,
          success_rate: r.fetches > 0 ? r.successes / r.fetches : 0,
          // No real p50 without raw samples — use mean_ms as an approximation.
          p50_ms: r.fetches > 0 ? Math.round(r.total_ms / r.fetches) : 0,
          // Per-status counts so the page can show "404×5, 502×2" instead
          // of just a red bar. Pre-v2 events won't contribute here, so
          // historical rows may sum to less than `fetches`.
          status_dist: r.status_dist || {},
          // Per-cause counts for transport failures, so a row reading 0%
          // success can be split into "offline×20" (user's network — ignore)
          // vs "reset×20" (the site is actively blocking — a real signal).
          error_kind_dist: r.error_kind_dist || {},
          // country_dist stays out of this public summary: per-host
          // country counts can single out one user. Read it from KV.
          tool_dist: r.tool_dist,
          retries: r.retries,
          retry_successes: r.retry_successes,
        }))
        .sort((a, b) => b.fetches - a.fetches)
        .slice(0, 500);

      const payload = JSON.stringify({
        generated_at: new Date().toISOString(),
        window_days: 7,
        rows,
      });

      await env.SUBSCRIPTIONS.put(cacheKey, payload, { expirationTtl: 300 });

      return new Response(payload, {
        headers: {
          ...headers,
          "Content-Type": "application/json",
          "Cache-Control": "public, max-age=300",
        },
      });
    }

    // ── Curated site-rules (the evolving "known behaviors" list) ─────────
    //
    // GET  /v1/site-rules       → public, cached 1h. Clients refresh this
    //                             daily into <wick-home>/site-rules.json,
    //                             which overlays the binary's bundled seed
    //                             (see rust/src/site_rules.rs). This is how
    //                             the list "constantly evolves" without a
    //                             reinstall.
    // POST /v1/site-rules/:key  → publisher-gated. The self-improvement loop
    //                             publishes a merged rules doc here (seed ∪
    //                             measured ∪ curated). Body is the full
    //                             { version, rules: { host: {...} } } doc.
    //                             `:key` must equal the SITE_RULES_PUBLISH_KEY
    //                             secret — a dedicated single-purpose token,
    //                             not an API_KEYS customer key — since
    //                             publishing rewrites what every client fetches.
    if (request.method === "GET" && path === "/v1/site-rules") {
      const stored = await env.SUBSCRIPTIONS.get("site-rules:published");
      const body = stored || JSON.stringify({ version: 1, rules: {} });
      return new Response(body, {
        headers: {
          ...headers,
          "Content-Type": "application/json",
          "Cache-Control": "public, max-age=3600",
        },
      });
    }

    const rulesPubMatch = path.match(/^\/v1\/site-rules\/([^/]+)$/);
    if (request.method === "POST" && rulesPubMatch) {
      const pubKey = rulesPubMatch[1];
      // Publishing rewrites the rules every client fetches, so it's gated on a
      // dedicated single-purpose secret (SITE_RULES_PUBLISH_KEY) — deliberately
      // NOT the API_KEYS customer pool. That way no read/proxy customer key can
      // publish, and rotating/setting the publisher token never risks the
      // (unreadable, must-be-supplied-whole) API_KEYS secret.
      if (!env.SITE_RULES_PUBLISH_KEY) {
        return new Response("Publishing not configured\n", { status: 503, headers });
      }
      if (pubKey !== env.SITE_RULES_PUBLISH_KEY) {
        return new Response("Invalid publisher key\n", { status: 403, headers });
      }
      let doc;
      try { doc = await request.json(); } catch {
        return new Response("bad json\n", { status: 400, headers });
      }
      if (!doc || typeof doc !== "object" || typeof doc.rules !== "object"
          || doc.rules === null || Array.isArray(doc.rules)) {
        // typeof [] === "object", so arrays must be rejected explicitly —
        // rules must be a host→rule map, not a list.
        return new Response("expected { rules: { host: {...} } }\n", { status: 400, headers });
      }
      const hostCount = Object.keys(doc.rules).length;
      if (hostCount > 5000) {
        return new Response("too many rules (max 5000)\n", { status: 400, headers });
      }
      // Store verbatim (clients tolerate unknown fields; an unrecognized
      // `render` is treated as "no opinion" client-side). Stamp a server
      // receive time so clients/operators can see staleness.
      const toStore = JSON.stringify({
        version: Number(doc.version) || 1,
        published_at: new Date().toISOString(),
        rules: doc.rules,
      });
      if (toStore.length > 1_000_000) {
        return new Response("payload too large (max 1MB)\n", { status: 413, headers });
      }
      await env.SUBSCRIPTIONS.put("site-rules:published", toStore);
      console.log(JSON.stringify({
        event: "site_rules_publish",
        hosts: hostCount,
        timestamp: new Date().toISOString(),
      }));
      return new Response(JSON.stringify({ ok: true, hosts: hostCount }), {
        status: 200,
        headers: { ...headers, "Content-Type": "application/json" },
      });
    }

    // Analytics dashboard — simple KV-based metrics
    // GET /analytics/:key (requires API key)
    if (path.match(/^\/analytics\/([^/]+)$/)) {
      const analyticsKey = path.match(/^\/analytics\/([^/]+)$/)[1];

      // Validate key
      let keys;
      try { keys = JSON.parse(env.API_KEYS || "{}"); } catch { keys = {}; }
      const sub = await env.SUBSCRIPTIONS.get(`key:${analyticsKey}`, "json");
      if ((!keys[analyticsKey] || !keys[analyticsKey].active) && !sub) {
        return new Response("Unauthorized\n", { status: 403, headers });
      }

      // Get last 7 days of data
      // Check all known OS/version combos since pings store as ping:date:event:os:version
      const osVersions = ["macos:0.5.0", "darwin:0.5.0", "macos:unknown", "darwin:unknown", "linux:0.5.0", "linux:unknown"];
      const days = [];
      for (let i = 0; i < 7; i++) {
        const d = new Date(Date.now() - i * 86400000).toISOString().split("T")[0];
        const total = parseInt(await env.SUBSCRIPTIONS.get(`ping:${d}:total`) || "0");
        let installs = 0, fetches = 0;
        for (const ov of osVersions) {
          installs += parseInt(await env.SUBSCRIPTIONS.get(`ping:${d}:install:${ov}`) || "0");
          fetches += parseInt(await env.SUBSCRIPTIONS.get(`ping:${d}:fetch:${ov}`) || "0");
        }
        days.push({ date: d, total, installs, fetches });
      }

      return new Response(JSON.stringify({ days }, null, 2), {
        headers: { ...headers, "Content-Type": "application/json" },
      });
    }

    // Public: serve macOS install script
    if (path === "/install-pro-mac.sh") {
      const script = await env.RELEASES.get("install-pro-mac.sh");
      if (!script) {
        return new Response("macOS install script not found\n", { status: 404, headers });
      }
      return new Response(script.body, {
        headers: { ...headers, "Content-Type": "text/plain; charset=utf-8", "Cache-Control": "public, max-age=300" },
      });
    }

    // Public: serve wick-tunnel script (no key needed)
    if (path === "/wick-tunnel") {
      const script = await env.RELEASES.get("wick-tunnel");
      if (!script) {
        return new Response("wick-tunnel not found. Contact hello@getwick.dev\n", {
          status: 404,
          headers,
        });
      }
      return new Response(script.body, {
        headers: {
          ...headers,
          "Content-Type": "text/plain; charset=utf-8",
          "Cache-Control": "public, max-age=300",
        },
      });
    }

    // Protected: CAPTCHA solve proxy — POST /solve/:key
    // Proxies to CapSolver using our API key. Customer never sees it.
    if (request.method === "POST" && path.match(/^\/solve\/([^/]+)$/)) {
      const solveKey = path.match(/^\/solve\/([^/]+)$/)[1];

      let keys;
      try { keys = JSON.parse(env.API_KEYS || "{}"); } catch {
        return new Response("Server error\n", { status: 500, headers });
      }
      if (!keys[solveKey] || !keys[solveKey].active) {
        return new Response("Invalid API key\n", { status: 403, headers });
      }

      if (!env.CAPSOLVER_API_KEY) {
        return new Response("CAPTCHA solving not configured\n", { status: 503, headers });
      }

      // Read the request body
      const body = await request.json().catch(() => null);
      if (!body) {
        return new Response("Missing request body\n", { status: 400, headers });
      }
      if (!body.task && !body.taskId) {
        return new Response("Missing task or taskId in request body\n", { status: 400, headers });
      }

      const action = body.action || "createTask";
      const capsolverUrl = `https://api.capsolver.com/${action}`;

      // Build CapSolver request — inject our API key
      const capBody = { clientKey: env.CAPSOLVER_API_KEY };
      if (action === "createTask") {
        capBody.task = body.task;
      } else if (action === "getTaskResult") {
        capBody.taskId = body.taskId;
      }

      const capResp = await fetch(capsolverUrl, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(capBody),
      });

      const capResult = await capResp.text();

      console.log(JSON.stringify({
        event: "captcha_solve",
        customer: keys[solveKey].customer,
        action,
        timestamp: new Date().toISOString(),
      }));

      return new Response(capResult, {
        status: capResp.status,
        headers: { ...headers, "Content-Type": "application/json" },
      });
    }

    // Protected: geo-proxy — fetch URLs from Cloudflare's edge network.
    // Bypasses geo-restrictions by originating from Cloudflare's regional PoPs
    // (Tokyo, Taipei, etc.) instead of the customer's server location.
    // POST /proxy/:key with JSON body { "url": "https://..." }
    if (request.method === "POST" && path.match(/^\/proxy\/([^/]+)$/)) {
      const proxyKey = path.match(/^\/proxy\/([^/]+)$/)[1];

      let keys;
      try { keys = JSON.parse(env.API_KEYS || "{}"); } catch {
        return new Response("Server error\n", { status: 500, headers });
      }
      if (!keys[proxyKey] || !keys[proxyKey].active) {
        return new Response("Invalid API key\n", { status: 403, headers });
      }

      const body = await request.json().catch(() => null);
      if (!body || !body.url) {
        return new Response("Missing url in request body\n", { status: 400, headers });
      }

      // Validate URL: only http/https, reject private/loopback/
      // link-local IP literals so a paid key can't be used to probe
      // our internal networks (SSRF). DNS-based targets are not
      // resolved here — this catches only IP-literal URLs.
      let targetUrl;
      try {
        targetUrl = new URL(body.url);
        if (!["http:", "https:"].includes(targetUrl.protocol)) {
          return new Response("Only http/https URLs\n", { status: 400, headers });
        }
        if (isPrivateHost(targetUrl.hostname)) {
          return new Response("Target host not allowed\n", { status: 400, headers });
        }
      } catch {
        return new Response("Invalid URL\n", { status: 400, headers });
      }

      // Fetch from Cloudflare's edge — exits from nearest PoP to target
      const proxyHeaders = {
        "User-Agent": body.userAgent || "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/143.0.0.0 Safari/537.36",
        "Accept": "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8",
        "Accept-Language": body.acceptLanguage || "en-US,en;q=0.9",
      };

      // Forward custom headers if provided
      if (body.headers) {
        for (const [k, v] of Object.entries(body.headers)) {
          proxyHeaders[k] = v;
        }
      }

      try {
        const resp = await fetch(body.url, {
          headers: proxyHeaders,
          redirect: "follow",
          cf: {
            // Hint Cloudflare to use a PoP near the target
            cacheTtl: 0,
            cacheEverything: false,
          },
        });

        const contentType = resp.headers.get("content-type") || "text/html";
        const responseBody = await resp.text();

        // Log `host + path` rather than the full URL so signed-URL
        // tokens and other query-string secrets don't leak into
        // worker logs. `targetUrl` is the parsed URL from above.
        console.log(JSON.stringify({
          event: "proxy",
          customer: keys[proxyKey].customer,
          host: targetUrl.hostname,
          path: targetUrl.pathname,
          status: resp.status,
          bytes: responseBody.length,
          timestamp: new Date().toISOString(),
        }));

        // Don't reflect the user-controlled URL back in a response
        // header — newlines in `body.url` can trigger invalid-header
        // errors, and query-string secrets can leak into any tooling
        // that records response headers.
        return new Response(responseBody, {
          status: resp.status,
          headers: {
            ...headers,
            "Content-Type": contentType,
            "X-Proxy-Status": resp.status.toString(),
          },
        });
      } catch (e) {
        return new Response(`Proxy fetch failed: ${e.message}\n`, {
          status: 502,
          headers,
        });
      }
    }

    // Protected: /releases/:key/:filename
    const releaseMatch = path.match(/^\/releases\/([^/]+)\/(.+)$/);
    if (!releaseMatch) {
      return new Response("Not found\n", { status: 404, headers });
    }

    const [, apiKey, filename] = releaseMatch;

    // Validate API key
    let keys;
    try {
      keys = JSON.parse(env.API_KEYS || "{}");
    } catch {
      return new Response("Server configuration error\n", { status: 500, headers });
    }

    const keyInfo = keys[apiKey];
    if (!keyInfo || !keyInfo.active) {
      return new Response(
        "Invalid or expired API key.\n" +
        "Contact hello@getwick.dev for Wick Pro access.\n",
        { status: 403, headers }
      );
    }

    // Validate filename (prevent path traversal)
    const allowedFiles = [
      "wick-pro-linux-x86_64.tar.gz",
      "wick-pro-linux-aarch64.tar.gz",
      "cef-runtime-linux-x86_64.tar.bz2",
      "cef-runtime-linux-aarch64.tar.bz2",
    ];
    if (!allowedFiles.includes(filename)) {
      return new Response("File not found\n", { status: 404, headers });
    }

    // Fetch from R2
    const object = await env.RELEASES.get(filename);
    if (!object) {
      return new Response(
        "Release not available yet. Contact hello@getwick.dev\n",
        { status: 404, headers }
      );
    }

    // Log download for tracking. Caller IP is intentionally omitted —
    // the repo's privacy posture is to not persist IP addresses as a
    // data point anywhere (worker logs included, since they're
    // retained/exported).
    console.log(JSON.stringify({
      event: "download",
      customer: keyInfo.customer,
      file: filename,
      timestamp: new Date().toISOString(),
    }));

    // Pick Content-Type by extension so `.tar.bz2` files aren't
    // served as `application/gzip`. Fall back to octet-stream for
    // anything we don't recognize.
    let contentType = "application/octet-stream";
    if (filename.endsWith(".tar.gz") || filename.endsWith(".tgz")) {
      contentType = "application/gzip";
    } else if (filename.endsWith(".tar.bz2")) {
      contentType = "application/x-bzip2";
    }

    return new Response(object.body, {
      headers: {
        ...headers,
        "Content-Type": contentType,
        "Content-Disposition": `attachment; filename="${filename}"`,
        "Cache-Control": "private, no-cache",
      },
    });
  },
};
