//! What the updater knows between runs: `/var/lib/slateos-update/state.json`. Root
//! writes it; Settings and the notification read it.

use crate::config::Paths;
use crate::manifest::Offer;
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::io::AsRawFd;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Phase {
    /// Nothing under way.
    #[default]
    Idle,
    /// Getting the release's source.
    Downloading,
    /// Downloading and building the new system.
    Building,
    /// The new system is in place; the next start runs it.
    Restart,
}

/// The update under way, or waiting for its restart.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Target {
    pub version: String,
    /// The version it replaces.
    pub previous_version: String,
    /// The new source is in place of the old one (which is kept as `slate.previous`).
    #[serde(default)]
    pub swapped: bool,
    /// The built system's store path.
    #[serde(default)]
    pub system: Option<String>,
    /// Its generation in the system profile, and the one before it.
    #[serde(default)]
    pub generation: Option<u64>,
    #[serde(default)]
    pub previous_generation: Option<u64>,
    /// The next start tries it once; the old version stays the default until it ran.
    #[serde(default)]
    pub trial: bool,
    /// This is the way back to the version before, not an update.
    #[serde(default)]
    pub back: bool,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Outcome {
    /// Runs, and is the default.
    Installed,
    /// Could not be downloaded or built; nothing changed.
    Failed,
    /// Was started and did not come up; the version before runs again.
    RolledBack,
    /// Taken back before the restart.
    Cancelled,
    /// The person went back to the version before.
    WentBack,
}

/// How the last update ended.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Last {
    pub version: String,
    pub outcome: Outcome,
    /// Seconds since 1970.
    pub when: u64,
    #[serde(default)]
    pub message: String,
}

/// The version an update replaced, while it can still be gone back to.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Previous {
    pub version: String,
    pub generation: u64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Progress {
    pub text: String,
    #[serde(default)]
    pub done: u64,
    #[serde(default)]
    pub total: u64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct State {
    #[serde(default)]
    pub format: u32,
    /// When the release server last answered, in seconds since 1970.
    #[serde(default)]
    pub checked: Option<u64>,
    /// Why the last check got no answer.
    #[serde(default)]
    pub check_error: Option<String>,
    /// A newer release, as of the last check.
    #[serde(default)]
    pub available: Option<Offer>,
    #[serde(default)]
    pub phase: Phase,
    #[serde(default)]
    pub progress: Option<Progress>,
    #[serde(default)]
    pub target: Option<Target>,
    #[serde(default)]
    pub last: Option<Last>,
    #[serde(default)]
    pub previous: Option<Previous>,
    /// A version the person went back from: not installed again unasked.
    #[serde(default)]
    pub skip: Option<String>,
}

pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

impl State {
    /// The state on disk; a missing or unreadable file is an empty state.
    pub fn load(paths: &Paths) -> Self {
        std::fs::read_to_string(paths.state_file())
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    /// Written whole and renamed into place, so a reader never sees half a file; and
    /// on the disk before this returns, because the next start decides by it.
    pub fn save(&mut self, paths: &Paths) -> Result<()> {
        self.write(paths, true)
    }

    /// The same without waiting for the disk: for progress, which nothing decides by.
    pub fn note(&mut self, paths: &Paths) -> Result<()> {
        self.write(paths, false)
    }

    fn write(&mut self, paths: &Paths, durable: bool) -> Result<()> {
        self.format = 1;
        let dir = paths.state_dir();
        std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
        let tmp = dir.join("state.json.tmp");
        let mut file = File::create(&tmp).with_context(|| format!("writing {}", tmp.display()))?;
        file.write_all(serde_json::to_string_pretty(self)?.as_bytes())?;
        file.write_all(b"\n")?;
        if durable {
            file.sync_all()?;
        }
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o644))?;
        std::fs::rename(&tmp, paths.state_file())?;
        Ok(())
    }

    pub fn finish(&mut self, version: &str, outcome: Outcome, message: impl Into<String>) {
        self.phase = Phase::Idle;
        self.progress = None;
        self.target = None;
        self.last = Some(Last {
            version: version.to_string(),
            outcome,
            when: now(),
            message: message.into(),
        });
    }
}

/// One updater at a time: an exclusive lock on a file, gone with the process.
pub struct Lock {
    _file: File,
}

impl Lock {
    pub fn take(paths: &Paths) -> Result<Self> {
        let dir = paths.state_dir();
        std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
        let file = File::create(paths.lock_file())?;
        // SAFETY: flock on a descriptor this function owns.
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            bail!("another update is running");
        }
        Ok(Self { _file: file })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_state_survives_a_round_trip_and_a_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::under(dir.path());
        assert_eq!(State::load(&paths), State::default());

        let mut state = State {
            phase: Phase::Restart,
            target: Some(Target {
                version: "0.3.1".into(),
                previous_version: "0.3.0".into(),
                generation: Some(8),
                trial: true,
                ..Default::default()
            }),
            ..Default::default()
        };
        state.save(&paths).unwrap();
        assert_eq!(State::load(&paths), state);
        let text = std::fs::read_to_string(paths.state_file()).unwrap();
        assert!(text.contains("\"phase\": \"restart\""), "{text}");

        state.finish("0.3.1", Outcome::RolledBack, "did not start");
        state.save(&paths).unwrap();
        let loaded = State::load(&paths);
        assert_eq!(loaded.phase, Phase::Idle);
        assert_eq!(loaded.last.unwrap().outcome, Outcome::RolledBack);
        assert!(std::fs::read_to_string(paths.state_file())
            .unwrap()
            .contains("rolled-back"));

        // A file from a later version with fields this one does not know still loads.
        std::fs::write(paths.state_file(), r#"{"format":2,"phase":"idle","new":1}"#).unwrap();
        assert_eq!(State::load(&paths).phase, Phase::Idle);
        std::fs::write(paths.state_file(), "garbage").unwrap();
        assert_eq!(State::load(&paths), State::default());
    }

    #[test]
    fn only_one_holds_the_lock() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::under(dir.path());
        let first = Lock::take(&paths).unwrap();
        assert!(Lock::take(&paths).is_err());
        drop(first);
        Lock::take(&paths).unwrap();
    }
}
