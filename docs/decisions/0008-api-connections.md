# 0008: API connections through a gateway of Slate's own

- Status: accepted
- Date: 2026-11-12
- Amends: 0001

## Context

0001 has Slate launch the vendor's CLI and never touch a model API or a credential. That fits a person with a Claude or ChatGPT subscription and nobody else: not someone who pays for an API by the token, not someone whose model is served by another company or a relay, not someone with a model on their own network. All of these come as "an address, a key and a model name", in one of two wire formats: Anthropic's Messages API, or OpenAI's chat completions, which most other providers have adopted.

The vendors' terms draw the same line. A subscription's sign-in is for the vendor's own clients; a third-party harness uses an API key.

Claude Code can be pointed at another address (`ANTHROPIC_BASE_URL`) and given a key in its environment. That alone is not enough. The address must speak the Messages API, which leaves out everything in OpenAI's format; and the key would sit in the environment of every command the agent runs, where one `env` puts it into the conversation and from there anywhere a prompt injection wants it.

## Decision

The agent stays the vendor's CLI. For an API connection, slash runs a gateway (`slate-gateway`) on 127.0.0.1 and gives the CLI its address and a token that is only good there, for one slash session. The gateway holds the key and reaches the person's endpoint:

- **Anthropic format:** the request goes on as it came, with the real key (and the connection's model in place of the CLI's, when it names one).
- **OpenAI format:** the request is rewritten for chat completions and the answer rewritten back: text as it streams, tool calls, images, the stop reason, token counts.

The connection lives in `~/.config/slate/provider.toml`, readable by its owner alone. So Slate now holds one credential: the API key a person gave it for this purpose. It still never holds or forwards a subscription's sign-in; with a subscription there is no gateway, and the CLI talks to its vendor by itself.

## Consequences

- Approvals, the audit log, snapshots and undo work the same with any provider, because the hooks and the permission tool are Claude Code's. This is why the gateway translates for Claude Code rather than Slate starting Codex for OpenAI-format endpoints: Codex runs under its own sandbox, without Slate's approvals.
- The key is out of the agent's reach by construction, not by policy: its processes see the session token. slated also asks before a `Read` of the provider file. The file itself is not encrypted; it is as safe as the CLIs' own credential files next to it.
- Slate owns a translation it must keep true to two moving APIs. What does not survive it is listed in `docs/agent-connection.md`: a provider's reasoning stream is dropped, Anthropic's server-side tools (web search, on-demand tool loading) do not exist elsewhere, so the CLI is told to send every tool up front and is not offered web search.
- Claude Code's prompts and tool descriptions are written for Claude. Other models follow them with varying skill; Slate does not adapt them.
- The CLI assumes Claude's context window. A model with a smaller one fails on a long session; `/new` starts a fresh one.
- Codex keeps its own sign-in and is not given API connections.

## Alternatives considered

- **Codex for OpenAI format.** No translation to own, but no approvals either, and a second agent with other habits for the same desktop.
- **The key in the CLI's environment** (Anthropic format only). Simple, and readable by everything the agent starts.
- **An existing router** (LiteLLM, claude-code-router). A Python or Node service and its dependencies on the path of every request, to do what a few hundred lines do here.
