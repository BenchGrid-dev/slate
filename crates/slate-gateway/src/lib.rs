//! A gateway on this machine between the agent CLI and a model API the person brought
//! a key for.
//!
//! The agent CLI (Claude Code) speaks Anthropic's Messages API to a base URL it is
//! given. With an API connection, slash gives it this gateway's: a listener on
//! 127.0.0.1 that either passes the request through to an Anthropic-format endpoint,
//! or translates it to OpenAI's chat completions ([`openai`]). The key stays in this
//! process; the CLI gets a token that is only good for this gateway and this session,
//! so what the agent runs cannot read the key from its environment.
//!
//! Sign-in with a subscription never comes through here: the vendor's CLI then talks
//! to its vendor by itself (docs/decisions/0008-api-connections.md).

pub mod openai;
mod server;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};

/// The wire format the endpoint speaks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Format {
    /// Anthropic's Messages API (`POST {base}/v1/messages`).
    Anthropic,
    /// OpenAI's chat completions (`POST {base}/chat/completions`).
    Openai,
}

/// How the key is presented to an Anthropic-format endpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum Auth {
    /// `x-api-key` for api.anthropic.com; both headers elsewhere (compatible
    /// endpoints differ in which one they read).
    #[default]
    Auto,
    XApiKey,
    Bearer,
}

/// An API connection: where requests go and with which key.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct Api {
    pub format: Format,
    /// For OpenAI format the URL that `/chat/completions` is appended to, usually
    /// ending in `/v1`. For Anthropic format the URL before `/v1/messages`.
    pub base_url: String,
    pub api_key: String,
    /// The model to ask for, whatever the CLI names. Required for OpenAI format;
    /// empty for Anthropic format keeps the CLI's own choice (sonnet, opus, haiku).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// A cheaper model for the CLI's background requests (titles, summaries), which
    /// it sends to its "haiku" tier. Defaults to `model`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub small_model: Option<String>,
    #[serde(default, skip_serializing_if = "is_auto")]
    pub auth: Auth,
}

fn is_auto(a: &Auth) -> bool {
    *a == Auth::Auto
}

/// `~/.config/slate/provider.toml`: how the agent reaches a model.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, Default)]
pub struct Provider {
    /// "subscription": the agent CLI's own sign-in. "api": the connection in `[api]`.
    #[serde(default)]
    pub connection: Connection,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api: Option<Api>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Connection {
    #[default]
    Subscription,
    Api,
}

impl Provider {
    pub fn path() -> Option<PathBuf> {
        dirs::config_dir().map(|d| d.join("slate").join("provider.toml"))
    }

    /// The file's content, or the default (a subscription) when there is none.
    pub fn load() -> Result<Self> {
        match Self::path() {
            Some(p) if p.exists() => Self::load_from(&p),
            _ => Ok(Self::default()),
        }
    }

    pub fn load_from(path: &std::path::Path) -> Result<Self> {
        let text =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        let p: Self =
            toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
        if p.connection == Connection::Api {
            p.api
                .as_ref()
                .context("provider.toml: connection = \"api\" needs an [api] section")?
                .check()?;
        }
        Ok(p)
    }

    /// The API connection in use, if that is how the agent connects.
    pub fn active_api(&self) -> Option<&Api> {
        match self.connection {
            Connection::Api => self.api.as_ref(),
            Connection::Subscription => None,
        }
    }
}

impl Api {
    /// Whether the connection is complete enough to try.
    pub fn check(&self) -> Result<()> {
        let url = self.base_url.trim();
        if !(url.starts_with("https://") || url.starts_with("http://")) {
            bail!("the API address must start with https:// (or http:// for a server on this network)");
        }
        if self.api_key.trim().is_empty() {
            bail!("the API key is empty");
        }
        if self.format == Format::Openai && self.model.as_deref().unwrap_or("").trim().is_empty() {
            bail!("an OpenAI-format connection needs a model name");
        }
        Ok(())
    }

    /// Where a Messages API request path (`/v1/messages`) goes on an Anthropic-format endpoint.
    pub fn anthropic_url(&self, path_and_query: &str) -> String {
        let base = self.base_url.trim().trim_end_matches('/');
        let base = base.strip_suffix("/v1").unwrap_or(base);
        format!("{base}{path_and_query}")
    }

    /// The chat completions URL of an OpenAI-format endpoint.
    pub fn chat_url(&self) -> String {
        let base = self.base_url.trim().trim_end_matches('/');
        if base.ends_with("/chat/completions") {
            base.to_string()
        } else {
            format!("{base}/chat/completions")
        }
    }

    /// The model to ask the endpoint for when the CLI asks for `requested`.
    pub fn model_for(&self, requested: &str) -> String {
        let main = self
            .model
            .as_deref()
            .map(str::trim)
            .filter(|m| !m.is_empty());
        let small = self
            .small_model
            .as_deref()
            .map(str::trim)
            .filter(|m| !m.is_empty());
        let Some(main) = main else {
            return requested.to_string();
        };
        if requested == main || Some(requested) == small {
            return requested.to_string();
        }
        match small {
            Some(small) if requested.contains("haiku") => small.to_string(),
            _ => main.to_string(),
        }
    }

    /// The host of the endpoint, for showing where requests go.
    pub fn host(&self) -> String {
        let rest = self.base_url.trim().split("://").nth(1).unwrap_or("");
        rest.split('/').next().unwrap_or("").to_string()
    }
}

pub(crate) struct State {
    pub api: RwLock<Api>,
    pub token: String,
    pub agent: ureq::Agent,
    pub quirks: Mutex<openai::Quirks>,
    pub last_error: Mutex<Option<String>>,
}

/// The running gateway. It serves until the process ends.
pub struct Gateway {
    state: Arc<State>,
    port: u16,
}

impl Gateway {
    /// Listen on a free port of 127.0.0.1 and serve `api`.
    pub fn start(api: Api) -> Result<Self> {
        api.check()?;
        let listener =
            std::net::TcpListener::bind(("127.0.0.1", 0)).context("opening the gateway's port")?;
        let port = listener.local_addr()?.port();
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .timeout_connect(Some(std::time::Duration::from_secs(20)))
            .timeout_recv_response(Some(std::time::Duration::from_secs(600)))
            .build()
            .into();
        let state = Arc::new(State {
            api: RwLock::new(api),
            token: random_token(),
            agent,
            quirks: Mutex::new(openai::Quirks::default()),
            last_error: Mutex::new(None),
        });
        let serving = state.clone();
        std::thread::Builder::new()
            .name("slate-gateway".into())
            .spawn(move || server::serve(listener, serving))
            .context("starting the gateway")?;
        Ok(Self { state, port })
    }

    /// What the agent CLI is given as its API address.
    pub fn base_url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    /// What the agent CLI is given as its key: good for this gateway only.
    pub fn token(&self) -> &str {
        &self.state.token
    }

    pub fn api(&self) -> Api {
        self.state.api.read().unwrap().clone()
    }

    /// Ask for another model from now on (slash's `/model`).
    pub fn set_model(&self, model: Option<String>) {
        self.state.api.write().unwrap().model = model;
    }

    /// The endpoint's last refusal, for a message that says more than "the turn failed".
    pub fn last_error(&self) -> Option<String> {
        self.state.last_error.lock().unwrap().clone()
    }
}

fn random_token() -> String {
    use std::io::Read;
    let mut bytes = [0u8; 24];
    let ok = std::fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut bytes))
        .is_ok();
    if !ok {
        // No /dev/urandom: the clock and the address of a local are what there is.
        let t = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let seed = t ^ ((&bytes as *const _ as usize as u128) << 64) ^ (std::process::id() as u128);
        for (i, b) in bytes.iter_mut().enumerate() {
            *b = (seed >> ((i % 16) * 8)) as u8 ^ (i as u8).wrapping_mul(151);
        }
    }
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    format!("slate-session-{hex}")
}

/// The outcome of trying a connection once with a tiny request.
#[derive(Debug, Clone, Serialize)]
pub struct TestResult {
    pub ok: bool,
    /// One line for a person: what answered, or what went wrong.
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<u16>,
}

/// Send "Reply with OK" through a gateway for `api`, as the agent CLI would.
pub fn test(api: &Api) -> TestResult {
    let gateway = match Gateway::start(api.clone()) {
        Ok(g) => g,
        Err(e) => {
            return TestResult {
                ok: false,
                message: e.to_string(),
                status: None,
            }
        }
    };
    let model = api
        .model
        .clone()
        .filter(|m| !m.trim().is_empty())
        .unwrap_or_else(|| "claude-haiku-4-5".into());
    let body = serde_json::json!({
        "model": model, "max_tokens": 16,
        "messages": [{"role": "user", "content": "Reply with the single word OK."}],
    });
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_global(Some(std::time::Duration::from_secs(60)))
        .build()
        .into();
    let sent = agent
        .post(format!("{}/v1/messages", gateway.base_url()))
        .header("content-type", "application/json")
        .header("x-api-key", gateway.token())
        .header("anthropic-version", "2023-06-01")
        .send(body.to_string());
    let mut response = match sent {
        Ok(r) => r,
        Err(e) => {
            return TestResult {
                ok: false,
                message: format!("the gateway did not answer: {e}"),
                status: None,
            }
        }
    };
    let status = response.status().as_u16();
    let text = response.body_mut().read_to_string().unwrap_or_default();
    if status == 200 {
        let v: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();
        let said = v
            .get("content")
            .and_then(|c| c.get(0))
            .and_then(|b| b.get("text"))
            .and_then(|t| t.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        let model = v
            .get("model")
            .and_then(|m| m.as_str())
            .unwrap_or(&model)
            .to_string();
        let said = if said.is_empty() {
            "an empty reply".to_string()
        } else {
            format!("\"{}\"", said.chars().take(40).collect::<String>())
        };
        return TestResult {
            ok: true,
            message: format!("{} answered with {model}: {said}", api.host()),
            status: Some(status),
        };
    }
    let reason = openai::error_message(&text);
    let hint = match status {
        401 | 403 => " (check the key)",
        404 => " (check the address and the model name)",
        502 => " (the address could not be reached)",
        _ => "",
    };
    // The gateway's own messages already name the endpoint; an endpoint's raw answer does not.
    let message = if reason.contains(&api.host()) {
        format!("{reason}{hint}")
    } else {
        format!("{} refused: {reason}{hint}", api.host())
    };
    TestResult {
        ok: false,
        message,
        status: Some(status),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn api(format: Format, base: &str, model: Option<&str>, small: Option<&str>) -> Api {
        Api {
            format,
            base_url: base.into(),
            api_key: "sk-test".into(),
            model: model.map(Into::into),
            small_model: small.map(Into::into),
            auth: Auth::Auto,
        }
    }

    #[test]
    fn urls_are_built_from_what_people_paste() {
        assert_eq!(
            api(Format::Openai, "https://api.openai.com/v1", Some("m"), None).chat_url(),
            "https://api.openai.com/v1/chat/completions"
        );
        assert_eq!(
            api(Format::Openai, "https://api.deepseek.com/", Some("m"), None).chat_url(),
            "https://api.deepseek.com/chat/completions"
        );
        assert_eq!(
            api(
                Format::Openai,
                "http://localhost:11434/v1/chat/completions",
                Some("m"),
                None
            )
            .chat_url(),
            "http://localhost:11434/v1/chat/completions"
        );
        let a = api(
            Format::Anthropic,
            "https://api.anthropic.com/v1/",
            None,
            None,
        );
        assert_eq!(
            a.anthropic_url("/v1/messages?beta=true"),
            "https://api.anthropic.com/v1/messages?beta=true"
        );
        assert_eq!(a.host(), "api.anthropic.com");
    }

    #[test]
    fn the_cli_s_model_names_map_to_the_connection_s() {
        let plain = api(Format::Anthropic, "https://api.anthropic.com", None, None);
        assert_eq!(plain.model_for("claude-opus-4-5"), "claude-opus-4-5");
        let one = api(Format::Openai, "https://x/v1", Some("deepseek-chat"), None);
        assert_eq!(one.model_for("claude-sonnet-4-5"), "deepseek-chat");
        assert_eq!(one.model_for("claude-haiku-4-5"), "deepseek-chat");
        let two = api(
            Format::Openai,
            "https://x/v1",
            Some("gpt-big"),
            Some("gpt-small"),
        );
        assert_eq!(two.model_for("claude-haiku-4-5"), "gpt-small");
        assert_eq!(two.model_for("claude-sonnet-4-5"), "gpt-big");
        assert_eq!(two.model_for("gpt-small"), "gpt-small");
    }

    #[test]
    fn the_file_round_trips_and_is_checked() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("provider.toml");
        std::fs::write(&path, "connection = \"api\"\n\n[api]\nformat = \"openai\"\nbase_url = \"https://api.openai.com/v1\"\napi_key = \"sk-1\"\nmodel = \"gpt-test\"\n").unwrap();
        let p = Provider::load_from(&path).unwrap();
        assert_eq!(p.active_api().unwrap().format, Format::Openai);
        let again: Provider = toml::from_str(&toml::to_string(&p).unwrap()).unwrap();
        assert_eq!(again, p);
        std::fs::write(&path, "connection = \"api\"\n[api]\nformat = \"openai\"\nbase_url = \"https://x\"\napi_key = \"k\"\n").unwrap();
        assert!(Provider::load_from(&path)
            .unwrap_err()
            .to_string()
            .contains("model"));
        std::fs::write(&path, "connection = \"subscription\"\n").unwrap();
        assert!(Provider::load_from(&path).unwrap().active_api().is_none());
        assert!(random_token().len() > 40 && random_token() != random_token());
    }
}
