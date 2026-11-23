//! What the system decides for slash.
//!
//! On SlateOS the choices that lower a guard are root's: whether sessions start with
//! approvals bypassed, whether Codex runs outside its sandbox. Settings changes them
//! with the person's password (`slateos-settings set`), and they are read from the
//! system's settings.json. slash.toml cannot make them: it lies in the person's home,
//! where anything running as the person, the agent included, could write it.
//!
//! Where the system holds no such choices (slash without SlateOS's desktop), slash.toml
//! decides, as it always did.

use serde_json::Value;
use std::path::Path;

/// The file SlateOS's module writes; its presence says the system holds the choices.
const SYSTEM: &str = "/etc/slate/system-settings.json";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Guards {
    /// Sessions start as `/auto on`.
    pub bypass_approvals: bool,
    /// Codex runs outside its own sandbox.
    pub codex_unsandboxed: bool,
}

/// The system's choices; None where the system has none to make.
pub fn guards() -> Option<Guards> {
    let settings = if Path::new("/etc/slateos").is_dir() {
        "/etc/slateos/settings.json"
    } else {
        "/etc/nixos/settings.json"
    };
    guards_at(Path::new(SYSTEM), Path::new(settings))
}

/// A settings.json that is missing, or says nothing, leaves every guard up.
pub fn guards_at(system: &Path, settings: &Path) -> Option<Guards> {
    if !system.exists() {
        return None;
    }
    let chosen: Value = std::fs::read_to_string(settings)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or(Value::Null);
    Some(Guards {
        bypass_approvals: chosen.get("bypassApprovals").and_then(Value::as_bool) == Some(true),
        codex_unsandboxed: chosen.get("codexSandbox").and_then(Value::as_str) == Some("lift"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_systems_file_decides_and_says_nothing_by_default() {
        let dir = tempfile::tempdir().unwrap();
        let system = dir.path().join("system-settings.json");
        let settings = dir.path().join("settings.json");
        // Not SlateOS: no choices here.
        assert_eq!(guards_at(&system, &settings), None);
        std::fs::write(&system, "{}").unwrap();
        // SlateOS, nothing chosen (or a file that cannot be read): every guard up.
        assert_eq!(guards_at(&system, &settings), Some(Guards::default()));
        std::fs::write(&settings, "not json").unwrap();
        assert_eq!(guards_at(&system, &settings), Some(Guards::default()));
        std::fs::write(
            &settings,
            r#"{"agents": ["codex"], "bypassApprovals": true, "codexSandbox": "lift"}"#,
        )
        .unwrap();
        assert_eq!(
            guards_at(&system, &settings),
            Some(Guards {
                bypass_approvals: true,
                codex_unsandboxed: true
            })
        );
        // Only the values slateos-settings writes count.
        std::fs::write(
            &settings,
            r#"{"bypassApprovals": "yes", "codexSandbox": "danger-full-access"}"#,
        )
        .unwrap();
        assert_eq!(guards_at(&system, &settings), Some(Guards::default()));
    }
}
