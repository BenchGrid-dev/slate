# Connecting the agent

SlateOS works through an AI agent, and the agent needs a model. There are two ways to give it one. The installer asks which, the welcome at first login asks again if the answer is still missing, and Settings, AI changes it later.

| | What you need | What runs |
|---|---|---|
| **Claude subscription** | A Claude account (Pro or Max) | Claude Code, signed in as you, talking to Anthropic by itself |
| **An API key of your own** | An address, a key and a model name, from any service in OpenAI's or Anthropic's format | Claude Code, talking to a gateway on this computer, which holds the key and reaches the service |

Either way the agent is Claude Code, so approvals, the audit log and undo work the same. Codex, where it is installed, keeps its own sign-in (Settings, AI); it runs without Slate's approvals.

## A Claude subscription

Choose **Claude subscription** and press **Sign In**. A terminal opens with Claude Code's own sign-in: it shows a link, the browser asks for your account, and the terminal says when it is done. SlateOS does not see the password. From the installer, the sign-in of the live session is carried over to the installed system.

## An API key

Choose **An API key of your own** and fill in:

| Field | Meaning |
|---|---|
| Service | A known address (OpenAI, Anthropic, OpenRouter, DeepSeek), or "another service" in one of the two formats |
| Format | **OpenAI**: the service has `…/chat/completions`. **Anthropic**: it has `…/v1/messages` |
| API address | For OpenAI format, the URL that `/chat/completions` is appended to; it usually ends in `/v1`. For Anthropic format, the URL before `/v1/messages` |
| API key | The key the service gave you (`sk-…`) |
| Model | The model's name as the service spells it. The list button asks the service for its models. For Anthropic's own API it may stay empty: Claude Code then chooses (sonnet, opus, haiku) |

**Test** sends one short request through the same path the agent's requests take, and says who answered or what was refused. The step is complete when the test passes. Without a network yet, **Use It Anyway** keeps what you typed; try it again later in Settings, AI.

A service on your own network works too: `http://` addresses are accepted, and a server that wants no key takes any text as one.

## The file

The connection is `~/.config/slate/provider.toml`, readable by you alone:

```toml
connection = "api"            # or "subscription"

[api]
format = "openai"             # or "anthropic"
base_url = "https://api.openai.com/v1"
api_key = "sk-…"
model = "…"
small_model = "…"             # optional: a cheaper model for the agent's background requests
```

slash reads it when it starts; restart the Slate prompt (Settings, AI) or open a new terminal after editing it by hand. From a terminal:

```sh
slate provider show           # the connection in use; the key is not printed
slate provider test           # one short request
slate provider models         # what the service lists
```

In slash, the banner names the model and the service (`backend: claude (MODEL at HOST)`), and `/model NAME` asks the service for another of its models.

## Where the key goes

The key stays in slash. Claude Code is given the gateway's address on 127.0.0.1 and a token that only that gateway accepts, for that session; the commands the agent runs see the token, never the key. slated asks before the agent reads `provider.toml`. With a service of your own, Claude Code's telemetry to Anthropic is off.

## What differs from a subscription

- **Web search** is done by Anthropic's servers and is not offered with another service. Fetching a page you name still works.
- **Models other than Claude** get prompts and tool descriptions written for Claude. Strong models follow them; small ones lose their way in long tasks.
- **Context window:** Claude Code assumes Claude's. A model with a smaller window fails when a session grows long; `/new` starts a fresh one.
- **Reasoning** that a service streams (`reasoning_content`) is not shown.
- **Parameters a service refuses** (`max_tokens` limits, `temperature`, `stream_options`) are learnt from its error message and left out from then on, so a first request may take one try more.

How this is built, and why: [decision 0008](decisions/0008-api-connections.md).
