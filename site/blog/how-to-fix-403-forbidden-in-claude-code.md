# How to Fix 403 Forbidden in Claude Code

You're using Claude Code, and you ask it to read a webpage. API docs, a pricing page, a blog post, a news article. It tries:

```
Agent: I'll fetch that page for you.

Result: 403 Forbidden

Agent: I'm sorry, I wasn't able to access that page.
       The site appears to be blocking automated requests.
```

This happens dozens of times a day. Claude Code can write code, debug systems, manage git repos — but it can't read a webpage. Here's why, and how to fix it in 30 seconds.

## Why Claude Code gets 403 errors

It's not about robots.txt. It's about **TLS fingerprinting**.

When Claude Code fetches a URL, it uses a standard HTTP client. Anti-bot systems like Cloudflare, Akamai, and Fastly inspect the TLS handshake — the very first message your connection sends — and compare it to known browser signatures.

Chrome's TLS handshake looks different from Go's, Python's, or Node's. The cipher suites, extensions, and their ordering create a unique fingerprint. Anti-bot systems identify non-browser clients in milliseconds, before the HTTP request even reaches the server.

That's your 403. The website never saw your request. The anti-bot layer killed it at the TLS level.

**Sites that commonly block Claude Code:**
- The New York Times (Akamai)
- Reddit (custom anti-bot)
- Cloudflare-protected sites (~20% of the web)
- Most news sites, financial sites, and documentation behind CDNs

## The fix: Wick

[Wick](https://getwick.dev) is a free, open-source MCP server that gives Claude Code browser-grade web access. It uses Chrome's actual network stack — the same BoringSSL, HTTP/2, and QUIC implementation that real Chrome uses. The TLS fingerprint is identical to a regular browser.

### Install (30 seconds)

**macOS:**
```bash
brew tap wickproject/wick && brew install wick
wick setup
```

**Linux:**
```bash
curl -fsSL https://wickproject.github.io/wick/apt/install.sh | bash
wick setup
```

**npm (any platform):**
```bash
npm install -g wick-mcp
wick setup
```

`wick setup` auto-detects Claude Code and configures it. That's it.

### Make Claude Code always use Wick

Add this to your project's `CLAUDE.md`:

```markdown
When fetching web pages, always use the wick_fetch MCP tool instead of
the built-in WebFetch tool. wick_fetch bypasses anti-bot protection and
returns cleaner content. Use wick_search for web searches.
```

Now when Claude Code needs to read a webpage, it reaches for `wick_fetch` instead of its built-in fetch that gets blocked.

## What Wick returns

Instead of a 403 error, your agent gets clean markdown:

```
Agent: I'll fetch that article using wick_fetch.

# The New York Times - Breaking News

Led by the freshman forward Cameron Boozer,
the No. 1 overall seed faces a tough test in
the NCAA tournament...
```

Clean, structured content your agent can actually reason about. Not raw HTML, not a wall of `<div>` tags — markdown with headings, links, and paragraphs.

## How it works

Wick runs as a local MCP server on your machine. When Claude Code calls `wick_fetch`:

1. The request goes through Chrome's actual network stack
2. The TLS fingerprint matches a real Chrome browser
3. The request exits from your residential IP (not a datacenter)
4. Anti-bot systems see a normal browser visit
5. The HTML response is extracted into clean markdown
6. Claude Code gets the content

No cloud service. No proxy. No API key. Everything runs locally.

## What about sites that need JavaScript?

Some sites (Reddit, SPAs, financial exchanges) require JavaScript to render content. The free tier handles most of the web, but for JS-heavy sites, [Wick Pro](https://getwick.dev) ($20/month) adds a full browser engine that renders JavaScript, handles CAPTCHAs automatically, and bypasses even aggressive anti-bot systems.

Run `wick pro activate` to upgrade. The free tier automatically detects when Pro would help and suggests it.

## Other tools Wick provides

**`wick_search`** — Search the web from Claude Code. Returns titles, URLs, and snippets. Then use `wick_fetch` to read any result in full.

**`wick_session`** — Clear cookies and session data to start fresh.

## Quick reference

| Problem | Solution |
|---|---|
| 403 Forbidden | Install Wick: `brew install wick && wick setup` |
| Claude uses built-in fetch | Add CLAUDE.md instructions (see above) |
| JS-rendered pages blank | `wick pro activate` for JS rendering |
| robots.txt blocking | `wick fetch URL --no-robots` |
| Need raw HTML | `wick fetch URL --format html` |

## Links

- **Install:** `brew tap wickproject/wick && brew install wick`
- **Docs:** [getwick.dev/docs.html](https://getwick.dev/docs.html)
- **GitHub:** [github.com/wickproject/wick](https://github.com/wickproject/wick)
- **Pro:** [getwick.dev](https://getwick.dev) ($20/month for JS rendering + advanced anti-detection)

---

*Wick is built by [Adam Fisk](https://github.com/adamfisk), creator of [Lantern](https://lantern.io) — a censorship circumvention tool used by 150 million people in Iran, China, and Russia. The same techniques that bypass government censors now bypass anti-bot walls.*
