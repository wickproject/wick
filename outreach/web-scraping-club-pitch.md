# Pitch — The Web Scraping Club

**Target:** Pierluigi Vinciguerra (substack.thewebscraping.club)
**Goal:** Hands-on Wick review, similar in shape to the Scrapling piece in Nov 2025.
**Channel:** Email (substack reply or whatever direct contact is on his Substack profile).
**Status:** Draft — not yet sent.

---

## Subject lines (pick one)

1. Wick — what Cronet + CEF gets you that curl_cffi + Playwright can't
2. A Rust MCP server that uses Chrome's actual network stack (not an impersonator)
3. Hands-on review request: Wick (open-source, MIT)

I'd lead with #1 — it's specific and tells the reader exactly what's interesting in the first six words.

---

## Body

Hi Pierluigi,

Adam Fisk here — former lead engineer at LimeWire, creator of [Lantern](https://getlantern.org). I read your Scrapling deep-dive in November and thought it might be worth flagging a project I've been shipping in the same neighborhood: **Wick** ([getwick.dev](https://getwick.dev) / [github.com/wickproject/wick](https://github.com/wickproject/wick)).

What I think is genuinely worth a look, in three points:

1. **Cronet, not curl_cffi.** Wick links the actual Chromium network stack (BoringSSL + libcronet) rather than emulating Chrome's TLS fingerprint with a separate library. The fingerprint match is bit-for-bit because it's literally Chrome's code making the connection. Most TLS-impersonation libs are an arms race; this one isn't.

2. **CEF, not CDP.** When sites need JS, Wick falls back to an embedded Chromium (CEF) renderer rather than driving Chrome over the Chrome DevTools Protocol the way Playwright/Puppeteer/Browserbase do. CDP leaves automation artifacts on the page (`navigator.webdriver`, CDP runtime hooks Cloudflare specifically checks for); CEF doesn't expose any of them. Wrote up the technical case here: [getwick.dev/blog/cef-vs-cdp.html](https://getwick.dev/blog/cef-vs-cdp.html).

3. **Public, live success rates.** Every fetch posts an anonymous `{host, strategy, ok, status, timing_ms}` envelope (no URL paths, no content, no IP persisted). The 7-day rollup is published at [getwick.dev/stats.html](https://getwick.dev/stats.html) — public, no auth. So "does Wick actually work on `<site>`" stops being something a vendor claims and becomes something you can read off a page. I'm not aware of another scraper that publishes its real-world success rate this way; I'd be curious whether that resonates with your readers.

Other context that might matter for a hands-on review:
- **MCP-native, single binary.** `brew install wick` and Claude Code/Cursor pick it up automatically via `wick setup`. No Python, no `pip install scrapling[ai]`, no separate process.
- **Local-by-default.** Requests exit your residential IP, not a datacenter one — which is half the reason Scrapling's README has nine proxy sponsors.
- **MIT, fully open-source.** No Pro tier, no API gates. Everything (Cronet, CEF, stealth patches, auto-CAPTCHA, residential-tunnel tools) is in the public repo and ships in the free binary.

Where Wick is honestly behind Scrapling — I'd rather flag this than have you find it: no Scrapy-style spider framework with checkpointing yet, no built-in proxy rotation pool, no DoH-with-proxy. Those are on the roadmap as direct copy-good-ideas after reading the Scrapling source.

If any of this is interesting enough to merit a piece, I'd be happy to:
- Send you a quick install + 10-line example tailored to whatever your usual benchmark sites are
- Answer technical questions on a call (I can deep-dive Cronet vs curl_cffi internals)
- Pre-load `wick install cef` for you so the heavy step is done before you start

Either way, thanks for the work you do — your Scrapling piece was the most honest review I've read in a while, and the field needs more of that.

Best,
Adam Fisk
hello@getwick.dev
github.com/wickproject/wick

---

## Notes for sending

- **Don't send from a generic alias.** Send from your personal `afisk@…` address (his readers respond to founders, not support inboxes).
- **Don't bcc anyone.** Solo reach-out reads better.
- **If he doesn't reply in 10 days**, one polite follow-up referencing the live-stats page being meaningfully populated by then. After that, drop it — he gets a lot of these.
- **If he says yes**, offer to write a guest post angle ("how Cronet differs from curl_cffi") if he's pressed for time — costs you nothing and gives him a free piece for the newsletter.
