# Connecting the agent

SlateOS works through an AI agent. The agent is a program, Claude Code or Codex, and it needs a model. The installer asks how it gets one, the welcome at first login asks again if the answer is still missing, and Settings, AI changes it later.

| | What you need | What runs |
|---|---|---|
| **Claude subscription** | A Claude account (Pro or Max) | Claude Code, signed in as you, talking to Anthropic by itself |
| **ChatGPT subscription** | A ChatGPT account (Plus or Pro) | Codex, signed in as you, talking to OpenAI by itself |
| **An API key of your own** | An address, a key and a model name, from any service in OpenAI's or Anthropic's format | Claude Code, talking to a gateway on this computer, which holds the key and reaches the service |

Approvals, the audit log and undo work with all three. What differs is how much the agent can do at first: Codex starts inside its own sandbox ([below](#a-chatgpt-subscription)).

## The agent's programs

SlateOS's image contains neither Claude Code nor Codex. Claude Code is Anthropic's, under a licence that does not let SlateOS hand it on, so every computer gets the programs itself: Claude Code from Anthropic's download server, Codex from the NixOS package cache. Together they are a few hundred megabytes, and getting them needs the internet.

| When | What happens |
|---|---|
| Installing, with a network | The installation downloads both to the disk. A sign-in in the installer needs its program earlier: **Download** fetches that one into the live session, and the installation copies it from there |
| Installing, without a network | SlateOS is installed without them. So it is when a download breaks off: the installation finishes without the programs rather than fail |
| At the first login | The welcome sees that the program is missing and offers **Download** (the internet, your password, a few minutes), then goes on to the connection |
| Later | Settings, AI, Your Agent shows which programs are installed and opens the same form |

From a terminal: `sudo slateos-settings set agents all` installs them, `sudo slateos-settings set agents none` removes them (the names are `claude-code` and `codex`). A system that uses the SlateOS module in a configuration of its own chooses with `services.slate.agents`, as before.

A new SlateOS release brings the versions of both it was tested with; a software update downloads them the same way.

## Going on without an agent

The installer's step and the welcome have **Skip**. It asks first, because it is a bad idea: without an agent the prompt (Super+S) and the terminal do not answer, and what is left is a bare desktop. It is there for a computer that has no network yet, or a person who has no account yet. The welcome comes back at every login until the agent works, and Settings, AI says in its first row what is missing.

## A Claude subscription

Choose **Claude subscription** and press **Sign In**. A terminal opens with Claude Code's own sign-in: it shows a link, the browser asks for your account, and the terminal says when it is done. SlateOS does not see the password. From the installer, the sign-in of the live session is carried over to the installed system.

## A ChatGPT subscription

Choose **ChatGPT subscription** and press **Sign In**: the same, with Codex's own sign-in. Choosing it makes Codex the agent (`backend = "codex"` in `~/.config/slate/slash.toml`; Settings, AI, Backend shows it), and choosing one of the other two makes it Claude Code again.

SlateOS asks before Codex's actions and undoes them as it does Claude Code's: Codex reports each command, file change and desktop action to SlateOS first (through the hooks in `/etc/codex/hooks.json`, which SlateOS installs), and waits for your answer where one is needed.

Codex also has a sandbox of its own, and SlateOS leaves it on until you say otherwise. Inside it Codex's commands change files only under the folder you are in, reach no network, and cannot become root. So at first the agent works on your files and drives applications on the desktop, and cannot install software, change system settings or download something from a command. Claude Code has no such sandbox: it can do those things, after asking.

**Settings, AI, Keep Codex in its sandbox** is the choice. Turned off, Codex can do what Claude Code can. SlateOS goes on asking before what needs your word, and undo stays; that is then the only guard, where the sandbox would have stopped a mistake at the folder you are in whatever was allowed, and a few of Codex's own tools (searching the web among them) do not pass SlateOS's questions. The switch says this before it turns off and then asks for your password, as every setting does that lowers a guard: the agent cannot change it for itself ([decisions 0011](decisions/0011-codex-through-system-hooks.md) and [0012](decisions/0012-guards-are-the-systems.md)).

Codex's sign-in can expire without Codex or this form noticing: when the agent answers that the sign-in must be renewed, sign in again in Settings, AI.

## An API key

Choose **An API key of your own** and fill in:

| Field | Meaning |
|---|---|
| Service | A known address (OpenAI, Anthropic, OpenRouter, DeepSeek), or "another service" in one of the two formats |
| Format | **OpenAI**: the service has `…/chat/completions`. **Anthropic**: it has `…/v1/messages` |
| API address | For OpenAI format, the URL that `/chat/completions` is appended to; it usually ends in `/v1`. For Anthropic format, the URL before `/v1/messages` |
| API key | The key the service gave you (`sk-…`) |
| Model | The model's name as the service spells it. The list button asks the service for its models. For Anthropic's own API it may stay empty: Claude Code then chooses (sonnet, opus, haiku) |

**Test** sends one short request through the same path the agent's requests take, and says who answered or what was refused. The step is complete when the test passes. Without a network yet, **Use It Anyway** keeps what you typed; try it again later in Settings, AI. The test needs no agent program, so in the installer an API key is complete before Claude Code is there; on an installed system the form asks for the program first.

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

How this is built, and why: [decision 0008](decisions/0008-api-connections.md) for API connections, [decision 0010](decisions/0010-agent-programs-are-downloaded.md) for the programs.
