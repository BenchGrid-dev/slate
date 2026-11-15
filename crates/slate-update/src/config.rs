//! Where things are, and what the running system was built to do about updates.

use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::{Path, PathBuf};

/// Every file and program the updater touches hangs off one root, so the tests run a
/// whole update inside a directory: `SLATEOS_UPDATE_ROOT`, or `/`.
#[derive(Clone, Debug)]
pub struct Paths {
    root: PathBuf,
}

impl Paths {
    pub fn system() -> Self {
        Self::under(
            std::env::var_os("SLATEOS_UPDATE_ROOT")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("/")),
        )
    }

    pub fn under(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// The computer itself, not a directory a test set up.
    pub fn is_real(&self) -> bool {
        self.root == Path::new("/")
    }

    pub fn at(&self, absolute: &str) -> PathBuf {
        self.root.join(absolute.trim_start_matches('/'))
    }

    /// A program: found through PATH on the computer, in `bin` under a test's root.
    pub fn tool(&self, name: &str) -> PathBuf {
        if self.is_real() {
            PathBuf::from(name)
        } else {
            self.root.join("bin").join(name)
        }
    }

    /// What the module wrote for the running system.
    pub fn config_file(&self) -> PathBuf {
        self.at("/etc/slate/update.json")
    }

    pub fn state_dir(&self) -> PathBuf {
        self.at("/var/lib/slateos-update")
    }

    pub fn state_file(&self) -> PathBuf {
        self.state_dir().join("state.json")
    }

    pub fn log_file(&self) -> PathBuf {
        self.state_dir().join("log")
    }

    pub fn lock_file(&self) -> PathBuf {
        self.state_dir().join("lock")
    }

    pub fn downloads(&self) -> PathBuf {
        self.state_dir().join("download")
    }

    /// Keeps a system that was built, and is not a generation yet, from being collected.
    pub fn built_root(&self) -> PathBuf {
        self.state_dir().join("built")
    }

    pub fn profile(&self) -> PathBuf {
        self.at("/nix/var/nix/profiles/system")
    }

    pub fn current_system(&self) -> PathBuf {
        self.at("/run/current-system")
    }

    pub fn efivars(&self) -> PathBuf {
        self.at("/sys/firmware/efi/efivars")
    }
}

/// `/etc/slate/update.json`: the running system's version and where it looks for the next.
#[derive(Clone, Debug, Deserialize)]
pub struct Config {
    /// The SlateOS version of the running system.
    pub version: String,
    /// The release manifest; its signature is at the same address with `.sig` added.
    pub url: String,
    /// Ed25519 public keys (base64) a manifest may be signed with.
    #[serde(default)]
    pub keys: Vec<String>,
    /// The directory of the system configuration, with the Slate source in `slate`.
    #[serde(default = "default_config_dir")]
    pub config: String,
    /// What must be running before a newly started version counts as working.
    #[serde(default)]
    pub health: Option<Health>,
    /// Start a new version once, on trial, before it becomes the default.
    #[serde(default = "yes")]
    pub trial: bool,
    /// Free space on /nix an update wants at least, in GiB.
    #[serde(default = "default_free")]
    pub free_gib: u64,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Health {
    /// The person the desktop logs in by itself.
    pub user: String,
    /// Programs of theirs that make up a working session.
    #[serde(default)]
    pub processes: Vec<String>,
}

fn default_config_dir() -> String {
    "/etc/slateos".into()
}

fn yes() -> bool {
    true
}

fn default_free() -> u64 {
    3
}

impl Config {
    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path).with_context(|| {
            format!(
                "{} is missing: this system was not set up for SlateOS updates \
                 (services.slate.update.enable)",
                path.display()
            )
        })?;
        serde_json::from_str(&text).with_context(|| format!("reading {}", path.display()))
    }

    pub fn config_dir(&self, paths: &Paths) -> PathBuf {
        paths.at(&self.config)
    }

    /// The Slate source the system is built from.
    pub fn source(&self, paths: &Paths) -> PathBuf {
        self.config_dir(paths).join("slate")
    }

    /// What the person chose in Settings; checking is the default.
    pub fn automatic(&self, paths: &Paths) -> Automatic {
        let file = self.config_dir(paths).join("settings.json");
        let value = std::fs::read_to_string(file)
            .ok()
            .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
            .and_then(|v| {
                v.get("automaticUpdates")
                    .and_then(|m| m.as_str().map(String::from))
            });
        match value.as_deref() {
            Some("off") => Automatic::Off,
            Some("download") => Automatic::Download,
            _ => Automatic::Check,
        }
    }
}

/// What happens without the person asking.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Automatic {
    /// Nothing.
    Off,
    /// Look for a new version once a day and say so.
    Check,
    /// Also download and build it, so that only the restart is left.
    Download,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_and_the_persons_choice() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::under(dir.path());
        let cfg: Config =
            serde_json::from_str(r#"{"version":"0.3.0","url":"https://example.org/u.json"}"#)
                .unwrap();
        assert_eq!(cfg.config, "/etc/slateos");
        assert!(cfg.trial);
        assert_eq!(cfg.automatic(&paths), Automatic::Check);

        std::fs::create_dir_all(cfg.config_dir(&paths)).unwrap();
        let settings = cfg.config_dir(&paths).join("settings.json");
        std::fs::write(&settings, r#"{"automaticUpdates":"download"}"#).unwrap();
        assert_eq!(cfg.automatic(&paths), Automatic::Download);
        std::fs::write(&settings, r#"{"automaticUpdates":"off"}"#).unwrap();
        assert_eq!(cfg.automatic(&paths), Automatic::Off);
        std::fs::write(&settings, "not json").unwrap();
        assert_eq!(cfg.automatic(&paths), Automatic::Check);
    }

    #[test]
    fn a_tool_is_found_through_path_only_on_the_computer() {
        assert_eq!(Paths::under("/").tool("nix-env"), PathBuf::from("nix-env"));
        assert_eq!(
            Paths::under("/tmp/x").tool("nix-env"),
            PathBuf::from("/tmp/x/bin/nix-env")
        );
    }
}
