# 0001: Bring your own agent

- Status: accepted
- Date: 2026-09-26

## Context

Users of Slate are expected to already pay for Claude Code or Codex. Anthropic's consumer terms (revised 2026-02, enforced 2026-04) restrict subscription OAuth to Claude.ai, Claude Code and the desktop app; the Agent SDK and any third-party harness must use an API key. OpenAI's terms for ChatGPT-plan Codex use are similar in spirit.

## Decision

Slate never calls a model API and never holds model credentials. It launches the vendor's official binary and integrates only through that binary's documented extension surfaces: headless/exec modes, hooks, MCP servers, skills, permission tools and config files.

## Consequences

- slash owns no agent loop; it is a router and renderer.
- All OS integration is delivered as MCP servers and hooks, which both vendors support.
- Backend adapters are small and swappable. A policy change by either vendor touches one adapter.
- Slate cannot use features a vendor does not expose. In particular, Linux computer use must come from slate-desktop, not from the vendor.
- Rate limits are the user's subscription limits. Design must minimise tokens (a11y trees over screenshots, skills over exploration).
