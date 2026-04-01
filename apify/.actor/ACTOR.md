# Wick Web Fetcher

Getting 403 Forbidden? Your scraper's TLS fingerprint is giving it away.

**Wick uses Chrome's real network stack** (BoringSSL, HTTP/2, QUIC) to fetch web pages. The TLS handshake is identical to a real Chrome browser -- not headless Chromium, not Playwright, not Puppeteer. Sites that block every other scraper return 200 OK to Wick.

## What it does

- **Fetch** any URL and get clean markdown, HTML, or plain text
- **Crawl** a site following links, up to 50 pages deep
- **Map** a site to discover all URLs via sitemap.xml + link following
- Returns LLM-ready markdown -- perfect for RAG pipelines, AI training, and content analysis

## Why not Website Content Crawler?

| | Wick | Website Content Crawler |
|---|---|---|
| TLS fingerprint | Real Chrome (Cronet) | Headless Chromium |
| Anti-bot bypass | High -- real browser signature | Medium -- detectable fingerprint |
| Output | Clean markdown | Clean markdown |
| Speed | Fast (no browser startup) | Slower (full browser launch) |
| Memory | 256 MB | 1-4 GB |
| Compute cost | ~4x cheaper per run | Standard |

## Modes

### Fetch (default)

Fetches one or more URLs and returns clean content. Each URL becomes one row in the output dataset.

### Crawl

Starts from a URL and follows same-domain links. Returns content for every page discovered, each as a separate dataset row. Control depth (1-5) and max pages (1-50).

### Map

Discovers all URLs on a site by checking sitemap.xml and following links. Returns a list of URLs without fetching their content -- useful for planning a targeted crawl.

## Residential IP mode (optional)

For maximum anti-detection, connect this Actor to your own **Wick Pro** instance running on your machine. Requests route through your residential IP -- no datacenter fingerprint, no proxy costs.

1. Install Wick Pro: `wick install pro` (see [getwick.dev](https://getwick.dev))
2. Start the API server: `wick serve --api`
3. Expose via tunnel (Cloudflare Tunnel, ngrok, etc.)
4. Paste the tunnel URL in the **Wick Tunnel URL** input field

This gives you Apify's scheduling and monitoring with Wick's anti-detection and your residential IP.

## Pricing

This Actor is **free to use** -- you only pay for Apify compute units. The bundled Wick engine is open source (MIT license).

For residential IP routing, you need [Wick Pro](https://getwick.dev) ($20/month).

## Links

- [Wick website](https://getwick.dev)
- [GitHub](https://github.com/wickproject/wick)
- [Documentation](https://getwick.dev/docs.html)
