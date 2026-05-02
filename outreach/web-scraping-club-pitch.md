# Pitch — The Web Scraping Club

**Target:** Pierluigi Vinciguerra (substack.thewebscraping.club)
**Goal:** A piece on Wick's positioning relative to the agentic-browser category — riding the LAB #103 framing rather than asking for a head-to-head bench.
**Channel:** Email or Substack reply.
**Status:** Draft — not yet sent.

---

## Why this version, not the head-to-head version

The previous draft of this pitch led with "would you do a hands-on Wick review." After reading **LAB #103: Bypassing DataDome-Protected Websites in the Agentic Era** (Apr 30, 2026), that ask is the wrong one:

- 11 of 14 contestants in his bench averaged 1.0/4 on the same task.
- Only Bright Data Browser API consistently passed (3.6/4 mean).
- The differentiator was **not** the browser — it was Bright Data's curated residential-IP rotation + actively-maintained stealth patches. They own both halves of the stack.
- We don't. Wick uses Cronet + CEF, but ships no managed proxy network and no week-by-week stealth-patch team. On `leroymerlin.fr` we'd land in the middle of his table.

Asking him to add Wick as a 15th row is a losing pitch. Asking him to write about a *different* fork in the road is a winning one.

---

## Subject line

**Wick — the fork in the road LAB #103 didn't cover**

Specific, references his work, opens a curiosity gap. Avoid "review request" language entirely.

---

## Body

Hi Pierluigi,

I read LAB #103 yesterday — your DataDome shootout. The honest framing in there ("each anti-bot needs its own answer, and the answer changes every quarter") is the cleanest summary I've seen of where this field actually is.

I'm Adam Fisk — former lead engineer at LimeWire, creator of [Lantern](https://getlantern.org). I've been building **Wick** ([getwick.dev](https://getwick.dev) / [github.com/wickproject/wick](https://github.com/wickproject/wick)) and I want to flag a structural angle your shootout didn't cover, not because I think Wick would have won, but because I think Wick took a different fork.

**The shootout's contestants converged on one architecture:** managed cloud browser + residential proxy + actively-maintained stealth patches. That's the category Hyperbrowser, Browserbase, Browser Use, Browserless, ZenRows, Bright Data Browser API, RayoBrowse all live in. Bright Data won your bench because they own all three halves of that stack. The others don't, and they fell to 1.0/4.

**Wick took a different fork:**

1. **Local MCP server, not a managed cloud browser.** `brew install wick` and Claude Code / Cursor pick it up via stdio. No fleet of cloud Chromiums to maintain, no per-session billing. The user's own machine *is* the runtime.
2. **User's own residential IP, by default.** No proxy is required to get reasonable success on most sites. We support `--proxy socks5://...` for users who want it, but the home base is "your IP, your reputation."
3. **Cronet, not curl_cffi.** We link `libcronet.so` — the actual Chromium network stack — rather than emulating the TLS fingerprint with a separate library. Bit-for-bit match because it's literally Chrome's code making the connection.
4. **CEF, not CDP.** When sites need JS, our renderer is embedded Chromium without the Chrome DevTools Protocol attached, so none of the CDP automation artifacts (`navigator.webdriver`, runtime hooks Cloudflare specifically checks for) are present. I wrote up that thesis [here](https://getwick.dev/blog/cef-vs-cdp.html) — it's relevant because Pydoll, Browser Use CDP, Scrapling DynamicFetcher, and Browserless in your bench are all CDP-driven and all averaged 1.0/4. CDP isn't necessary-and-sufficient evidence, but it correlates with failing.
5. **Public, live per-host success rates.** Every Wick fetch posts an anonymous `{host, strategy, ok, status, timing_ms}` envelope (no URL paths, no content, no IP persisted). The 7-day rollup is at [getwick.dev/stats.html](https://getwick.dev/stats.html), no auth, refreshed every 5 minutes. So "does Wick work on `<site>`" is a checkable question, not a vendor claim.

**Where Wick honestly lands on your bench**: I ran one quick local test against `leroymerlin.fr` to set realistic expectations. From a previously-used IP, DataDome served the standard `captcha-delivery.com` interstitial on the homepage. Indistinguishable from what most of your contestants saw. With a fresh residential IP I'd expect us to clear the homepage (~1/4) like Pydoll, Camoufox, Browserbase, Browser Use, Browserless. We'd not catch Bright Data Browser API. They earned that lead with curated IP rotation and a dedicated stealth team; we have neither.

**The piece I think would be worth writing** — and you have the audience for it — is not "Wick on `leroymerlin.fr`." It's the structural one: **what does it mean that the agentic-browser category has converged on a single architecture, what's the alternative fork (local MCP / user's own IP / public success-rate transparency), and what does the live data say about which class of anti-bot is gaining ground?** That's a beat your shootout opens up but doesn't close, and it's a beat I have data for.

Three concrete offers, pick whichever (or none):

1. **Background interview** — I can deep-dive Cronet vs curl_cffi internals, the CEF-vs-CDP delta, and why Wick publishes per-host success rates. 30 minutes; no slides; you can quote freely.
2. **Live-stats column** — `getwick.dev/stats.html` is updating in real time. I can build you a custom slice (e.g. "DataDome-protected hosts: 7-day Wick success rate") so you have a recurring data source on which anti-bot vendors are tightening vs loosening over time.
3. **Guest post** — if you're pressed for content but interested in the architectural framing, I can write the piece (~1500 words, in your voice/format) and you publish or kill it as you like.

Either way, thanks for the work you do. The Scrapling and DataDome pieces in particular were the most honest writeups in the field this year.

Best,
Adam Fisk
hello@getwick.dev
[github.com/wickproject/wick](https://github.com/wickproject/wick)

---

## Notes for sending

- Send from your personal address (`afisk@…`), not `hello@getwick.dev`. Founders responding to founders work.
- **Don't attach anything.** Links only. Anything else looks like a press kit.
- **One follow-up, max 10 days later.** Reference the live-stats page being meaningfully populated by then ("real-world evidence"). After that, drop it.
- **If he asks for the bench result anyway**, run it from a clean residential IP first. The Bright Data residential pool with FR exit is what his contestants used, so we'd be fighting on the same ground. Goal isn't to beat Bright Data; goal is to land 1/4 honestly without being filtered at the TCP layer the way our burned-IP test was.
- **If he says yes to offer #2 (live-stats column)**, that's the highest-leverage outcome. A recurring data source he uses keeps Wick in the newsletter monthly without Wick having to win shootouts.
