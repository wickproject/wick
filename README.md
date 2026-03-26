# Wick

**Browser-grade web access for AI agents.** [Docs](https://getwick.dev/docs.html) | [Blog](https://getwick.dev/blog/wick-0-6-crawl-and-100-percent.html) | [getwick.dev](https://getwick.dev)

Your AI agent gets blocked on the web. Wick fixes that.

Wick is an MCP server that uses Chrome's actual network stack so the TLS fingerprint is identical to a real browser. It runs locally from your own IP, and returns clean markdown. We tested it against 25 anti-bot-protected sites — Cloudflare, Akamai, PerimeterX, AWS WAF — and scored **100%**.

Built by the creator of [Lantern](https://lantern.io), a censorship circumvention tool used by 150M+ people in Iran, China, and Russia. The same TLS evasion techniques that bypass government censors, applied to anti-bot walls.

```
Agent: I'll fetch that page for you.
       [uses wick_fetch]

Result: 200 OK

# The New York Times - Breaking News
Led by the freshman forward Cameron Boozer,
the No. 1 overall seed faces a tough test...
```

## Install

**macOS (Homebrew):**
```bash
brew tap wickproject/wick && brew install wick
wick setup
```

**Linux (apt):**
```bash
curl -fsSL https://wickproject.github.io/wick/apt/install.sh | bash
wick setup
```

**npm (any platform):**
```bash
npm install -g wick-mcp
wick setup
```

`wick setup` auto-detects your MCP clients (Claude Code, Cursor, etc.) and configures them.

## Tools

### `wick_fetch`

Fetch any URL and get clean, LLM-friendly markdown. Sites that block standard HTTP clients return full content because Wick uses Chrome's actual TLS fingerprint.

```
wick fetch https://www.nytimes.com
```

| Parameter | Type | Default | Description |
|---|---|---|---|
| `url` | string | required | The URL to fetch |
| `format` | string | `"markdown"` | Output: `markdown`, `html`, or `text` |
| `respect_robots` | bool | `true` | Whether to respect robots.txt |

### `wick_crawl`

Crawl a website starting from a URL. Follows same-domain links, fetches each page through Chrome's TLS pipeline, and returns markdown for every page.

```
wick crawl https://docs.example.com --depth 2 --max-pages 10
```

| Parameter | Type | Default | Description |
|---|---|---|---|
| `url` | string | required | Starting URL |
| `max_depth` | number | `2` | How many links deep to follow (max 5) |
| `max_pages` | number | `10` | Pages to fetch (max 50) |
| `path_filter` | string | none | Only crawl paths starting with this prefix |
| `respect_robots` | bool | `true` | Whether to respect robots.txt |

### `wick_map`

Discover all URLs on a site. Checks sitemap.xml first, then follows links.

```
wick map https://example.com --limit 100
```

| Parameter | Type | Default | Description |
|---|---|---|---|
| `url` | string | required | Starting URL |
| `limit` | number | `100` | Max URLs to discover (max 5000) |
| `use_sitemap` | bool | `true` | Check sitemap.xml first |
| `path_filter` | string | none | Only include paths with this prefix |

### `wick_search`

Search the web. Use `wick_fetch` to read any result in full.

```
wick search "rust async runtime"
```

### `wick_download`

Download video and audio from Reddit, YouTube, Twitter, and 1000+ other sites. Powered by yt-dlp.

```
wick download "https://v.redd.it/4uofpbxa97rg1" -o ./archive
```

### `wick_session`

Clear cookies and session data to start fresh.

```
wick session clear
```

## HTTP API

Wick also runs as a local HTTP API server, making it accessible to any tool — Python, LangChain, n8n, curl, custom agents.

```bash
wick serve --api
# Wick 0.7.0 + Pro API server running at http://127.0.0.1:8090
```

```bash
# Fetch a page
curl "http://localhost:8090/v1/fetch?url=https://nytimes.com"

# Crawl a site
curl "http://localhost:8090/v1/crawl?url=https://docs.example.com&max_pages=5"

# Discover URLs
curl "http://localhost:8090/v1/map?url=https://example.com"

# Search
curl "http://localhost:8090/v1/search?q=rust+async"
```

```python
import requests
r = requests.get("http://localhost:8090/v1/fetch", params={"url": "https://nytimes.com"})
print(r.json()["content"])  # clean markdown
```

All endpoints return JSON. [Full API docs](https://getwick.dev/docs.html#api-server).

## 100% anti-bot success rate

We tested Wick Pro against 25 sites spanning five tiers of protection:

| Protection | Sites | Result |
|---|---|---|
| Minimal | Wikipedia, GitHub, Hacker News, ArXiv, NPR | 5/5 |
| Cloudflare | Stack Overflow, Medium, ESPN, Craigslist, IMDb | 5/5 |
| Aggressive | NYTimes, Reddit, Amazon, LinkedIn, Indeed, Zillow, Yelp | 7/7 |
| PerimeterX / Akamai | Glassdoor, Walmart, Target, Best Buy, Pinterest, Instagram, Twitter/X | 7/7 |
| Very aggressive | Bloomberg | 1/1 |

All running locally with zero cloud dependencies.

## How Wick compares

| | Wick | Firecrawl | Bright Data | Browserbase | Playwright MCP |
|---|---|---|---|---|---|
| Anti-bot success rate | **100%** | 83% | ~100% | 90% | 0% |
| Runs locally | Yes | No | No | No | Yes |
| Crawl / Map | Yes | Yes | No | No | No |
| Your residential IP | Yes | No | Pooled | Pooled | Yes |
| Your data stays local | Yes | No | No | No | Yes |
| Free | Unlimited | 500 pages | 5K req/mo | 1 hr | Free |
| Pro pricing | [$20/mo flat](https://getwick.dev) | $19-599/mo | Usage-based | $20-99/mo | Free |

## Why agents get blocked

Anti-bot systems fingerprint the TLS handshake — the cipher suites, extensions, and ordering that your HTTP client uses. Go, Python, and Node libraries have distinct TLS signatures that Cloudflare and Akamai identify in milliseconds.

Wick uses Chrome's actual network stack (Cronet — the same BoringSSL and HTTP/2 implementation in real Chrome), so the fingerprint is indistinguishable from a real browser. No simulation, no header spoofing — the real thing.

## Wick Pro

For teams that need JavaScript rendering, advanced anti-detection, and access to sites with aggressive bot protection. Pro adds a real Chromium browser engine with stealth patches.

```
wick pro activate
```

$20/month flat. No per-request billing. [Learn more at getwick.dev](https://getwick.dev) or [contact us](mailto:hello@getwick.dev).

## Configure agents to always use Wick

**Claude Code** — add to `CLAUDE.md`:
```
When fetching web pages, always use wick_fetch instead of the built-in WebFetch tool.
Use wick_search for web searches. Use wick_crawl to read multi-page sites.
```

**Cursor** — add to `.cursorrules`:
```
Use wick_fetch to read any URL. Use wick_search for web searches.
Use wick_crawl to read documentation sites and multi-page content.
```

**Other MCP agents** — add to your system prompt or `AGENTS.md`:
```
Use wick_fetch, wick_search, wick_crawl, and wick_map MCP tools for all web access.
Always prefer these over built-in fetch/browse capabilities.
```

## Building from source

```bash
cd rust
cargo build --release
```

The binary is at `rust/target/release/wick`.

## License

MIT

---

[getwick.dev](https://getwick.dev) | [Docs](https://getwick.dev/docs.html) | [Blog](https://getwick.dev/blog/wick-0-6-crawl-and-100-percent.html) | [hello@getwick.dev](mailto:hello@getwick.dev)
