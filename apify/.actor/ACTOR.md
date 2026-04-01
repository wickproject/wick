# Wick Web Fetcher

A lightweight content extraction Actor powered by [Wick](https://getwick.dev), an open-source tool that uses Chrome's real network stack (Cronet) to fetch web pages. Because requests go through the same TLS implementation as a real Chrome browser (BoringSSL, HTTP/2, QUIC), Wick reaches sites that block raw HTTP clients.

## When to use this Actor

- **Quick single-page fetches** where spinning up a full browser is overkill
- **LLM and RAG pipelines** that need clean markdown from web pages
- **Lightweight content extraction** at low memory cost (256 MB)
- **Complement to browser-based Actors** -- use Wick for the pages that don't need JS rendering, save browser compute for the pages that do

## How it works

Under the hood, this Actor runs the Wick binary as a local HTTP API server inside the container. Wick makes requests using [Cronet](https://chromium.googlesource.com/chromium/src/+/master/components/cronet/) -- Chrome's network stack extracted as a standalone library. The response HTML is converted to clean markdown, stripping navigation, ads, and boilerplate.

No headless browser is launched. This makes it fast (~1-3s per page) and lightweight (256 MB vs typical 1-4 GB for browser-based Actors).

## Getting started

Run the Actor with a list of URLs:

```json
{
    "urls": ["https://www.nytimes.com", "https://docs.example.com"],
    "mode": "fetch",
    "format": "markdown"
}
```

Or crawl a whole site:

```json
{
    "urls": ["https://docs.example.com"],
    "mode": "crawl",
    "maxDepth": 2,
    "maxPages": 20
}
```

Results appear in the **Output** tab as a table. Each row is one page with its URL, title, content, status code, and timing.

## Modes

### Fetch (default)

Fetches one or more URLs and returns clean content. Each URL becomes one row in the output dataset with title, content, status code, and timing.

### Crawl

Starts from a URL and follows same-domain links. Returns content for every page discovered, each as a separate dataset row. Control depth (1-5) and max pages (1-50).

### Map

Discovers all URLs on a site by checking sitemap.xml and following links. Returns a URL list without fetching content -- useful for planning a targeted crawl or building a sitemap.

## Output

Each dataset row contains:

| Field | Description |
|-------|-------------|
| `url` | The URL that was fetched |
| `title` | Page title |
| `content` | Page content in markdown, HTML, or plain text |
| `statusCode` | HTTP response status |
| `timingMs` | Fetch duration in milliseconds |
| `format` | Output format used |
| `fetchedAt` | ISO 8601 timestamp |

## Residential IP mode (optional)

For additional anti-detection, you can connect this Actor to your own Wick instance running on your machine. Requests then route through your residential IP, combining Apify's scheduling and monitoring with your own network.

1. Install [Wick Pro](https://getwick.dev) on your machine
2. Start the API server: `wick serve --api`
3. Expose it via a tunnel (Cloudflare Tunnel, ngrok, etc.)
4. Enter the tunnel URL in the **Wick Tunnel URL** input field

## Limitations

- **No JavaScript rendering** in the bundled engine. For JS-heavy SPAs, pair this Actor with a browser-based Actor like [Website Content Crawler](https://apify.com/apify/website-content-crawler) or use Wick's tunnel mode with a Pro instance that includes JS rendering.
- **Best for content pages.** Wick excels at articles, documentation, blogs, and product pages. For structured data extraction (e.g., specific fields from a listing), consider combining Wick's output with an LLM or a purpose-built scraper.

## Integrations

Wick's output works with Apify's built-in integrations. Some ideas:

- **Pinecone / Qdrant / PGVector** -- Crawl a docs site, then push the markdown straight into a vector database for RAG.
- **OpenAI Vector Store** -- Feed crawled content to an OpenAI Assistant.
- **Google Sheets** -- Export fetched pages to a spreadsheet for review.
- **Zapier / Make / n8n** -- Trigger downstream workflows when a crawl finishes.

Set these up from the **Integrations** tab on your Actor run page.

## Cost estimate

This Actor uses 256 MB of memory and runs fast, so compute costs are low:

| Task | Approximate cost |
|------|-----------------|
| Fetch 10 URLs | ~$0.001 |
| Crawl 50 pages | ~$0.005 |
| Map a site (100 URLs) | ~$0.001 |

You only pay for Apify compute units. The Wick engine is open source ([MIT license](https://github.com/wickproject/wick)).

Residential IP mode requires [Wick Pro](https://getwick.dev) ($20/month).

## Resources

- [Wick documentation](https://getwick.dev/docs.html)
- [GitHub repository](https://github.com/wickproject/wick)
- [How Wick's TLS fingerprinting works](https://getwick.dev/blog/why-your-ai-agent-cant-read-the-web.html)
