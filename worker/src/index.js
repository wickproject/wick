/**
 * Wick — Release Server + Telemetry
 *
 * Public routes:
 *   GET  /install-pro.sh, /install-pro-mac.sh, /wick-tunnel
 *   GET  /financial-data
 *   POST /ping                   → legacy daily usage ping
 *   POST /v1/events              → per-fetch telemetry
 *   GET  /v1/stats/summary       → public 7-day stats aggregate
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
    //   "version": "0.9.2", "os": "macos" }
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

      const existingRaw = await env.SUBSCRIPTIONS.get(key);
      let existing = { fetches: 0, successes: 0, total_ms: 0 };
      if (existingRaw) {
        try {
          existing = JSON.parse(existingRaw);
        } catch {
          // Corrupted KV value — start fresh rather than 500 ingestion.
          existing = { fetches: 0, successes: 0, total_ms: 0 };
        }
      }

      existing.fetches += 1;
      if (body.ok) existing.successes += 1;
      const ms = Number(body.timing_ms) || 0;
      if (ms > 0) existing.total_ms += Math.min(ms, 600000); // clamp at 10 min to avoid runaway sums

      await env.SUBSCRIPTIONS.put(key, JSON.stringify(existing), {
        expirationTtl: 30 * 86400,
      });

      return new Response("", { status: 204, headers });
    }

    // ── Public stats summary ─────────────────────────────────────
    //
    // GET /v1/stats/summary — 7-day aggregate of the KV event counters,
    // cached 5 minutes. Public, no auth. Refreshing on a cache miss
    // scans up to 7*1000 KV keys so keep the cache honest.
    if (request.method === "GET" && path === "/v1/stats/summary") {
      const cacheKey = "stats:summary:v1";
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
              host, strategy, fetches: 0, successes: 0, total_ms: 0,
            };
            cur.fetches += v.fetches || 0;
            cur.successes += v.successes || 0;
            cur.total_ms += v.total_ms || 0;
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

        console.log(JSON.stringify({
          event: "proxy",
          customer: keys[proxyKey].customer,
          url: body.url,
          status: resp.status,
          bytes: responseBody.length,
          timestamp: new Date().toISOString(),
        }));

        return new Response(responseBody, {
          status: resp.status,
          headers: {
            ...headers,
            "Content-Type": contentType,
            "X-Proxy-Status": resp.status.toString(),
            "X-Proxy-Url": body.url,
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

    return new Response(object.body, {
      headers: {
        ...headers,
        "Content-Type": "application/gzip",
        "Content-Disposition": `attachment; filename="${filename}"`,
        "Cache-Control": "private, no-cache",
      },
    });
  },
};
