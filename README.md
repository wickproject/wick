# Wick

**Browser-grade web access for AI agents.**

Your AI agent gets blocked on the web. Wick fixes that.

Wick is an MCP server that gives AI coding agents (Claude Code, Cursor, Windsurf, etc.) the same web access their human operators have. It runs locally on your machine, uses Chrome's actual network stack, and returns clean markdown.

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

`wick setup` auto-detects your MCP clients (Claude Code, Cursor, etc.) and configures them. That's it — your agent now has `wick_fetch` and `wick_search`.

## Tools

### `wick_fetch`

Fetch any URL and get clean, LLM-friendly markdown. Strips navigation, ads, and boilerplate.

```
wick fetch https://www.nytimes.com
```

Sites that block standard HTTP clients (Cloudflare, Akamai, etc.) return 200 with full content because Wick uses Chrome's actual TLS fingerprint.

**Parameters:**
| Name | Type | Default | Description |
|------|------|---------|-------------|
| `url` | string | required | The URL to fetch |
| `format` | string | `"markdown"` | Output format: `markdown`, `html`, or `text` |
| `respect_robots` | bool | `true` | Whether to respect robots.txt |

### `wick_search`

Search the web and get structured results. Then use `wick_fetch` to read any result in full.

```
wick search "rust async runtime"
```

Returns titles, URLs, and snippets for each result.

**Parameters:**
| Name | Type | Default | Description |
|------|------|---------|-------------|
| `query` | string | required | Search query |
| `num_results` | number | `5` | Number of results (1-20) |

### `wick_session`

Clear cookies and session data to start fresh.

```
wick session clear
```

## Why agents get blocked

When your agent makes an HTTP request, anti-bot systems see a completely different fingerprint than a real browser:

- **TLS fingerprint** — Go, Python, and Node HTTP libraries have distinct TLS signatures. Cloudflare and Akamai identify them in milliseconds.
- **Missing browser headers** — Real browsers send `Sec-Ch-UA`, `Sec-Fetch-Mode`, and dozens of other headers. Agents send none or send them wrong.
- **Datacenter IPs** — Cloud-hosted agents come from IP ranges that anti-bot systems have pre-flagged.

Wick solves this by using Chrome's actual network stack (not a simulation), running locally from your own residential IP.

## How it works

Wick runs as an MCP server on your machine. When your agent calls `wick_fetch`:

1. The request goes through Chrome's network stack (BoringSSL, HTTP/2, QUIC)
2. The TLS fingerprint is identical to a real Chrome browser
3. The request exits from your residential IP
4. The response HTML is extracted into clean markdown
5. Your agent gets the content, not the HTML soup

No cloud service. No proxy. No API key. Everything runs locally.

## CLI usage

Wick also works as a standalone CLI:

```bash
# Fetch a page
wick fetch https://example.com

# Fetch as HTML
wick fetch https://example.com --format html

# Search the web
wick search "latest rust features" --num 10

# Ignore robots.txt
wick fetch https://reddit.com/r/technology --no-robots

# Start as MCP server (used by agents)
wick serve --mcp
```

## What makes Wick different

| | Wick | Firecrawl | Bright Data | Browserbase | Playwright MCP |
|---|---|---|---|---|---|
| Anti-bot bypass | Yes | No | Partial | Paid | No |
| Runs locally | Yes | No | No | No | Yes |
| Your residential IP | Yes | No | Pooled | Pooled | Yes |
| Clean markdown | Yes | Yes | Yes | No | No |
| Web search | Yes | No | No | No | No |
| Your data stays local | Yes | No | No | No | Yes |
| Free | Forever | 500 pages | 5K req | 1 hr | Free |
| Geo-restriction bypass | Pro | No | Paid | Paid | No |

## Wick Pro

For companies that need JavaScript rendering, advanced anti-detection, geo-restriction bypass, and access to sites with aggressive bot protection.

[Learn more at getwick.dev](https://getwick.dev) or [contact us](mailto:hello@getwick.dev).

## Configuration

`wick setup` writes MCP configuration to:
- **Claude Code:** `~/.claude/claude_desktop_config.json`
- **Cursor:** `~/.cursor/mcp.json`

Manual configuration:
```json
{
  "mcpServers": {
    "wick": {
      "command": "wick",
      "args": ["serve", "--mcp"]
    }
  }
}
```

### Making agents always use Wick

By default, agents use their built-in fetch (which gets blocked). To make them prefer Wick, add instructions to your project's config:

**Claude Code** — add to your `CLAUDE.md`:
```markdown
When fetching web pages, always use the wick_fetch MCP tool instead of
the built-in WebFetch tool. wick_fetch bypasses anti-bot protection and
returns cleaner content. Use wick_search for web searches.
```

**Cursor** — add to `.cursorrules`:
```
When you need to read a webpage or fetch a URL, use the wick_fetch tool.
When you need to search the web, use the wick_search tool.
```

**Other MCP agents** — add to your system prompt, `AGENTS.md`, or equivalent instructions file:
```
You have access to wick_fetch and wick_search MCP tools for web access.

- Use wick_fetch to read any URL. It returns clean markdown and bypasses
  anti-bot protection that blocks standard HTTP requests.
- Use wick_search to search the web. Returns titles, URLs, and snippets.
- Always prefer these tools over built-in fetch/browse capabilities.
```

This ensures your agent reaches for Wick automatically instead of failing with 403 errors.

## Building from source

```bash
cd rust
cargo build --release
```

The binary is at `rust/target/release/wick`.

## License

MIT
