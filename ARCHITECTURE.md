# Wick: Architecture & Language Decision

> Should Wick stay in Go, or rewrite in Rust/C++/TypeScript?

**Date:** 2026-03-21
**Status:** Under evaluation
**Context:** Phase 1 (Go + Cronet) is working. Phase 2+ needs JavaScript rendering via CEF. Go has no mature CEF bindings.

---

## The Core Question

Wick currently has two Chromium dependencies:

1. **Cronet** (network stack only) — handles 90%+ of requests, already working in Go via `sagernet/cronet-go`
2. **CEF** (full browser engine) — needed for JS rendering, no production Go bindings exist

The question isn't just "what language for CEF" — it's whether the two-process architecture (Go + C++ subprocess) introduces enough complexity and fragility to justify a single-language rewrite.

---

## Can CEF Be Detected?

This is the threshold question. If CEF is detectable and blockable, the whole approach needs rethinking.

### Network Level: Undetectable

CEF uses the exact same network stack as Chrome (BoringSSL, HTTP/2, QUIC). At the wire level:

| Signal | Chrome | CEF | Detectable? |
|--------|--------|-----|-------------|
| TLS ClientHello (JA3/JA4) | BoringSSL | Same BoringSSL | No |
| HTTP/2 SETTINGS frame | Chromium defaults | Same | No |
| HTTP/2 PRIORITY frames | Chrome's priority tree | Same | No |
| QUIC parameters | Chrome's QUIC | Same (if enabled) | No |
| Certificate handling | System + CT checks | Same | No |

**CEF's network fingerprint is identical to the Chrome version it was built against.** This is the same reason Cronet works — both are Chromium's actual code.

### JavaScript Level: Detectable But Patchable

Anti-bot systems fingerprint the JS environment. CEF differs from Chrome in several ways:

| Signal | Chrome | CEF (default) | Risk | Patchable? |
|--------|--------|---------------|------|------------|
| `chrome.runtime` | Full Extension API | Missing | High | Yes — inject fake |
| `navigator.plugins` | PDF Plugin, PDF Viewer, Native Client | Empty array | High | Yes — spoof |
| `chrome.csi()` | Present | Missing | Medium | Yes |
| `chrome.loadTimes()` | Present | Missing | Medium | Yes |
| `navigator.webdriver` | false | false | None | N/A (same) |
| Speech synthesis | Present | May be missing | Low | Harder |
| `Notification.permission` | "default" or "granted" | May throw | Low | Yes |
| Screen dimensions (OSR mode) | Realistic | 0x0 or inner=outer | Medium | Yes — set realistic values |

**The critical difference from CDP-based tools:** These are missing *features*, not *automation artifacts*. Playwright/Puppeteer leave active traces (`navigator.webdriver = true`, `__cdp_binding__`, modified prototypes). CEF just lacks some Chrome-specific APIs that aren't part of the web standard.

### How Anti-Bot Systems Use These Signals

Detection is **score-based, not binary**. Cloudflare, Akamai, and DataDome assign a risk score from many signals:

```
Risk Score = Σ(signal_weight × signal_value)

Signals:
  - TLS fingerprint mismatch:     +50 points  ← CEF passes (0 points)
  - HTTP/2 SETTINGS mismatch:     +40 points  ← CEF passes (0 points)
  - Datacenter IP:                +30 points  ← CEF passes (residential, 0 points)
  - Missing chrome.runtime:       +15 points  ← patchable
  - Empty navigator.plugins:      +10 points  ← patchable
  - No mouse/keyboard events:     +20 points  ← behavioral, manageable
  - Missing Chrome PDF plugin:    +5 points   ← patchable
  - Canvas fingerprint anomaly:   +10 points  ← CEF has real GPU, passes
```

A well-configured CEF instance with injected Chrome APIs, realistic screen dimensions, and occasional simulated events would score very low — likely below the challenge threshold.

### Can Sites Block CEF Specifically?

**In theory:** A site could enumerate every Chrome-specific API and check for subtle behavioral differences. But this is fragile — Chrome itself changes these APIs between versions, and false positives against real Chrome users are unacceptable.

**In practice:** No major anti-bot vendor currently blocks CEF specifically. They block *automation frameworks* (Selenium, Playwright, Puppeteer) because those leave distinctive CDP traces. CEF doesn't use CDP for DOM access.

**The Steam precedent:** Valve's Steam client is a CEF application. It loads web content from third-party sites (community pages, embedded storefronts) without being blocked. If anti-bot systems blocked CEF, they'd block Steam's ~130 million users.

### Risk Assessment

| Scenario | Likelihood | Impact | Mitigation |
|----------|-----------|--------|------------|
| Anti-bot systems detect default CEF | Medium | Low | Inject Chrome APIs before page load |
| Anti-bot systems detect patched CEF | Low | Medium | Build CEF from Chromium source with patches compiled in |
| Anti-bot systems specifically target CEF | Very Low | High | Switch to user's actual Chrome (last resort) |
| CEF version lag causes fingerprint mismatch | Medium | Low | Track Chrome stable releases, rebuild regularly |

**Verdict: CEF is a viable approach.** The JS-level detection vectors are real but patchable, and the network level is identical to Chrome. The risk is lower than CDP-based approaches (Playwright/Puppeteer) and much lower than Go's standard HTTP stack.

---

## Architecture Options

### Option A: Stay in Go (Hybrid)

```
wick (Go, 29MB)                    wick-renderer (C++, ~120MB)
┌────────────────────┐             ┌─────────────────────────┐
│ MCP Server (stdio) │             │ CEF offscreen browser   │
│ Cronet HTTP client │  stdio IPC  │ Chrome API injection    │
│ Content extraction  │◄──────────►│ CefMessageRouter DOM    │
│ Robots.txt, cookies│             │ extraction              │
│ CLI (cobra)        │             └─────────────────────────┘
└────────────────────┘
```

**Pros:**
- Phase 1 is done and working. No rewrite needed.
- CEF renderer is in C++ where CEF is native.
- Clear separation of concerns.

**Cons:**
- Two languages, two build systems, two binaries.
- IPC adds latency and failure modes (pipe broken, renderer crash, message serialization bugs).
- CEF's multi-process model (browser + renderer + GPU subprocess) already means 3-4 OS processes. Adding the Go process makes it 4-5.
- Debugging cross-process issues is painful.
- Chrome API injection (for stealth) requires C++ code in the renderer process, which is harder to iterate on than a single-language solution.
- Go's cgo has well-documented overhead and complexity for Cronet; adding CEF IPC on top compounds this.

**Effort:** ~4-6 weeks for the C++ renderer, IPC protocol, and integration.

### Option B: Rewrite in Rust

```
wick (Rust, single binary, ~30-40MB without CEF)
┌──────────────────────────────────────────────┐
│ MCP Server (rmcp, stdio)                      │
│ Cronet HTTP client (FFI to libcronet)         │
│ Content extraction (scraper + pulldown-cmark) │
│ CEF integration (FFI to libcef C API)         │
│ Chrome API injection (V8 context, pre-load)   │
│ CLI                                           │
└──────────────────────────────────────────────┘
      │
      ├─ CEF renderer subprocess (same binary, --type=renderer)
      ├─ CEF GPU subprocess (same binary, --type=gpu-process)
      └─ CEF utility subprocess (same binary, --type=utility)
```

**Pros:**
- Single language, single binary, single build system.
- Native FFI to both Cronet (C) and CEF (C) — no cgo overhead.
- CEF's subprocess model works naturally (same binary with different `--type` flags).
- Rust's type system catches many of the memory/concurrency bugs that make C++ CEF integration fragile.
- Chrome API injection code lives alongside the MCP server code — easier to iterate.
- `rmcp` crate provides a mature MCP server implementation.
- Excellent cross-platform support.
- Async runtime (tokio) handles MCP I/O + CEF callbacks naturally.

**Cons:**
- Full rewrite of Phase 1 (MCP server, CLI, content extraction, fetch pipeline).
- No existing Rust CEF bindings — need to write FFI wrappers (~2-4 weeks).
- Cronet FFI from Rust requires the same static library approach as Go.
- Smaller ecosystem for HTML → markdown conversion (though `scraper` + `pulldown-cmark` cover it).
- Team needs Rust expertise.

**Effort:** ~6-8 weeks for full rewrite including CEF integration.

**Rust crate equivalents:**

| Go Package | Rust Equivalent | Maturity |
|------------|----------------|----------|
| `sagernet/cronet-go` | FFI to `libcronet.a` (same C library) | Same underlying lib |
| `modelcontextprotocol/go-sdk` | `rmcp` | Good — actively maintained |
| `readeck/go-readability` | `readability` crate or `scraper` + custom | Medium |
| `JohannesKaufmann/html-to-markdown` | `htmd` or custom with `pulldown-cmark` | Medium |
| `temoto/robotstxt` | `robotstxt` crate | Good |
| `spf13/cobra` | `clap` | Excellent |

### Option C: Electron / TypeScript

```
wick (Electron app, ~150MB)
┌──────────────────────────────────────────────┐
│ MCP Server (official @modelcontextprotocol/sdk│
│ Chromium network stack (electron net module)  │
│ BrowserWindow (hidden) for JS rendering       │
│ Content extraction (readability + turndown)    │
│ CLI (commander.js)                            │
└──────────────────────────────────────────────┘
```

**Pros:**
- Electron IS Chromium. Network stack is 100% Chrome. Rendering is 100% Chrome.
- No FFI, no bindings, no C code. Everything is TypeScript.
- MCP SDK is most mature in TypeScript (it's the reference implementation).
- Massive npm ecosystem for content extraction (Readability.js, Turndown, cheerio).
- `BrowserWindow` with `show: false` gives full page rendering without CDP.
- npm distribution is trivial.
- Fastest to implement.

**Cons:**
- ~150MB binary for ALL requests, even simple Cronet-level fetches.
- No "lean mode" — you always ship the full browser.
- Electron's `navigator.userAgent` contains "Electron" by default (patchable but an extra step).
- Missing `chrome.runtime` and empty `navigator.plugins` (same as CEF, same patches needed).
- V8 startup time adds latency even for simple fetches.
- Electron is perceived as heavy/bloated by developer audience.
- Harder to achieve the "29MB self-contained binary" marketing message.

**Effort:** ~3-4 weeks for full implementation.

### Option D: Rust + wry/Tauri (platform-native webview)

```
wick (Rust + wry, ~10-15MB)
┌──────────────────────────────────────────────┐
│ MCP Server (rmcp)                             │
│ Cronet HTTP client (FFI)                      │
│ wry WebView for JS rendering                  │
│   macOS: WKWebView (WebKit)                   │
│   Linux: WebKitGTK (WebKit)                   │
│   Windows: WebView2 (Chromium/Edge)           │
│ Content extraction                            │
└──────────────────────────────────────────────┘
```

**Pros:**
- Tiny binary (~10-15MB) — no bundled browser engine.
- Uses the OS-provided webview — zero download cost for JS rendering.
- `wry` is mature (maintained by Tauri team, widely used).
- Rust gives single-language benefits.

**Cons:**
- **Different browser engine per platform.** macOS uses WebKit (Safari), Linux uses WebKitGTK, Windows uses Edge/Chromium. The TLS fingerprint differs per platform and does NOT match Chrome.
- On macOS, WKWebView has a Safari TLS fingerprint, which is a completely different JA3/JA4 than Cronet/Chrome. Anti-bot systems would see "TLS says Safari, HTTP headers say Chrome" — an instant red flag.
- WKWebView does not work reliably offscreen (Apple limitation).
- **Fingerprint inconsistency between Cronet path and webview path.**

**Verdict: Not viable.** The mixed fingerprint problem (Cronet says Chrome, WKWebView says Safari) is worse than the problems it solves.

---

## Comparison Matrix

| Factor | Go Hybrid | Rust | Electron | Rust + wry |
|--------|-----------|------|----------|------------|
| **Binary size (no CEF)** | 29MB | 30-40MB | 150MB | 10-15MB |
| **Binary size (with CEF)** | 29MB + 120MB | 30-40MB + 120MB | 150MB (included) | 10-15MB (system) |
| **Language count** | 2 (Go + C++) | 1 | 1 | 1 |
| **Process count** | 4-5 | 3-4 (CEF subprocesses) | 3-4 | 2-3 |
| **CEF integration** | IPC to subprocess | Direct FFI | N/A (IS Chromium) | N/A (uses WebView) |
| **MCP SDK maturity** | Good | Good (rmcp) | Best (reference impl) | Good (rmcp) |
| **Cronet for fast path** | Yes (existing) | Yes (same FFI) | Unnecessary (Electron net) | Yes (FFI) |
| **Fingerprint consistency** | Chrome (both paths) | Chrome (both paths) | Chrome (unified) | Mixed (Chrome + Safari) |
| **CEF stealth patches** | C++ (hard to iterate) | Rust (medium) | JS (easy to iterate) | N/A |
| **Rewrite effort** | 0 (extend existing) | 6-8 weeks | 3-4 weeks | 5-6 weeks |
| **Cross-platform** | Yes | Yes | Yes | Yes but mixed fingerprint |
| **Developer perception** | Lean, fast | Lean, fast, modern | Heavy, bloated | Lean |

---

## Recommendation

### Eliminate: Rust + wry (Option D)

Mixed TLS fingerprints (Chrome on network, Safari on JS rendering) is a fundamental flaw that can't be patched. Eliminated.

### Strong contender: Electron (Option C)

If we accept the 150MB binary size, Electron is the fastest path to a fully working product. The "it IS Chrome" argument is powerful. But the size and perception issues are real for a developer tool — especially one marketed as "lightweight local daemon."

Electron makes sense if:
- Speed to market is the top priority
- The audience doesn't care about binary size
- We want to leverage the npm/TypeScript ecosystem fully

### Strong contender: Rust (Option B)

Single language, single binary, native FFI to both Cronet and CEF. The cleanest long-term architecture. But it's a full rewrite and requires writing CEF FFI bindings from scratch.

Rust makes sense if:
- We're building for the long term (years, not months)
- We want the leanest possible distribution
- We value type safety for the CEF integration (where memory bugs are dangerous)
- We're willing to invest 6-8 weeks before shipping new features

### Viable but fragile: Go Hybrid (Option A)

Keeps the existing codebase but adds complexity at every layer. Two languages, two build systems, IPC between Go and C++, CEF multi-process management from a non-native host.

Go hybrid makes sense if:
- We need to ship CEF rendering in < 4 weeks
- We're not confident in the Rust or Electron paths
- Phase 1's Go code is too valuable to discard

### Proposed path

**Short term:** Ship what we have (Go + Cronet, Phase 1). Continue acquiring users with the static-content use case. This is already valuable.

**Medium term (4-8 weeks):** Prototype the CEF integration in **Rust** as a separate branch/repo. Focus on:
1. Cronet FFI (validate same static library works from Rust)
2. CEF FFI (minimal: create offscreen browser, navigate, extract DOM)
3. Chrome API injection (navigator.plugins, chrome.runtime spoofing)
4. MCP server via rmcp

If the Rust prototype works well, migrate the full codebase. If it hits unexpected walls (CEF FFI complexity, rmcp limitations), fall back to Go Hybrid.

**Why not Electron?** Despite being the fastest path, the 150MB binary and "just another Electron app" framing works against Wick's positioning as a lean, local-first tool. The developer audience for MCP tools (Claude Code, Cursor users) is exactly the audience that notices and cares about this.

---

## Appendix: CEF Stealth Patch Checklist

Patches needed to make CEF indistinguishable from Chrome at the JS level:

```javascript
// Inject before ANY page script via CefRenderProcessHandler::OnContextCreated

// 1. Chrome runtime API (most important — checked by Cloudflare)
window.chrome = window.chrome || {};
window.chrome.runtime = {
  OnInstalledReason: { CHROME_UPDATE: "chrome_update", INSTALL: "install", ... },
  OnRestartRequiredReason: { APP_UPDATE: "app_update", OS_UPDATE: "os_update", ... },
  PlatformArch: { ARM: "arm", X86_32: "x86-32", X86_64: "x86-64", ... },
  PlatformOs: { ANDROID: "android", CROS: "cros", LINUX: "linux", MAC: "mac", WIN: "win" },
  RequestUpdateCheckStatus: { NO_UPDATE: "no_update", THROTTLED: "throttled", UPDATE_AVAILABLE: "update_available" },
  connect: function() { /* noop */ },
  sendMessage: function() { /* noop */ },
};

// 2. Navigator plugins (checked by Cloudflare, Akamai)
Object.defineProperty(navigator, 'plugins', {
  get: () => {
    const plugins = [
      { name: 'Chrome PDF Plugin', filename: 'internal-pdf-viewer', description: 'Portable Document Format', length: 1 },
      { name: 'Chrome PDF Viewer', filename: 'mhjfbmdgcfjbbpaeojofohoefgiehjai', description: '', length: 1 },
      { name: 'Native Client', filename: 'internal-nacl-plugin', description: '', length: 2 },
    ];
    plugins.refresh = () => {};
    return plugins;
  }
});

// 3. Chrome-specific legacy APIs
window.chrome.csi = function() { return { startE: Date.now(), onloadT: Date.now(), pageT: 1, tran: 15 }; };
window.chrome.loadTimes = function() {
  return { commitLoadTime: Date.now() / 1000, connectionInfo: "h2", ... };
};

// 4. Permissions API consistency
const originalQuery = window.navigator.permissions.query;
window.navigator.permissions.query = function(parameters) {
  if (parameters.name === 'notifications') {
    return Promise.resolve({ state: Notification.permission });
  }
  return originalQuery.apply(this, arguments);
};

// 5. WebGL vendor/renderer (if running without GPU)
// Ensure WebGL reports realistic values, not "SwiftShader" or "Google Inc."

// 6. Screen dimensions (for offscreen mode)
if (window.outerWidth === 0) {
  Object.defineProperty(window, 'outerWidth', { get: () => 1920 });
  Object.defineProperty(window, 'outerHeight', { get: () => 1080 });
}
```

These patches are injected via `CefRenderProcessHandler::OnContextCreated` (C++ level) or equivalent, ensuring they execute before any page scripts. This is fundamentally different from CDP's `Runtime.evaluate` which injects AFTER page scripts can observe the modification.

---

*This document is a living analysis. Last updated: 2026-03-21.*
