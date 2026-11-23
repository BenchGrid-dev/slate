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
    /// Run `!` lines with `-i` so rc files (aliases, functions) load. Slower per command.
    pub shell_interactive: bool,
    /// How many manual shell commands to keep as context for the agent.
    pub context_commands: usize,
    /// Start with approvals bypassed (`/auto on`). Confirm-tier actions run without asking.
    pub auto_approve: bool,
    /// Start in verbose mode (`/verbose`): raw backend events, tool details, session ids.
    pub verbose: bool,
    pub claude: ClaudeConfig,
    pub codex: CodexConfig,
    pub slated: SlatedConfig,
    /// How the agent reaches a model: its own sign-in, or an API connection with the
    /// person's key. Not part of slash.toml: it lives in provider.toml, which holds
    /// the key and is readable by the person alone.
    #[serde(skip)]
    pub provider: slate_gateway::Provider,
    /// Why provider.toml could not be used, to say once at start.
    #[serde(skip)]
    pub provider_problem: Option<String>,
    /// What slash.toml asked for and the system did not grant, to say once at start.
    #[serde(skip)]
    pub not_granted: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct SlatedConfig {
    /// Use slated for approvals, audit and undo when available.
    pub enable: bool,
    /// Start slated if it is not running.
    pub auto_start: bool,
}

impl Default for SlatedConfig {
    fn default() -> Self {
        Self {
            enable: true,
            auto_start: true,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct ClaudeConfig {
    pub bin: String,
    /// Passed as --model. An alias like "sonnet" or "opus", or a full model id.
    pub model: Option<String>,
    /// Passed as --permission-mode when slated is NOT in use. With slated, the mode is
    /// "default" and slated's hook decides. Without it, "default" denies most tool calls.
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
    /// Passed as -m. None keeps Codex's own default.
    pub model: Option<String>,
    /// Passed as --sandbox.
    pub sandbox: String,
    pub extra_args: Vec<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            backend: "claude".into(),
            fallback_shell: None,
            shell_interactive: false,
            context_commands: 20,
            auto_approve: false,
            verbose: false,
            claude: ClaudeConfig::default(),
            codex: CodexConfig::default(),
            slated: SlatedConfig::default(),
            provider: slate_gateway::Provider::default(),
            provider_problem: None,
            not_granted: vec![],
        }
    }
}

impl Default for ClaudeConfig {
    fn default() -> Self {
        Self {
            bin: "claude".into(),
            model: Some("sonnet".into()),
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
            model: None,
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
        let mut cfg = match Self::path() {
            Some(path) if path.exists() => {
                let text = std::fs::read_to_string(&path)
                    .with_context(|| format!("reading {}", path.display()))?;
                toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?
            }
            _ => Self::default(),
        };
        // A broken provider.toml must not lock the person out of their shell: say
        // so and go on with the agent's own sign-in.
        match slate_gateway::Provider::load() {
            Ok(p) => cfg.provider = p,
            Err(e) => cfg.provider_problem = Some(format!("{e:#}")),
        }
        cfg.apply_guards(crate::system::guards());
        Ok(cfg)
    }

    /// On SlateOS the choices that lower a guard are the system's (see `system`):
    /// what slash.toml says about them does not count there.
    pub fn apply_guards(&mut self, guards: Option<crate::system::Guards>) {
        use crate::backend::codex::NO_SANDBOX;
        let Some(guards) = guards else {
            return; // no system to ask: slash.toml decides
        };
        if self.auto_approve && !guards.bypass_approvals {
            self.not_granted.push(
                "auto_approve in slash.toml is not used on SlateOS: Settings, AI turns it on, with your password".into(),
            );
        }
        self.auto_approve = guards.bypass_approvals;
        if guards.codex_unsandboxed {
            self.codex.sandbox = NO_SANDBOX.into();
        } else if self.codex.sandbox == NO_SANDBOX {
            self.not_granted.push(
                "sandbox = \"danger-full-access\" in slash.toml is not used on SlateOS: Settings, AI lets Codex out of its sandbox, with your password".into(),
            );
            self.codex.sandbox = CodexConfig::default().sandbox;
        }
    }

    /// The POSIX shell slash delegates to. Never resolves to slash itself, and
    /// never to a path that does not exist (NixOS has no /bin/zsh).
    pub fn shell(&self) -> String {
        let mut candidates: Vec<String> = vec![];
        if let Some(s) = &self.fallback_shell {
            candidates.push(s.clone());
        }
        if let Ok(s) = std::env::var("SLATE_FALLBACK_SHELL") {
            candidates.push(s);
        }
        if let Ok(s) = std::env::var("SHELL") {
            let base = std::path::Path::new(&s)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("");
            if base != "slash" {
                candidates.push(s);
            }
        }
        for s in [
            "/bin/zsh",
            "/usr/bin/zsh",
            "/bin/bash",
            "/usr/bin/bash",
            "/bin/sh",
        ] {
            candidates.push(s.into());
        }
        // Bare names (e.g. "zsh") are resolved through PATH by the OS; trust them.
        candidates
            .into_iter()
            .find(|c| !c.contains('/') || std::path::Path::new(c).exists())
            .unwrap_or_else(|| "/bin/sh".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::codex::NO_SANDBOX;
    use crate::system::Guards;

    fn asked_for_everything() -> Config {
        let mut cfg = Config {
            auto_approve: true,
            ..Config::default()
        };
        cfg.codex.sandbox = NO_SANDBOX.into();
        cfg
    }

    #[test]
    fn on_slateos_the_guards_are_the_systems_not_slash_tomls() {
        // slash.toml asks; the system has not granted: both stay up, and slash says so.
        let mut cfg = asked_for_everything();
        cfg.apply_guards(Some(Guards::default()));
        assert!(!cfg.auto_approve);
        assert_eq!(cfg.codex.sandbox, "workspace-write");
        assert_eq!(cfg.not_granted.len(), 2);

        // The system grants; slash.toml need not ask.
        let mut cfg = Config::default();
        cfg.apply_guards(Some(Guards {
            bypass_approvals: true,
            codex_unsandboxed: true,
        }));
        assert!(cfg.auto_approve);
        assert_eq!(cfg.codex.sandbox, NO_SANDBOX);
        assert!(cfg.not_granted.is_empty());

        // A stricter sandbox of the person's own stays.
        let mut cfg = Config::default();
        cfg.codex.sandbox = "read-only".into();
        cfg.apply_guards(Some(Guards::default()));
        assert_eq!(cfg.codex.sandbox, "read-only");
    }

    #[test]
    fn without_a_system_to_ask_slash_toml_decides() {
        let mut cfg = asked_for_everything();
        cfg.apply_guards(None);
        assert!(cfg.auto_approve);
        assert_eq!(cfg.codex.sandbox, NO_SANDBOX);
        assert!(cfg.not_granted.is_empty());
    }
}
