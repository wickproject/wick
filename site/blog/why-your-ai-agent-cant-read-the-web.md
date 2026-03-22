# Why Your AI Agent Can't Read the Web (And How to Fix It)

You're using Claude Code, Cursor, or another AI coding agent. You ask it to read a webpage — API docs, a pricing page, a blog post. It tries. And fails.

```
Agent: I'll fetch that page for you.

Result: 403 Forbidden

Agent: I'm sorry, I wasn't able to access that page.
       The site appears to be blocking automated requests.
       You could try copying the content and pasting it here...
```

Sound familiar? This happens dozens of times a day for developers using AI agents. The agent can write code, debug systems, manage git repos — but it can't read a webpage.

## Why agents get blocked

It's not about robots.txt or rate limiting. It's about **fingerprinting**.

When your agent makes an HTTP request, the website sees something very different from what it sees when you visit in Chrome:

- **Different network signature.** The agent uses Go or Python's HTTP library. Cloudflare, Fastly, and Akamai can distinguish these from real browsers in milliseconds — before the request even reaches the server.

- **No browser signals.** Real browsers send dozens of headers that identify them: `Sec-Ch-UA`, `Sec-Fetch-Mode`, `Accept-Language`, specific cookie behaviors. Agents send none of these, or send them incorrectly.

- **Datacenter vs. residential.** If your agent runs in the cloud, it's coming from an IP range that anti-bot systems have already flagged.

The result: your agent gets blocked on sites you can visit effortlessly in your browser. The New York Times. Reddit. Cloudflare-protected API docs. Even documentation sites for the tools you're building with.

## The irony

The human is right there. You're the one asking the agent to read the page. You have a browser. You have cookies. You have a residential IP. You're not a bot — you're a developer trying to get work done.

But there's no way to share your "human-ness" with your agent. Until now.

## Wick: browser-grade access for AI agents

[Wick](https://getwick.dev) is a free, open-source MCP server that gives your AI agent the same web access you have.

It runs locally on your machine. When your agent needs to fetch a page, Wick handles the request using the same networking technology as real browsers — not a simulation, not a wrapper, the actual implementation. The request goes out from your own IP, with the same signature Chrome would produce.

```
Agent: I'll fetch that page for you.
       [uses wick_fetch]

Result: 200 OK · 340ms

# The New York Times - Breaking News

Led by the freshman forward Cameron Boozer,
the No. 1 overall seed faces a tough test...
```

Same page. Same URL. Different tool.

## What you get

**`wick_fetch`** — Fetch any URL and get clean, LLM-friendly markdown. Strips boilerplate, navigation, and ads. Your agent gets the content, not the HTML soup.

**`wick_search`** — Search the web directly from your agent. Returns titles, URLs, and snippets. Then use `wick_fetch` to read any result in full. Your agent can now research topics, not just read links you give it.

**CAPTCHA handling** — When a site serves a CAPTCHA, Wick shows you a notification. You solve it (5 seconds), and your agent continues automatically. Because you're the human the CAPTCHA is looking for. No third-party solving service, no cloud, no cost.

## Install in 30 seconds

**macOS:**
```bash
brew tap myleshorton/wick && brew install wick
wick setup
```

**Linux:**
```bash
curl -fsSL https://myleshorton.github.io/wick/apt/install.sh | bash
wick setup
```

**npm (any platform):**
```bash
npm install -g wick-mcp
wick setup
```

That's it. Your agent now has `wick_fetch` and `wick_search`. No configuration. No API keys. No cloud service.

## What makes Wick different

There are other tools in this space — Firecrawl, Bright Data MCP, Browserbase, Playwright. Here's why Wick is different:

**Local-first.** Wick runs on your machine. Your data never passes through a cloud service. Every alternative routes your traffic through their servers.

**Your IP.** Requests come from your residential connection. No pooled proxy IPs with sketchy histories. No datacenter IPs that are pre-flagged. To the website, it looks like you browsing normally — because it basically is.

**Authentic, not mimicked.** Most tools try to *look* like a browser by faking headers. Anti-bot systems see through this in milliseconds. Wick uses the same networking technology that real browsers use. There's no difference to detect because there is no difference.

**Free forever.** The core tool is open source and costs nothing. It runs entirely on your hardware. No usage limits, no trial period, no credit card.

**Works everywhere.** macOS, Linux, any MCP client. Homebrew, apt, npm — install however you prefer.

| | Wick | Firecrawl | Bright Data | Browserbase | Playwright MCP |
|---|---|---|---|---|---|
| Anti-bot bypass | Yes | No | Partial | No | No |
| Runs locally | Yes | No | No | No | Yes |
| Your residential IP | Yes | No | Pooled | No | Yes |
| Clean markdown | Yes | Yes | Yes | No | No |
| Web search | Yes | No | No | No | No |
| Free tier | Forever | 500 pages | 5K req | Trial | Free |

## For companies that need more

The free version handles the majority of the web. But some companies need access to sites that go beyond what a Chrome fingerprint alone can handle — JavaScript-heavy single-page apps, sites with advanced bot detection, or pages that require rendering before content is available.

**Wick Pro** is a bespoke service for companies accessing high-value data:

- Full JavaScript rendering for dynamic pages
- Advanced anti-detection that's continuously updated
- Residential IP tunneling from cloud servers
- Custom configuration per client
- CAPTCHA handling (automated and human-in-the-loop)
- Dedicated support with SLA

We work with teams in financial data, competitive intelligence, compliance, market research, and other sectors where reliable web access is critical — not mass scraping.

**Interested?** [Contact us](mailto:hello@getwick.dev). We'll set up a call to understand what you need.

## Try it now

```bash
brew tap myleshorton/wick && brew install wick && wick setup
```

Then ask your agent to read a webpage. Any webpage.

---

*Check us out at [getwick.dev](https://getwick.dev) or on [GitHub](https://github.com/myleshorton/wick).*
