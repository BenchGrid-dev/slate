//! slash configuration: ~/.config/slate/slash.toml

use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::PathBuf;

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Config {
    /// "claude" or "codex".
    pub backend: String,
    /// Shell used for `!` lines and for non-interactive fallback. Defaults to $SHELL, then /bin/zsh.
    pub fallback_shell: Option<String>,
    /// How many manual shell commands to keep as context for the agent.
    pub context_commands: usize,
    pub claude: ClaudeConfig,
    pub codex: CodexConfig,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct ClaudeConfig {
    pub bin: String,
    /// Passed as --permission-mode. Until slated's approval broker exists this is the
    /// only permission control in headless mode. "default" will deny most tool calls.
    pub permission_mode: String,
    /// Passed as --allowedTools, comma-joined.
    pub allowed_tools: Vec<String>,
    /// Extra args appended verbatim.
    pub extra_args: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct CodexConfig {
    pub bin: String,
    /// Passed as --sandbox.
    pub sandbox: String,
    pub extra_args: Vec<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            backend: "claude".into(),
            fallback_shell: None,
            context_commands: 20,
            claude: ClaudeConfig::default(),
            codex: CodexConfig::default(),
        }
    }
}

impl Default for ClaudeConfig {
    fn default() -> Self {
        Self {
            bin: "claude".into(),
            permission_mode: "acceptEdits".into(),
            allowed_tools: vec![],
            extra_args: vec![],
        }
    }
}

impl Default for CodexConfig {
    fn default() -> Self {
        Self {
            bin: "codex".into(),
            sandbox: "workspace-write".into(),
            extra_args: vec![],
        }
    }
}

impl Config {
    pub fn path() -> Option<PathBuf> {
        dirs::config_dir().map(|d| d.join("slate").join("slash.toml"))
    }

    pub fn load() -> Result<Self> {
        let Some(path) = Self::path() else {
            return Ok(Self::default());
        };
        if !path.exists() {
            return Ok(Self::default());
        }
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))
    }

    /// The POSIX shell slash delegates to. Never resolves to slash itself.
    pub fn shell(&self) -> String {
        if let Some(s) = &self.fallback_shell {
            return s.clone();
        }
        if let Ok(s) = std::env::var("SLATE_FALLBACK_SHELL") {
            return s;
        }
        if let Ok(s) = std::env::var("SHELL") {
            let base = std::path::Path::new(&s)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("");
            if base != "slash" {
                return s;
            }
        }
        "/bin/zsh".into()
    }
}
