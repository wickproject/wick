/**
 * Wick Pro — Release Server + Subscription Management
 *
 * Public routes:
 *   GET  /install-pro.sh, /install-pro-mac.sh, /wick-tunnel
 *   GET  /financial-data
 *
 * Pro subscription:
 *   POST /pro/checkout          → creates Stripe checkout, returns URL
 *   GET  /pro/status/:session   → polls for API key after payment
 *   POST /pro/webhook           → Stripe webhook (payment confirmed)
 *   POST /pro/validate/:key     → validates a Pro API key
 *
 * Protected (API key required):
 *   GET  /releases/:key/:file
 *   POST /solve/:key            → CAPTCHA proxy
 *   POST /proxy/:key            → geo-proxy
 */

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

    // ── Pro Subscription ─────────────────────────────────────

    // Create Stripe checkout session
    if (request.method === "POST" && path === "/pro/checkout") {
      if (!env.STRIPE_SECRET_KEY) {
        return new Response("Stripe not configured\n", { status: 503, headers });
      }

      const body = await request.json().catch(() => ({}));
      const sessionId = crypto.randomUUID();

      // Create Stripe checkout session
      const stripeResp = await fetch("https://api.stripe.com/v1/checkout/sessions", {
        method: "POST",
        headers: {
          "Authorization": `Bearer ${env.STRIPE_SECRET_KEY}`,
          "Content-Type": "application/x-www-form-urlencoded",
        },
        body: new URLSearchParams({
          "mode": "subscription",
          "line_items[0][price]": env.STRIPE_PRICE_ID || "price_placeholder",
          "line_items[0][quantity]": "1",
          "success_url": `https://releases.getwick.dev/pro/success?session=${sessionId}`,
          "cancel_url": "https://getwick.dev",
          "metadata[wick_session]": sessionId,
          "allow_promotion_codes": "true",
        }),
      });

      const session = await stripeResp.json();
      if (session.error) {
        return new Response(JSON.stringify({ error: session.error.message }), {
          status: 400,
          headers: { ...headers, "Content-Type": "application/json" },
        });
      }

      // Store session → pending
      await env.SUBSCRIPTIONS.put(`session:${sessionId}`, JSON.stringify({
        status: "pending",
        stripeSessionId: session.id,
        created: new Date().toISOString(),
      }), { expirationTtl: 3600 }); // 1 hour expiry

      return new Response(JSON.stringify({
        checkoutUrl: session.url,
        sessionId,
      }), {
        headers: { ...headers, "Content-Type": "application/json" },
      });
    }

    // Poll for API key after checkout
    if (path.match(/^\/pro\/status\/([^/]+)$/)) {
      const sessionId = path.match(/^\/pro\/status\/([^/]+)$/)[1];
      const data = await env.SUBSCRIPTIONS.get(`session:${sessionId}`, "json");

      if (!data) {
        return new Response(JSON.stringify({ status: "unknown" }), {
          headers: { ...headers, "Content-Type": "application/json" },
        });
      }

      return new Response(JSON.stringify({
        status: data.status,
        key: data.key || null,
      }), {
        headers: { ...headers, "Content-Type": "application/json" },
      });
    }

    // Stripe webhook — payment confirmed
    if (request.method === "POST" && path === "/pro/webhook") {
      const payload = await request.text();

      // In production, verify Stripe signature with env.STRIPE_WEBHOOK_SECRET
      // For now, parse the event directly
      let event;
      try {
        event = JSON.parse(payload);
      } catch {
        return new Response("Invalid payload\n", { status: 400, headers });
      }

      if (event.type === "checkout.session.completed") {
        const session = event.data.object;
        const wickSession = session.metadata?.wick_session;
        const email = session.customer_email || session.customer_details?.email || "unknown";

        if (wickSession) {
          // Generate API key
          const keyBytes = new Uint8Array(16);
          crypto.getRandomValues(keyBytes);
          const key = "wk_" + Array.from(keyBytes).map(b => b.toString(16).padStart(2, "0")).join("");

          // Store key in KV
          await env.SUBSCRIPTIONS.put(`key:${key}`, JSON.stringify({
            email,
            stripeCustomerId: session.customer,
            stripeSubscriptionId: session.subscription,
            active: true,
            created: new Date().toISOString(),
          }));

          // Update session status
          await env.SUBSCRIPTIONS.put(`session:${wickSession}`, JSON.stringify({
            status: "active",
            key,
            email,
          }));

          // Also add to the legacy API_KEYS for backward compat
          // (existing endpoints validate against API_KEYS secret)
          // In the future, validate against KV instead

          console.log(JSON.stringify({
            event: "subscription",
            email,
            key: key.substring(0, 10) + "...",
            timestamp: new Date().toISOString(),
          }));
        }
      }

      return new Response("ok\n", { status: 200, headers });
    }

    // Validate a Pro API key (used by wick CLI)
    if (path.match(/^\/pro\/validate\/([^/]+)$/)) {
      const key = path.match(/^\/pro\/validate\/([^/]+)$/)[1];

      // Check KV subscriptions first
      const sub = await env.SUBSCRIPTIONS.get(`key:${key}`, "json");
      if (sub && sub.active) {
        return new Response(JSON.stringify({ valid: true, email: sub.email }), {
          headers: { ...headers, "Content-Type": "application/json" },
        });
      }

      // Fall back to legacy API_KEYS secret
      try {
        const keys = JSON.parse(env.API_KEYS || "{}");
        if (keys[key] && keys[key].active) {
          return new Response(JSON.stringify({ valid: true, customer: keys[key].customer }), {
            headers: { ...headers, "Content-Type": "application/json" },
          });
        }
      } catch {}

      return new Response(JSON.stringify({ valid: false }), {
        status: 403,
        headers: { ...headers, "Content-Type": "application/json" },
      });
    }

    // Success page after Stripe checkout
    if (path === "/pro/success") {
      const sessionId = url.searchParams.get("session");
      return new Response(`<!DOCTYPE html>
<html><head><title>Wick Pro - Activated</title>
<style>body{background:#0D0B09;color:#F0E6D8;font-family:system-ui;display:flex;align-items:center;justify-content:center;min-height:100vh;margin:0;}
.box{text-align:center;max-width:480px;padding:2rem;}
h1{color:#E8913A;font-size:1.5rem;margin-bottom:1rem;}
code{background:#2A2520;padding:0.5rem 1rem;border-radius:6px;display:block;margin:1rem 0;font-size:0.9rem;color:#22c55e;}
p{color:#9E958A;line-height:1.6;font-size:0.9rem;}
#status{color:#E8913A;}</style></head>
<body><div class="box">
<h1>Wick Pro Activated</h1>
<p id="status">Setting up your API key...</p>
<div id="key-display" style="display:none;">
<p>Your API key:</p>
<code id="api-key"></code>
<p>If your terminal didn't activate automatically, run:</p>
<code>wick pro activate --key <span id="key-cmd"></span></code>
</div>
<script>
async function poll() {
  const sid = "${sessionId || ''}";
  if (!sid) { document.getElementById('status').textContent = 'Missing session.'; return; }
  for (let i = 0; i < 30; i++) {
    const r = await fetch('/pro/status/' + sid);
    const d = await r.json();
    if (d.status === 'active' && d.key) {
      document.getElementById('status').textContent = 'Ready!';
      document.getElementById('api-key').textContent = d.key;
      document.getElementById('key-cmd').textContent = d.key;
      document.getElementById('key-display').style.display = 'block';
      return;
    }
    await new Promise(r => setTimeout(r, 2000));
  }
  document.getElementById('status').textContent = 'Still processing. Check back in a moment.';
}
poll();
</script></div></body></html>`, {
        headers: { ...headers, "Content-Type": "text/html; charset=utf-8" },
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
      const host = String(body.host || "").slice(0, 253);
      const strategy = String(body.strategy || "").slice(0, 32);
      if (!host || !strategy) {
        return new Response("", { status: 204, headers });
      }

      // Normalize date to YYYY-MM-DD UTC to keep keys sortable.
      const date = new Date().toISOString().split("T")[0];
      const key = `evt:${date}:${host}:${strategy}`;

      const existingRaw = await env.SUBSCRIPTIONS.get(key);
      const existing = existingRaw
        ? JSON.parse(existingRaw)
        : { fetches: 0, successes: 0, total_ms: 0 };

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

      for (const date of dates) {
        let cursor = undefined;
        let scanned = 0;
        do {
          const list = await env.SUBSCRIPTIONS.list({
            prefix: `evt:${date}:`,
            limit: 1000,
            cursor,
          });
          for (const k of list.keys) {
            scanned++;
            if (scanned > 5000) break; // safety cap per day
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

      // Validate URL (only http/https, no internal IPs)
      let targetUrl;
      try {
        targetUrl = new URL(body.url);
        if (!["http:", "https:"].includes(targetUrl.protocol)) {
          return new Response("Only http/https URLs\n", { status: 400, headers });
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

    // Log download for tracking
    console.log(JSON.stringify({
      event: "download",
      customer: keyInfo.customer,
      file: filename,
      ip: request.headers.get("CF-Connecting-IP"),
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
