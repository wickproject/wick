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

// ── Helpers ─────────────────────────────────────────────────

/**
 * Verify a Stripe webhook signature header.
 * Stripe-Signature is formatted `t=<timestamp>,v1=<hex>`; we reconstruct
 * the signed payload (`<t>.<raw body>`), HMAC-SHA256 it with the webhook
 * secret, and constant-time compare to the `v1` value. A 5-minute
 * timestamp tolerance rejects replays. Returns true only on a match.
 * See https://docs.stripe.com/webhooks/signatures.
 */
async function verifyStripeSignature(payload, sigHeader, secret) {
  if (!sigHeader || !secret) return false;
  const parts = {};
  for (const p of sigHeader.split(",")) {
    const eq = p.indexOf("=");
    if (eq <= 0) continue;
    const k = p.slice(0, eq).trim();
    const v = p.slice(eq + 1).trim();
    // Keep only the first value per key — matches Stripe's v1/v0 format.
    if (!(k in parts)) parts[k] = v;
  }
  const t = parts.t;
  const v1 = parts.v1;
  if (!t || !v1) return false;
  const ts = Number(t);
  if (!Number.isFinite(ts)) return false;
  const age = Math.abs(Date.now() / 1000 - ts);
  if (age > 300) return false;

  const enc = new TextEncoder();
  const key = await crypto.subtle.importKey(
    "raw", enc.encode(secret),
    { name: "HMAC", hash: "SHA-256" }, false, ["sign"],
  );
  const sig = await crypto.subtle.sign("HMAC", key, enc.encode(`${t}.${payload}`));
  const expected = [...new Uint8Array(sig)]
    .map(b => b.toString(16).padStart(2, "0")).join("");

  if (expected.length !== v1.length) return false;
  let diff = 0;
  for (let i = 0; i < expected.length; i++) {
    diff |= expected.charCodeAt(i) ^ v1.charCodeAt(i);
  }
  return diff === 0;
}

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

      // Verify the Stripe-Signature header before trusting the event —
      // otherwise anyone who can reach this endpoint can forge a
      // `checkout.session.completed` event and mint an API key.
      if (!env.STRIPE_WEBHOOK_SECRET) {
        return new Response("Webhook not configured\n", { status: 503, headers });
      }
      const sig = request.headers.get("Stripe-Signature");
      const ok = await verifyStripeSignature(payload, sig, env.STRIPE_WEBHOOK_SECRET);
      if (!ok) {
        return new Response("Invalid signature\n", { status: 400, headers });
      }

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

          // Update session status. Keep a TTL on the active record so
          // `session:` entries don't grow unbounded in KV — the key
          // itself lives in `key:<apiKey>` without a TTL, and the
          // client has ~24h to poll this session for the API key.
          await env.SUBSCRIPTIONS.put(`session:${wickSession}`, JSON.stringify({
            status: "active",
            key,
            email,
          }), { expirationTtl: 60 * 60 * 24 });

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
      // Don't server-side-interpolate the query parameter into the
      // inline script — read it on the client instead. That avoids
      // any XSS risk from malformed `?session=...` values ending up
      // inside a `<script>` string literal.
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
  const sid = new URLSearchParams(window.location.search).get('session') || '';
  if (!sid) { document.getElementById('status').textContent = 'Missing session.'; return; }
  for (let i = 0; i < 30; i++) {
    const r = await fetch('/pro/status/' + encodeURIComponent(sid));
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
