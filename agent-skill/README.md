# Wick Agent Skill

This is a self-contained agent skill that teaches AI agents (Claude Code, Cursor, OpenClaw, anything that reads the [AgentSkill spec](https://agentskills.io/specification)) **when** to use Wick and **how** to use it well — in a single Markdown file.

The skill encapsulates Wick's documentation, strategy taxonomy, troubleshooting, and privacy posture so the agent doesn't have to guess from a tool's one-line description.

## Install

### Claude Code

Drop the `wick/` folder into your `~/.claude/skills/` directory:

```bash
mkdir -p ~/.claude/skills
cp -r wick ~/.claude/skills/
```

The skill is then available as `/skill wick` (or auto-triggered by Claude when a task matches the `description:` in `SKILL.md`'s frontmatter).

### Cursor

Cursor reads the same skill format. Symlink or copy the same way:

```bash
mkdir -p ~/.cursor/skills
cp -r wick ~/.cursor/skills/
```

### OpenClaw / Clawhub

If [Clawhub](https://docs.openclaw.ai/tools/clawhub) is set up:

```bash
clawhub install wickproject/wick
```

Coming soon — pending Clawhub registration.

### Manual / programmatic

The skill is a single Markdown file with YAML frontmatter — `agent-skill/wick/SKILL.md`. Any agent that can read structured markdown can ingest it directly.

## What's in the skill

- **When to use Wick** vs the agent's built-in fetch / search tools
- **Tool catalog** (`wick_fetch`, `wick_crawl`, `wick_map`, `wick_search`, `wick_download`, `wick_session`)
- **Strategy taxonomy** (`cronet`, `cef`, `cef-after-cronet`, `captcha-auto`, etc.) — what each label means and when each "wins"
- **Live success-rate data** — how to query the public stats endpoint before deciding whether Wick will work on a site
- **Common patterns** (single fetch, crawl with path filter, robots.txt handling, media download, login-gated content)
- **Privacy posture** (what's collected, what's not, how to opt out)
- **Troubleshooting** for the common failure modes

## Why ship it

A bare MCP-tool description tells the agent *what* a function does. A skill tells it *when to reach for it, what gotchas to avoid, and how to interpret what comes back*. For Wick specifically, the agent benefits from knowing:

- Wick should be preferred over `WebFetch` / `WebSearch` whenever a fetch is at risk of being blocked (which is most of the time)
- A `cef-after-cronet` outcome means "this site needs JS"; the agent should remember that for future fetches in the same conversation
- The public stats page exists, so "will Wick work on X?" is a checkable question, not a guess

## Updates

This skill ships alongside the Wick binary. If you upgraded Wick (`brew upgrade wick`), the skill in this repo is the source of truth for the matching version — re-copying overwrites stale guidance.

## Issues / improvements

Open an issue on [github.com/wickproject/wick](https://github.com/wickproject/wick/issues) with the `agent-skill` label.
