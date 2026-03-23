# Wick: Browser-Grade Web Access for AI Agents

> *The part of the lantern that actually burns.*

**Status:** Draft
**Authors:** Myles Horton, Adam Fisk
**Date:** 2026-03-19

---

## Table of Contents

1. [Vision](#vision)
2. [Problem](#problem)
3. [Solution](#solution)
4. [Architecture](#architecture)
5. [Technical Design](#technical-design)
6. [MCP Interface](#mcp-interface)
7. [CAPTCHA Strategy](#captcha-strategy)
8. [Distribution](#distribution)
9. [Business Model](#business-model)
10. [Competitive Landscape](#competitive-landscape)
11. [Legal Analysis](#legal-analysis)
12. [Risks](#risks)
13. [Roadmap](#roadmap)

---

## Vision

AI agents are increasingly capable but effectively blind on the web. When Claude Code, Cursor, or any MCP-compatible agent tries to fetch a webpage, it sends a request that screams "I am a bot" — wrong TLS fingerprint, wrong HTTP/2 framing, datacenter IP, no cookies, no JavaScript execution. The result: blocked by Cloudflare, served a CAPTCHA, or fed a degraded response.

Wick gives AI agents the same web access their human operators have. Not by routing through third-party residential IP pools (legally radioactive after Google's IPIDEA takedown and the FBI's March 2026 PSA), but by using Chrome's actual network stack and the user's own residential IP — because the user IS the one making the request, through their agent.

---

## Problem

### Why agents get blocked

Modern anti-bot systems fingerprint traffic at multiple layers:

| Layer | What Chrome sends | What AI agents send | Detection |
|-------|-------------------|---------------------|-----------|
| **TLS ClientHello** | BoringSSL with Chrome-specific cipher suites, extensions, curves | Go's `crypto/tls` or Python's `ssl` | JA3/JA4 fingerprint mismatch |
| **HTTP/2 SETTINGS** | `INITIAL_WINDOW_SIZE=6291456`, Chrome-specific PRIORITY frames | Go's `x/net/http2`: `INITIAL_WINDOW_SIZE=65535` (100x smaller) | HTTP/2 fingerprinting (Akamai) |
| **HTTP/2 pseudo-headers** | `:method, :authority, :scheme, :path` (Chrome ordering) | Often different ordering | Header-order fingerprinting |
| **QUIC/HTTP/3** | Chrome's QUIC implementation | Usually absent | Protocol downgrade detection |
| **IP reputation** | Residential ISP | Cloud provider (AWS, GCP, Azure) | IP database lookup |
| **JavaScript** | Executes challenges | No JS engine | Challenge failure |
| **Browser APIs** | Full `navigator`, `canvas`, `WebGL` | None | Automation detection |

### Why uTLS is insufficient

uTLS (used by Lantern, sing-box, and most circumvention tools) patches Go's TLS library to emit a ClientHello that *looks like* Chrome. But modern detection goes deeper:

- **HTTP/2 SETTINGS frames**: uTLS doesn't touch HTTP/2. Go's defaults are wildly different from Chrome's.
- **Cross-layer consistency**: TLS says "Chrome" but HTTP/2 behavior says "Go." This is a known detection vector used by Akamai and Cloudflare.
- **Maintenance burden**: Every Chrome update changes fingerprint parameters. uTLS must manually track these. Cronet updates automatically by rebuilding against new Chromium.
- **x448 curve discrepancy**: uTLS has known JA3 divergence on the x448 curve that Chrome doesn't advertise.

### The market signal

- Browserbase raised $40M (Series B, $300M valuation) for cloud headless browsers for agents
- Tavily raised $25M for search/extraction APIs for AI agents
- Browser Use raised $17M for agent browser steering
- Bright Data launched an MCP server with 60+ tools and 5K free requests/month
- 70+ AI copyright lawsuits filed through 2025, signaling content access is the central tension in AI

The market for agent web access exists, is growing fast, and has no dominant player yet.

---

## Solution

Wick is a lightweight local daemon + optional cloud service that gives AI agents browser-grade HTTP access through Chrome's actual network stack (Cronet), exiting from the user's own residential IP.

### Core principles

1. **Authentic, not mimicked.** Use Chrome's actual TLS/HTTP/2/QUIC code, not a reimplementation.
2. **Local-first.** The free tier runs entirely on the user's machine. Zero cloud infrastructure cost.
3. **User's IP, user's request.** No pooled residential IPs. The user is accessing the site through their own connection.
4. **MCP-native.** Designed for AI agents from day one, not retrofitted.
5. **Human-in-the-loop.** When a CAPTCHA appears, the user solves it — because they're the human the CAPTCHA is looking for.

---

## Architecture

### System overview

```
┌─────────────────────────────────────────────────────────────┐
│  AI Agent (Claude Code / Cursor / any MCP client)           │
│                                                             │
│  MCP tool call: wick_fetch(url, options)                   │
└──────────────────────┬──────────────────────────────────────┘
                       │ JSON-RPC (stdio or HTTP)
                       ▼
┌─────────────────────────────────────────────────────────────┐
│  Wick Daemon (local, ~15MB)                                │
│                                                             │
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────────┐  │
│  │ MCP Server   │  │ Request      │  │ Response          │  │
│  │ (JSON-RPC)   │──│ Pipeline     │──│ Pipeline          │  │
│  └──────────────┘  └──────┬───────┘  └────────┬─────────┘  │
│                           │                    │            │
│  ┌──────────────┐  ┌──────┴───────┐  ┌────────┴─────────┐  │
│  │ Cookie Store  │  │ Cronet       │  │ Content          │  │
│  │ (persistent) │  │ Engine       │  │ Extractor        │  │
│  └──────────────┘  │ (Chromium    │  │ (HTML→Markdown)  │  │
│                    │  143 network  │  └──────────────────┘  │
│                    │  stack)       │                         │
│                    └──────┬───────┘                         │
│                           │                                 │
│  ┌──────────────────────────────────────────────────────┐   │
│  │ CAPTCHA Detector + User-in-the-Loop Handler          │   │
│  └──────────────────────────────────────────────────────┘   │
└──────────────────────┬──────────────────────────────────────┘
                       │ HTTPS (Chrome-identical TLS + HTTP/2)
                       ▼
                 ┌────────────┐
                 │ Target     │
                 │ Website    │
                 │ (user's    │
                 │  resi IP)  │
                 └────────────┘
```

### Cloud tier (paid)

```
┌──────────────────────┐
│  AI Agent            │
└──────────┬───────────┘
           │ MCP tool call
           ▼
┌──────────────────────┐         ┌──────────────────────────┐
│  Wick Daemon        │────────▶│  Wick Cloud             │
│  (local)             │  API    │                          │
│                      │◀────────│  ┌──────────────────┐    │
│  Routes JS-required  │         │  │ Headless Chrome   │    │
│  requests to cloud   │         │  │ Pool              │    │
│                      │         │  └─────────┬────────┘    │
└──────────┬───────────┘         │            │             │
           │                     │  ┌─────────▼────────┐    │
           │ WireGuard tunnel    │  │ Content Extractor │    │
           │ (optional: route    │  │ (rendered DOM →   │    │
           │  cloud traffic      │  │  Markdown/JSON)   │    │
           │  through user's     │  └──────────────────┘    │
           │  residential IP)    │                          │
           ▼                     └──────────────────────────┘
     ┌────────────┐
     │ Target     │  ← exits from user's IP via WireGuard
     │ Website    │     OR from cloud IP (simpler, less evasive)
     └────────────┘
```

### Residential IP routing for cloud tier

When a request requires JavaScript rendering (cloud headless Chrome), the traffic can optionally be routed back through the user's device to preserve their residential IP:

1. Wick daemon establishes a WireGuard tunnel to Wick Cloud
2. Headless Chrome on cloud renders the page
3. Outbound HTTP from the Chrome instance routes through the WireGuard tunnel
4. Traffic exits from the user's residential IP

This is architecturally identical to a corporate split-tunnel VPN (remote worker → office → internet), reversed. The user's device is the "office." Existing Lantern infrastructure (WireGuard fork at `getlantern/wireguard-go`, sing-box transport layer) provides all the building blocks.

**When to use residential routing:**
- Sites with aggressive IP reputation checks (Cloudflare's bot score heavily weights IP)
- Geo-restricted content the user has legitimate access to
- Sites that rate-limit or block cloud provider IP ranges

**When cloud IP is fine:**
- Most documentation sites, API references, public data
- Sites behind Cloudflare but with permissive bot settings
- Reduces latency (no double-hop)

The daemon auto-detects which mode is needed based on the response (cloud-IP attempt first, fall back to residential routing if blocked).

---

## Technical Design

### Core: Cronet Engine

**Why Cronet over uTLS:**

Cronet is Chromium's actual network stack extracted as a library. It is not a reimplementation or fingerprint mimicry — it IS Chrome's networking code (BoringSSL, HTTP/2, QUIC, cookie store, connection pooling). Every layer matches Chrome because every layer comes from Chrome.

| Aspect | uTLS | Cronet |
|--------|------|--------|
| TLS fingerprint | Mimicked (can diverge) | Identical (same code) |
| HTTP/2 SETTINGS | Go defaults (detectable) | Chrome defaults (authentic) |
| HTTP/2 PRIORITY frames | Missing or wrong | Chrome-native |
| HTTP/3 / QUIC | Separate library, own fingerprint | Chrome's QUIC stack |
| Maintenance | Manual tracking of Chrome changes | Rebuild against new Chromium |
| Binary size | ~0 (pure Go) | 8-15 MB shared library |

**Implementation:**

Use `github.com/sagernet/cronet-go` (actively maintained, Chromium 143 as of March 2026). It exposes `http.RoundTripper`, making integration with Go's standard HTTP library trivial:

```go
import (
    "net/http"
    cronet "github.com/sagernet/cronet-go"
)

func NewWickClient() *http.Client {
    engine := cronet.NewEngine()
    params := cronet.NewEngineParams()
    params.SetEnableHTTP2(true)
    params.SetEnableQuic(true)
    params.SetUserAgent(chromeUserAgent()) // Match current Chrome UA
    params.SetStoragePath(cookieStorePath()) // Persistent cookies
    engine.StartWithParams(params)

    return &http.Client{
        Transport: cronet.NewRoundTripper(engine),
    }
}
```

**Platform support (from sagernet/cronet-go releases):**

| Platform | Size | Notes |
|----------|------|-------|
| macOS amd64/arm64 | ~10 MB | Primary dev platform |
| Linux amd64/arm64 | ~10-11 MB | Cloud + Linux desktop |
| Windows amd64/arm64 | ~8 MB | Windows desktop |
| Android arm/arm64 | ~9-11 MB | Future mobile agent use |
| iOS arm64 | ~10 MB | Future mobile agent use |

**Lantern's prior art:**

Lantern already has forks of both `cronet` and `cronet-go` (at `getlantern/cronet` and `getlantern/cronet-go`), though both are stale (2022-2023). The SagerNet versions are current and better maintained. We should use SagerNet's builds and contribute upstream rather than maintaining a fork.

### Request Pipeline

```
Incoming MCP request
    │
    ├─ URL validation + sanitization
    │
    ├─ Cookie injection (from persistent store)
    │
    ├─ Header construction
    │   ├─ Chrome-compatible Accept, Accept-Language, Accept-Encoding
    │   ├─ Sec-Ch-UA, Sec-Ch-UA-Mobile, Sec-Ch-UA-Platform
    │   ├─ Sec-Fetch-Site, Sec-Fetch-Mode, Sec-Fetch-Dest
    │   └─ Referer (based on navigation context)
    │
    ├─ Cronet HTTP request (TLS + HTTP/2 + QUIC)
    │
    ├─ Response analysis
    │   ├─ Success → Content extraction pipeline
    │   ├─ CAPTCHA detected → CAPTCHA handler
    │   ├─ JS required → Route to cloud tier (if enabled)
    │   └─ Rate limited → Backoff + retry with jitter
    │
    └─ Response to agent
        ├─ Markdown content (default)
        ├─ Raw HTML (optional)
        ├─ Structured JSON (optional, extraction mode)
        └─ Metadata (status, headers, cookies set, timing)
```

### Content Extraction

AI agents don't want raw HTML. They want clean, LLM-friendly content. The response pipeline converts HTML to structured output:

1. **HTML → Markdown** (default): Strip nav, ads, footers. Extract article/main content. Preserve headings, lists, tables, code blocks, links. Use a readability algorithm (similar to Mozilla Readability / Postlight Mercury).

2. **Structured extraction** (optional): Given a schema, extract specific fields from the page. E.g., "extract product name, price, and description" → returns JSON. Uses CSS selectors and heuristics locally; can use LLM extraction in cloud tier.

3. **Screenshot** (cloud tier only): Return a PNG/JPEG of the rendered page. Useful for agents that need visual context.

### Cookie Persistence

A critical component. Many sites work fine after the first visit sets cookies (Cloudflare `cf_clearance`, session tokens, consent cookies). Wick maintains a persistent cookie store per user:

- Stored in the user's config directory (`~/.wick/cookies.db`)
- SQLite-backed (like Chrome's own cookie store)
- Respects cookie expiry, domain scoping, Secure/HttpOnly flags
- Shared across all agent sessions
- Encrypted at rest with a user-specific key

This means: solve a Cloudflare challenge once, and subsequent requests to that domain work without challenge for hours or days.

### JavaScript Rendering (Pro)

For JavaScript-heavy sites (SPAs, dynamically loaded content), Wick Pro offers local JavaScript rendering that runs entirely on the user's machine. Contact us for details.

---

## MCP Interface

Wick exposes its capabilities through the Model Context Protocol, making it automatically available to any MCP-compatible agent (Claude Code, Cursor, Windsurf, etc.).

### Tools

#### `wick_fetch`

Primary tool. Fetches a URL with browser-grade fidelity.

```json
{
  "name": "wick_fetch",
  "description": "Fetch a web page using Chrome's network stack with browser-grade TLS fingerprinting. Returns clean, LLM-friendly content. Falls back to JavaScript rendering for dynamic pages.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "url": {
        "type": "string",
        "description": "The URL to fetch"
      },
      "format": {
        "type": "string",
        "enum": ["markdown", "html", "text", "json"],
        "default": "markdown",
        "description": "Output format. markdown (default) strips boilerplate and returns clean content."
      },
      "extract": {
        "type": "object",
        "description": "Optional structured extraction schema. Fields to extract from the page.",
        "additionalProperties": { "type": "string" }
      },
      "wait_for_js": {
        "type": "boolean",
        "default": false,
        "description": "Force JavaScript rendering via cloud tier (requires paid plan)."
      },
      "screenshot": {
        "type": "boolean",
        "default": false,
        "description": "Return a screenshot of the rendered page (requires paid plan)."
      }
    },
    "required": ["url"]
  }
}
```

**Example agent interaction:**
```
Agent: I need to read the documentation at https://example.com/docs/api
       [calls wick_fetch with url="https://example.com/docs/api"]

Wick: Returns markdown content of the page, clean and ready for the LLM context window.
```

#### `wick_search`

Web search with result fetching. Searches via a search engine, then optionally fetches top results.

```json
{
  "name": "wick_search",
  "description": "Search the web and optionally fetch top results with browser-grade access.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "query": {
        "type": "string",
        "description": "Search query"
      },
      "num_results": {
        "type": "integer",
        "default": 5,
        "description": "Number of search results to return"
      },
      "fetch_results": {
        "type": "boolean",
        "default": false,
        "description": "Fetch full content of each result URL"
      },
      "format": {
        "type": "string",
        "enum": ["markdown", "html", "text"],
        "default": "markdown"
      }
    },
    "required": ["query"]
  }
}
```

#### `wick_session`

Manage browser sessions (cookies, auth state) across requests.

```json
{
  "name": "wick_session",
  "description": "Manage persistent browser sessions for multi-step web interactions.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "action": {
        "type": "string",
        "enum": ["list", "clear", "clear_domain", "export"],
        "description": "Session management action"
      },
      "domain": {
        "type": "string",
        "description": "Domain to operate on (for clear_domain)"
      }
    },
    "required": ["action"]
  }
}
```

### Transport

Wick's MCP server supports two transports:

1. **stdio** (default): For local integration with Claude Code, Cursor, etc. The daemon is launched as a subprocess.
2. **HTTP + SSE**: For remote/cloud deployment. Enables team sharing of a single Wick instance.

### Agent configuration

Claude Code `settings.json`:
```json
{
  "mcpServers": {
    "wick": {
      "command": "wick",
      "args": ["serve", "--mcp"],
      "env": {
        "FLINT_API_KEY": "your-key-here"
      }
    }
  }
}
```

Or via Homebrew one-liner:
```bash
brew install getlantern/tap/wick && wick setup
```

`wick setup` auto-detects installed MCP clients (Claude Code, Cursor, etc.) and configures them.

---

## CAPTCHA Strategy

### Layer 1: Avoid triggering (most requests)

Cronet's authentic Chrome fingerprint + the user's residential IP + persistent cookies means most requests never trigger a CAPTCHA. The signals that trigger challenges (TLS mismatch, HTTP/2 anomalies, datacenter IP, missing cookies) are all addressed by the core architecture.

### Layer 2: User-in-the-loop (Wick's differentiator)

When a CAPTCHA is detected, Wick leverages its unique position as a local daemon on the user's device:

```
Cronet request returns CAPTCHA page
    │
    ├─ Detect challenge type:
    │   ├─ Cloudflare Turnstile (script src: challenges.cloudflare.com)
    │   ├─ reCAPTCHA v2/v3 (script src: google.com/recaptcha)
    │   ├─ hCaptcha (script src: hcaptcha.com)
    │   └─ Custom JS challenge (Cloudflare managed challenge page)
    │
    ├─ Native OS notification:
    │   "Wick: Your AI agent needs help with a CAPTCHA on example.com"
    │
    ├─ User clicks notification → minimal webview opens
    │   (macOS: WKWebView, Windows: WebView2, Linux: WebKitGTK)
    │
    ├─ User solves CAPTCHA (typically 5-10 seconds)
    │
    ├─ Extract clearance token/cookies from webview
    │   (cf_clearance, __cf_bm, session cookies, etc.)
    │
    ├─ Store in persistent cookie database
    │
    └─ Retry original request with new cookies → success
```

**Why this is the right approach:**

- CAPTCHAs exist to prove a human is present. A human IS present — the user.
- The user solves their OWN CAPTCHA on their OWN device for their OWN request.
- No ethical ambiguity: this is identical to the user clicking through a CAPTCHA in their browser.
- Session cookies persist for hours or days, so this is a rare event per domain.
- No third-party CAPTCHA solving service cost.
- No dependency on external APIs.

**Implementation notes:**

The webview must be configured to match Wick's Cronet cookies and state, so the clearance token is valid for subsequent Cronet requests. This requires sharing the cookie store between Cronet and the webview, or extracting and replaying cookies between them.

### Layer 3: Automated solving (paid add-on)

For unattended operation (CI/CD, batch jobs), offer optional third-party CAPTCHA solving:

| Provider | Method | Cost/1K solves | Speed |
|----------|--------|---------------|-------|
| CapSolver | AI-based | $1-2 | 5-15s |
| 2Captcha | Human workers | $1-3 | 15-45s |

This is a pass-through cost with markup. Opt-in only.

### Layer 4: Local JS challenge solving (CEF, paid tier)

Cloudflare's "managed challenge" (the "Checking your browser..." interstitial) is a JavaScript challenge, not a visual CAPTCHA. The local CEF renderer executes the JS challenge natively — from the user's residential IP, with no CDP fingerprints — and extracts the clearance cookie. Most managed challenges are resolved automatically without user interaction.

---

## Distribution

### Primary: MCP tool directories + package managers

```
brew tap myleshorton/wick && brew install wick    # macOS
npm install -g wick-mcp                            # cross-platform (wraps Go binary)
```

The `wick setup` command auto-configures all detected MCP clients.

### Secondary: Agent documentation and word-of-mouth

When an MCP client uses WebFetch and gets blocked, the error message is visible to both the agent and user. Blog posts, tutorials, and MCP server directories drive discovery at this moment of need.

### Tertiary: IDE extensions

VS Code / JetBrains extensions that bundle Wick and add a "Fetch with Wick" command. These also serve as MCP server configuration helpers.

### Growth loop

```
Developer installs Wick (free) for their AI agent
    → Agent uses Wick to access previously-blocked sites
    → Developer hits a JS-heavy site that needs cloud rendering
    → Upgrade to paid tier
    → Developer recommends Wick to team (multiplayer via HTTP transport)
    → Team installs Wick
```

---

## Business Model

### Pricing tiers

| Tier | Price | What you get | Our cost |
|------|-------|--------------|----------|
| **Free** | $0 | Local Cronet fetch, browser-grade TLS, cookie persistence, CAPTCHA user-in-the-loop, markdown extraction | ~$0 (runs on user's device) |
| **Pro** | $29/mo | Everything Free + local CEF JS rendering (auto-configured), structured extraction, auto CAPTCHA solving, priority support | ~$0 marginal (runs on user's device) |
| **Team** | $79/mo (5 seats) | Everything Pro + shared sessions, team cookie store, HTTP MCP transport | ~$2-5/team (API/auth infrastructure) |
| **Enterprise** | Custom | Everything Team + SLA, SSO, audit logs, on-prem deployment, custom extraction models | Variable |

### Unit economics (Pro tier)

- Revenue: $29/mo
- Infrastructure cost: ~$0 (everything runs locally on user's device)
- CAPTCHA solving (if automated): ~$0.002/solve × estimated 50/mo = $0.10
- Auth/billing/support overhead: ~$2/user amortized
- **Gross margin: ~93%**

The local-first architecture means the Pro tier is almost pure margin — we're selling software, not cloud compute. The user's machine does all the work.

### Revenue projections (conservative)

| Timeline | Free users | Paid users | MRR |
|----------|-----------|-----------|-----|
| 6 months | 5,000 | 200 | $6K |
| 12 months | 50,000 | 2,000 | $60K |
| 24 months | 200,000 | 10,000 | $400K |

The free tier costs us nothing — it runs entirely on the user's hardware. Free users are pure funnel.

---

## Competitive Landscape

### Direct competitors

| Company | Approach | Strengths | Weaknesses vs. Wick |
|---------|----------|-----------|---------------------|
| **Bright Data MCP** | Cloud proxy with 150M+ residential IPs | Huge IP pool, 60+ MCP tools, enterprise brand | Ethically problematic IP sourcing history (Luminati/Hola), expensive, no local-first option, legally exposed |
| **Firecrawl** | Cloud scraping + `/agent` endpoint | Open source, good extraction, 77% coverage | No TLS fingerprinting, cloud-only, no residential IP |
| **Browserbase** | Cloud headless Chrome | $40M funding, 50M sessions, mature | Cloud-only, no local tier, no anti-detection (relies on IP quality), expensive at scale |
| **Tavily** | Search API for RAG | $25M funding, strong AI-native positioning | Search-focused (not arbitrary URL access), no anti-detection |
| **Playwright MCP** | Browser automation via accessibility tree | Free, built into Claude Code | No anti-detection, heavy (full browser per request), not designed for fetching |

### Wick's moat

1. **Cronet (authentic Chrome fingerprint)**: Not mimicry. The actual network stack. Competitors use uTLS at best, nothing at worst.
2. **Local-first architecture**: Free tier costs us $0. Competitors run cloud infrastructure for every request.
3. **User's own residential IP**: No pooled IPs, no ethical/legal baggage.
4. **User-in-the-loop CAPTCHA**: Only possible because we run locally. Cloud-only competitors can't do this.
5. **Lantern's anti-detection DNA**: Decade of experience evading the GFW — a harder adversary than Cloudflare.

### Not competitors

- **Anthropic (Computer Use, Claude for Chrome)**: Browser control, not HTTP access. Different use case (visual interaction vs. content fetching).
- **OpenAI (Operator/ChatGPT Agent)**: End-user product, not developer infrastructure.
- **Standard HTTP clients (curl, httpx, requests)**: No anti-detection, no MCP, no content extraction.

---

## Legal Analysis

### Why Wick is legally defensible

**The core architecture is clean:**
- The user installs Wick on their device
- The user's AI agent makes requests on the user's behalf
- Requests exit from the user's own IP address
- This is functionally identical to the user browsing with Chrome

**Key precedents:**

- **hiQ v. LinkedIn (2022)**: Scraping publicly accessible data does not violate the CFAA.
- **Meta v. Bright Data (2024)**: Scraping public-facing data without logging in is not a ToS violation.
- **Van Buren v. United States (2021, SCOTUS)**: CFAA only covers accessing systems with no authorization, not exceeding authorized access.

**What Wick does NOT do (critical distinctions):**

| Practice | Legal risk | Wick's position |
|----------|-----------|-----------------|
| Pooled residential IPs | High (FBI PSA, Google/IPIDEA takedown) | Does not pool IPs. User's own IP only. |
| Bypassing authentication | High (CFAA) | Never bypasses login walls. Public pages only. |
| Ignoring robots.txt | Medium (ethical, not legal) | Configurable. Default: respect robots.txt. User can override. |
| Mass scraping | Medium (ToS, resource abuse) | Rate-limited by default. Single-user scale. |
| Selling user bandwidth | High (Hola class action) | Never routes others' traffic through user's device. |

### robots.txt policy

**Default behavior:** Respect robots.txt. If a site disallows the path, return an error with an explanation.

**User override:** `wick_fetch(url, respect_robots=false)` — the user can choose to override, accepting responsibility. This mirrors Chrome's behavior (browsers don't check robots.txt for human-initiated requests).

**Rationale:** robots.txt is advisory (RFC 9309). It's designed for crawlers operating at scale, not individual page fetches by a user's agent. A human browsing to the same URL in Chrome wouldn't check robots.txt. Wick defaults to respecting it as a good-faith gesture, but the user has the final say.

---

## Risks

### Technical risks

| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|-----------|
| **Chromium updates break Cronet builds** | Medium | Medium | SagerNet actively maintains builds. Lantern has institutional knowledge. Multiple Cronet build systems exist (NaiveProxy, SagerNet, Nicegram). |
| **Anti-bot systems fingerprint Cronet specifically** | Low | High | Cronet IS Chrome's stack. Detecting Cronet means detecting Chrome. They'd have to fingerprint based on behavioral patterns (request timing, navigation patterns), not network signatures. |
| **Cloudflare deploys new detection methods** | High | Medium | Layered approach: Cronet handles network fingerprint, headless Chrome handles JS challenges, user-in-the-loop handles CAPTCHAs. Multiple fallback levels. |
| **Cookie store corruption/conflicts** | Low | Low | SQLite WAL mode, encrypted backups, per-domain isolation. |

### Business risks

| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|-----------|
| **Anthropic builds this into Claude Code** | Medium | Critical | Move fast. Establish user base and brand before this happens. Anthropic may prefer to partner rather than build (MCP's whole thesis is extensibility). Offer features Anthropic won't (robots.txt override, CAPTCHA solving). |
| **Bright Data dominates MCP market** | Medium | High | Differentiate on architecture (local-first, user's IP, no ethical baggage). Bright Data's Luminati history is a liability with developers who care about ethics. |
| **Legal landscape shifts against scraping** | Low-Medium | High | Architecture is defensible (user's own IP, user's own request). Pivot toward content licensing model if market shifts. |
| **Low conversion from free to paid** | Medium | Medium | Free tier costs us nothing. Even low conversion is profitable. Add more paid-only features (structured extraction, search, batch operations). |

### Reputational risks

| Risk | Mitigation |
|------|-----------|
| Used for abusive scraping at scale | Rate limiting, abuse detection, ToS prohibiting automated mass-scraping |
| Association with "bot" tools | Positioning: "browser-grade web access," not "scraping tool." Framing: the user's agent accesses the same pages the user can. |
| Content publishers object | Default robots.txt respect. Future: content licensing/compensation mechanism. |

---

## Roadmap

### Phase 1: Foundation (Weeks 1-4)

**Goal:** Working local daemon with MCP interface and Cronet-based fetch.

- [ ] Go project scaffolding with `sagernet/cronet-go` dependency
- [ ] Cronet engine initialization with Chrome-equivalent configuration
- [ ] Request pipeline: URL → headers → Cronet → response
- [ ] Cookie persistence (SQLite store)
- [ ] HTML → Markdown content extraction (readability algorithm)
- [ ] MCP server (stdio transport) exposing `wick_fetch`
- [ ] `wick setup` auto-configuration for Claude Code
- [ ] macOS build + Homebrew formula
- [ ] Linux amd64 build
- [ ] Basic test suite against known-blocked sites

**Deliverable:** `brew install wick && wick setup` gives Claude Code browser-grade web access.

### Phase 2: CAPTCHA + Polish (Weeks 5-8)

**Goal:** Handle CAPTCHAs gracefully. Cross-platform support.

- [ ] CAPTCHA page detection (Cloudflare, reCAPTCHA, hCaptcha signatures)
- [ ] Native OS notification system (macOS notification center, Windows toast, Linux libnotify)
- [ ] Minimal webview for CAPTCHA solving (WKWebView / WebView2 / WebKitGTK)
- [ ] Cookie extraction from webview → Cronet cookie store
- [ ] Windows build + installer
- [ ] `wick_search` tool (web search + result fetching)
- [ ] `wick_session` tool (cookie management)
- [ ] Error messages with helpful context for agents
- [ ] Rate limiting and backoff logic

**Deliverable:** Wick handles Cloudflare-protected sites gracefully across macOS, Linux, Windows.

### Phase 3: Local JS Rendering via CEF (Weeks 9-14)

**Goal:** Full JavaScript rendering for SPAs and JS challenges, running locally.

- [ ] C++ `wick-renderer` binary with CEF offscreen rendering
- [ ] CefMessageRouter-based DOM extraction (no CDP)
- [ ] stdio IPC between `wick` (Go) and `wick-renderer` (C++)
- [ ] Auto-detection of JS-required pages (empty content, SPA bootstrap, `<noscript>`)
- [ ] `wick setup --with-js` downloads platform-specific CEF distribution (~120MB)
- [ ] Structured extraction (CSS selectors + LLM-assisted)
- [ ] Evaluate full rewrite in Rust or C++ if Go+CEF integration proves too complex

**Deliverable:** `wick_fetch(url, wait_for_js=true)` renders JavaScript-heavy pages locally via CEF.

### Phase 3b: Paid Tier (Weeks 12-16)

**Goal:** Revenue from Pro features.

- [ ] Stripe billing + API key management
- [ ] Auto-CAPTCHA solving (webview + CapSolver/2Captcha fallback)
- [ ] Auto-configured CEF (no manual `--with-js` setup)
- [ ] Structured data extraction
- [ ] Priority support
- [ ] HTTP + SSE MCP transport for team/remote use

### Phase 4: Scale + Ecosystem (Weeks 15-20)

**Goal:** Growth features, integrations, and ecosystem play.

- [ ] npm package (`@wick/cli`) for cross-platform Node.js distribution
- [ ] VS Code extension (MCP configuration + "Fetch with Wick" command)
- [ ] Automated CAPTCHA solving integration (CapSolver/2Captcha)
- [ ] Batch fetching (parallel URL list processing)
- [ ] Caching layer (avoid redundant fetches for popular docs pages)
- [ ] Analytics dashboard (usage, success rates, blocked domains)
- [ ] Public MCP server directory listings
- [ ] Blog + developer content (launch post, benchmarks vs. competitors)

### Future: Content Licensing (Phase 5+)

If Wick gains traction and builds relationships with frequently-accessed sites, explore the content licensing marketplace model:

- Publishers register sites and set per-access pricing
- Agents authenticate and pay per access
- Wick takes a transaction fee
- Content served in LLM-optimized format
- Everyone wins: publishers get compensated, agents get reliable access, no adversarial cat-and-mouse

This is a long-term play that becomes viable only with significant scale and publisher trust.

---

## Appendix A: Cronet vs. uTLS Deep Comparison

### Detection vectors addressed by Cronet but not uTLS

1. **HTTP/2 SETTINGS frame**: Go's `x/net/http2` sends `INITIAL_WINDOW_SIZE=65535`. Chrome sends `6291456`. This 100x difference is trivially detectable. Cronet matches Chrome exactly.

2. **HTTP/2 PRIORITY frames**: Chrome sends complex priority trees. Go's HTTP/2 sends none or flat priorities. Akamai fingerprints this.

3. **HTTP/2 pseudo-header ordering**: Chrome sends `:method, :authority, :scheme, :path`. Some Go HTTP/2 implementations reorder these.

4. **QUIC version negotiation**: Chrome's QUIC has specific version preferences and transport parameters. No Go QUIC library matches these.

5. **Connection-level behavior**: Chrome's connection pooling, keepalive timing, and window update patterns differ from Go's. Cronet matches because it IS Chrome's connection management code.

6. **TCP socket options**: Chrome sets specific TCP_NODELAY, SO_RCVBUF, and window scaling values that differ from Go defaults. (Note: this depends on OS-level socket options, not fully controlled by Cronet, but closer to Chrome's behavior.)

### Benchmark: detection rates

Estimated detection rates for different approaches against modern anti-bot systems (Cloudflare, Akamai, DataDome):

| Approach | Estimated block rate |
|----------|---------------------|
| Raw Go `net/http` | 80-95% |
| Go + uTLS (Chrome fingerprint) | 30-50% |
| Go + uTLS + custom HTTP/2 framing | 15-25% |
| Cronet | 5-10% |
| Cronet + residential IP | 1-3% |
| Headless Chrome + residential IP | <1% |

These are estimates based on Lantern's experience with anti-detection and publicly available fingerprinting research. Actual rates vary by site and anti-bot vendor.

## Appendix B: Lantern Infrastructure Reuse

Components from the Lantern codebase that can be directly leveraged:

| Component | Lantern Source | Wick Use |
|-----------|---------------|-----------|
| WireGuard tunnel | `getlantern/wireguard-go` | Residential IP routing for cloud tier |
| Cronet Go bindings | `getlantern/cronet-go` (stale) → use `sagernet/cronet-go` | Core network engine |
| Sing-box transport | `getlantern/sing-box-minimal` | Transport multiplexing (if needed) |
| User auth + payments | `radiance/api` | Account management, Stripe billing |
| Config management | `radiance/config` | Dynamic configuration |
| Private server provisioning | `lantern-core/private-server` | Cloud infrastructure deployment |
| Event system | `radiance/events` | Real-time status updates to agents |
| IPC (named pipes) | `lantern-core/vpn_tunnel` | Daemon ↔ webview communication |

## Appendix C: Example Agent Interaction

### Before Wick

```
User: "Read the API docs at https://example.com/docs/api"

Agent: I'll fetch that page for you.
       [uses WebFetch tool]

Result: 403 Forbidden / Cloudflare challenge page / empty body

Agent: I'm sorry, I wasn't able to access that page. The site appears to
       be blocking automated requests. You could try copying the content
       and pasting it here, or I can work with what I know about their API.
```

### After Wick

```
User: "Read the API docs at https://example.com/docs/api"

Agent: I'll fetch that page for you.
       [uses wick_fetch tool]

Result: Clean markdown of the full API documentation

Agent: Here's what I found in their API docs:
       [proceeds to help with the actual task]
```

### CAPTCHA scenario

```
User: "Check the pricing on https://protected-site.com/pricing"

Agent: [uses wick_fetch tool]

Wick: [detects Cloudflare challenge]
       [sends OS notification: "Wick: CAPTCHA needed for protected-site.com"]

User: [clicks notification, solves CAPTCHA in 5 seconds]

Wick: [captures cf_clearance cookie, retries request]

Result: Clean markdown of the pricing page

Agent: Here's the pricing information:
       [proceeds normally, no mention of the CAPTCHA]
```

---

*This document is a living design. Last updated: 2026-03-20.*
