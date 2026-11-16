//! An update from beginning to end.
//!
//! `install` gets the release's source, puts it in place of the running one (which is
//! kept), builds the system from it and enters the result in the boot menu, to be
//! started once: the version that runs stays the default. `boot`, at the next start,
//! finds out which of the two came up. The new one, working: it becomes the default.
//! The old one: the new one did not start, and everything is put back.

use crate::config::{Config, Paths};
use crate::manifest::{Fetcher, Offer, Version};
use crate::state::{now, Lock, Outcome, Phase, Previous, Progress, State, Target};
use crate::system::{self, Sys};
use anyhow::{anyhow, bail, Context, Result};
use std::path::{Path, PathBuf};
use std::time::Duration;

pub struct Updater {
    pub paths: Paths,
    pub config: Config,
    fetcher: Fetcher,
    /// How long two looks at the session are apart before it counts as staying up.
    pub settle: Duration,
}

/// Who asked for the installation.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Asked {
    /// The person: now, whatever the last attempt did.
    Person,
    /// The timer: not on battery, and not a version that failed before.
    Timer,
}

fn mib(bytes: u64) -> String {
    if bytes >= 1 << 30 {
        format!("{:.1} GB", bytes as f64 / (1u64 << 30) as f64)
    } else {
        format!("{:.0} MB", bytes as f64 / (1u64 << 20) as f64)
    }
}

impl Updater {
    pub fn new(paths: Paths) -> Result<Self> {
        let config = Config::load(&paths.config_file())?;
        Ok(Self::with(paths, config))
    }

    pub fn with(paths: Paths, config: Config) -> Self {
        Self {
            paths,
            config,
            fetcher: Fetcher::new(),
            settle: Duration::from_secs(20),
        }
    }

    fn sys(&self) -> Sys<'_> {
        Sys::new(&self.paths)
    }

    pub fn current(&self) -> Result<Version> {
        self.config
            .version
            .parse()
            .context("this system's own version")
    }

    fn source(&self) -> PathBuf {
        self.config.source(&self.paths)
    }

    fn next_source(&self) -> PathBuf {
        self.source().with_extension("next")
    }

    fn previous_source(&self) -> PathBuf {
        self.source().with_extension("previous")
    }

    fn configuration(&self) -> PathBuf {
        self.config
            .config_dir(&self.paths)
            .join("configuration.nix")
    }

    /// Only a system laid out the way the installer does it is updated this way: its
    /// configuration next to the Slate source it is built from.
    fn layout(&self) -> Result<()> {
        if !self.source().join("flake.nix").is_file() || !self.configuration().is_file() {
            bail!(
                "this system is not built from {}: it follows its own configuration, \
                 and is updated the way that configuration is",
                self.source().display()
            );
        }
        Ok(())
    }

    /// Asks the release server. `record` (root only) keeps the answer for Settings.
    pub fn check(&self, record: bool) -> Result<Option<Offer>> {
        let result = self
            .fetcher
            .find(&self.config.url, &self.config.keys, &self.current()?);
        if record {
            let mut state = State::load(&self.paths);
            match &result {
                Ok(offer) => {
                    state.checked = Some(now());
                    state.check_error = None;
                    state.available = offer.clone();
                }
                Err(e) => state.check_error = Some(format!("{e:#}")),
            }
            state.save(&self.paths)?;
        }
        result
    }

    fn progress(&self, state: &mut State, text: impl Into<String>, done: u64, total: u64) {
        let text = text.into();
        self.sys().log(&format!("== {text}"));
        state.note(&self.paths, Progress { text, done, total });
    }

    /// Downloads, builds and enters the newest release in the boot menu. Ok(None):
    /// nothing to do. `say` hears each step.
    pub fn install(&self, asked: Asked, say: &mut dyn FnMut(&str)) -> Result<Option<Target>> {
        let _lock = Lock::take(&self.paths)?;
        let mut state = State::load(&self.paths);
        self.recover(&mut state)?;
        if state.phase == Phase::Restart {
            let version = state.target.as_ref().map(|t| t.version.clone());
            bail!(
                "SlateOS {} is installed and starts at the next restart",
                version.unwrap_or_default()
            );
        }
        self.layout()?;

        let offer = match self.check(true) {
            Ok(Some(offer)) => offer,
            Ok(None) => return Ok(None),
            Err(e) => return Err(e),
        };
        state = State::load(&self.paths);
        let version = offer.manifest.version.clone();
        if asked == Asked::Timer {
            let failed_before = state
                .last
                .as_ref()
                .is_some_and(|l| l.version == version && l.outcome != Outcome::Installed);
            if failed_before {
                say(&format!(
                    "SlateOS {version} did not install last time; it waits for you to ask"
                ));
                return Ok(None);
            }
            if state.skip.as_deref() == Some(version.as_str()) {
                say(&format!(
                    "you went back from SlateOS {version}; it waits for you to ask"
                ));
                return Ok(None);
            }
            if self.sys().on_battery() {
                say("on battery: the download waits for the charger");
                return Ok(None);
            }
        }
        if let Some(free) = self.sys().free_bytes() {
            if free < self.config.free_gib << 30 {
                bail!(
                    "the update needs {} GB of free space, and there are {}",
                    self.config.free_gib,
                    mib(free)
                );
            }
        }

        self.sys().start_log();
        state.skip = None;
        state.phase = Phase::Downloading;
        state.target = Some(Target {
            version: version.clone(),
            previous_version: self.config.version.clone(),
            boot: self.sys().boot_id(),
            ..Default::default()
        });
        state.save(&self.paths)?;

        match self.steps(&offer, &mut state, say) {
            Ok(target) => {
                state.phase = Phase::Restart;
                state.progress = None;
                state.target = Some(target.clone());
                state.save(&self.paths)?;
                Ok(Some(target))
            }
            Err(e) => {
                let message = format!("{e:#}");
                self.sys().log(&format!("== failed: {message}"));
                self.undo(&mut state);
                state.finish(&version, Outcome::Failed, message);
                state.save(&self.paths)?;
                Err(e)
            }
        }
    }

    fn steps(&self, offer: &Offer, state: &mut State, say: &mut dyn FnMut(&str)) -> Result<Target> {
        let sys = self.sys();
        let version = &offer.manifest.version;

        say(&format!("Downloading SlateOS {version}"));
        self.progress(state, format!("Downloading SlateOS {version}"), 0, 0);
        std::fs::create_dir_all(self.paths.downloads())?;
        let archive = self
            .paths
            .downloads()
            .join(format!("slateos-{version}-source.tar.gz"));
        let (hash, size) = self
            .fetcher
            .to_file(&offer.source_url(), &archive, &mut |_| {})?;
        if !hash.eq_ignore_ascii_case(&offer.manifest.source.sha256)
            || (offer.manifest.source.size != 0 && offer.manifest.source.size != size)
        {
            let _ = std::fs::remove_file(&archive);
            bail!("the downloaded source is not the one the release names (its checksum differs)");
        }
        let next = self.next_source();
        system::unpack(&archive, &next)?;
        let _ = std::fs::remove_file(&archive);
        let unpacked = system::source_version(&next)?;
        if unpacked != *version || !next.join("flake.nix").is_file() {
            let _ = std::fs::remove_dir_all(&next);
            bail!("the downloaded source is SlateOS {unpacked}, not {version}");
        }

        // The new source takes the place of the old one, so that the system is built
        // from exactly what later rebuilds will see. The old one is kept.
        let _ = std::fs::remove_dir_all(self.previous_source());
        std::fs::rename(self.source(), self.previous_source())?;
        if let Err(e) = std::fs::rename(&next, self.source()) {
            std::fs::rename(self.previous_source(), self.source())?;
            return Err(e.into());
        }
        state.phase = Phase::Building;
        if let Some(target) = state.target.as_mut() {
            target.swapped = true;
        }
        state.save(&self.paths)?;

        say("Preparing");
        self.progress(state, "Preparing", 0, 0);
        let nixpkgs = sys.pinned_nixpkgs(&self.source())?;
        let drv = sys.instantiate(&nixpkgs, &self.configuration())?;
        let cache = offer.manifest.cache.as_ref();
        let plan = sys.plan(&drv, cache)?;
        if let Some(free) = sys.free_bytes() {
            if free < plan.unpacked_bytes + (1 << 30) {
                bail!(
                    "the update takes {} on disk, and there are {} free",
                    mib(plan.unpacked_bytes),
                    mib(free)
                );
            }
        }
        let total = plan.fetch + plan.build;
        let what = if plan.fetch > 0 {
            format!("Downloading {} and building", mib(plan.download_bytes))
        } else {
            "Building".to_string()
        };
        say(&format!(
            "{what}: {} to download, {} to build",
            plan.fetch, plan.build
        ));
        self.progress(state, what.clone(), 0, total);

        let mut done = 0u64;
        let mut shown = std::time::Instant::now();
        let system = sys.realise(&drv, cache, &mut |line| {
            if line.starts_with("copying path '") || line.starts_with("building '") {
                done += 1;
                // The state file is what Settings draws its bar from; twice a second is plenty.
                if shown.elapsed() > Duration::from_millis(500) {
                    shown = std::time::Instant::now();
                    state.note(
                        &self.paths,
                        Progress {
                            text: what.clone(),
                            done: done.min(total),
                            total,
                        },
                    );
                }
            }
        })?;

        say("Setting up the next start");
        self.progress(state, "Setting up the next start", total, total);
        let mut target = state.target.clone().unwrap_or_default();
        target.system = Some(system.clone());
        state.target = Some(target);
        state.save(&self.paths)?;
        let newer = Path::new(&system).join("sw/bin/slateos-update");
        if newer.is_file() {
            sys.hand_over(&newer, &system)?;
            *state = State::load(&self.paths);
        } else {
            self.activate(state, Path::new(&system))?;
        }
        state
            .target
            .clone()
            .ok_or_else(|| anyhow!("the update lost track of itself"))
    }

    /// The second half of an installation, for the updater of the release being
    /// installed (`slateos-update activate`): the first half holds the lock and waits.
    pub fn activate_recorded(&self, system: &Path) -> Result<()> {
        let mut state = State::load(&self.paths);
        if state.phase != Phase::Building || state.target.is_none() {
            bail!("no installation is under way");
        }
        self.activate(&mut state, system)
    }

    /// Enters a built system in the boot menu. With a trial, the running version stays
    /// the default and the new one is started once; without (no systemd-boot, firmware
    /// variables that cannot be written), the new one is the default at once.
    fn activate(&self, state: &mut State, system: &Path) -> Result<()> {
        let sys = self.sys();
        let previous = sys.current_generation()?;
        let previous_system = sys
            .generation_system(previous)
            .ok_or_else(|| anyhow!("generation {previous} of the system is gone"))?;
        let generation = sys.add_generation(&system.to_string_lossy())?;
        if let Some(target) = state.target.as_mut() {
            target.generation = Some(generation);
            target.previous_generation = Some(previous);
        }
        state.save(&self.paths)?;

        let mut trial = false;
        if self.config.trial && generation != previous && sys.can_try_once() {
            sys.switch_generation(previous)?;
            sys.write_boot_menu(&previous_system)?;
            if sys.entry_exists(generation) {
                match sys.set_oneshot(&Sys::entry(generation)) {
                    Ok(()) => trial = true,
                    Err(e) => sys.log(&format!("no trial start: {e:#}")),
                }
            }
            if !trial {
                sys.switch_generation(generation)?;
            }
        }
        if !trial {
            sys.write_boot_menu(system)?;
        }
        if let Some(target) = state.target.as_mut() {
            target.trial = trial;
        }
        state.save(&self.paths)?;
        Ok(())
    }

    /// Puts back what an update changed: the generation it added, the boot menu, the
    /// source. Each step is tried whatever became of the one before.
    fn undo(&self, state: &mut State) {
        let sys = self.sys();
        let Some(target) = state.target.clone() else {
            return;
        };
        if let Some(generation) = target.generation {
            let previous = target.previous_generation;
            if let Some(previous) = previous {
                if sys.current_generation().ok() == Some(generation) && previous != generation {
                    let _ = sys.switch_generation(previous);
                }
            }
            if sys.current_generation().ok() != Some(generation) {
                if target.trial {
                    let _ = sys.set_oneshot("");
                }
                if sys.generation_system(generation).is_some() {
                    let _ = sys.delete_generation(generation);
                }
                if let Some(system) = previous.and_then(|p| sys.generation_system(p)) {
                    if let Err(e) = sys.write_boot_menu(&system) {
                        sys.log(&format!("the boot menu was not rewritten: {e:#}"));
                    }
                }
            }
        }
        let _ = std::fs::remove_file(self.paths.built_root());
        if target.swapped {
            self.restore_source(&target);
        }
        let _ = std::fs::remove_dir_all(self.next_source());
    }

    /// The source that was replaced takes its place again.
    fn restore_source(&self, target: &Target) {
        let previous = self.previous_source();
        if !previous.join("flake.nix").is_file() {
            return;
        }
        // Only when what is in place is the update's: a person may have put something
        // else there since.
        let in_place = system::source_version(&self.source()).ok();
        if in_place.is_some() && in_place.as_deref() != Some(target.version.as_str()) {
            return;
        }
        let failed = self.source().with_extension("failed");
        let _ = std::fs::remove_dir_all(&failed);
        if self.source().exists() && std::fs::rename(self.source(), &failed).is_err() {
            return;
        }
        if std::fs::rename(&previous, self.source()).is_err() {
            let _ = std::fs::rename(&failed, self.source());
            return;
        }
        let _ = std::fs::remove_dir_all(&failed);
    }

    /// An installation that was cut short (the power went, the process was killed):
    /// what it had changed is put back. Called with the lock held, so none runs.
    fn recover(&self, state: &mut State) -> Result<()> {
        if matches!(state.phase, Phase::Downloading | Phase::Building) {
            let version = state
                .target
                .as_ref()
                .map(|t| t.version.clone())
                .unwrap_or_default();
            self.undo(state);
            state.finish(
                &version,
                Outcome::Failed,
                "the installation was interrupted; nothing was changed",
            );
            state.save(&self.paths)?;
        }
        Ok(())
    }

    /// Takes back an update that waits for its restart.
    pub fn cancel(&self) -> Result<String> {
        let _lock = Lock::take(&self.paths)?;
        let mut state = State::load(&self.paths);
        self.recover(&mut state)?;
        let Some(target) = state
            .target
            .clone()
            .filter(|_| state.phase == Phase::Restart)
        else {
            bail!("no update waits for a restart");
        };
        if target.back {
            // Going back changed the default and the source; undo that the same way.
            self.undo_back(&target)?;
        } else {
            self.undo(&mut state);
        }
        state.finish(&target.version, Outcome::Cancelled, "");
        state.save(&self.paths)?;
        Ok(target.version)
    }

    /// At every start: finish or undo what the last update left. Blocks until the
    /// session is up when a new version is on trial.
    pub fn boot(&self) -> Result<Option<Outcome>> {
        let mut state = State::load(&self.paths);
        match state.phase {
            Phase::Idle => return Ok(None),
            Phase::Downloading | Phase::Building => {
                let _lock = Lock::take(&self.paths)?;
                self.recover(&mut state)?;
                return Ok(Some(Outcome::Failed));
            }
            Phase::Restart => {}
        }
        let target = state.target.clone().ok_or_else(|| {
            anyhow!("an update waits for a restart, and the state does not say which")
        })?;
        let sys = self.sys();
        // Asked again during the start the update was installed in (a rebuild starts
        // the system's units anew): the restart is still to come, and the version that
        // runs is the old one because nothing else was tried yet.
        if target.boot.is_some() && target.boot == sys.boot_id() {
            return Ok(None);
        }
        let running = std::fs::canonicalize(self.paths.current_system()).ok();
        let wanted = target
            .system
            .as_deref()
            .and_then(|s| std::fs::canonicalize(s).ok());
        let came_up = running.is_some() && running == wanted;
        // A rebuild since the installation gives the same release another store path.
        let same_release = self.config.version == target.version;

        if !(came_up || same_release) {
            // The new version was to start and the old one runs: it did not come up.
            let _lock = Lock::take(&self.paths)?;
            if target.back {
                state.finish(
                    &target.version,
                    Outcome::Failed,
                    "the version before did not start",
                );
            } else {
                self.undo(&mut state);
                state.finish(
                    &target.version,
                    Outcome::RolledBack,
                    format!(
                        "SlateOS {} did not start; this is {} again",
                        target.version, self.config.version
                    ),
                );
            }
            state.save(&self.paths)?;
            return Ok(Some(
                state
                    .last
                    .as_ref()
                    .map(|l| l.outcome)
                    .unwrap_or(Outcome::RolledBack),
            ));
        }

        // It runs. It counts once the system finished starting and the session stays up.
        loop {
            if let Some(first) = sys.healthy(self.config.health.as_ref()) {
                std::thread::sleep(self.settle);
                if sys.healthy(self.config.health.as_ref()) == Some(first) {
                    break;
                }
            } else {
                std::thread::sleep(self.settle.min(Duration::from_secs(5)));
            }
        }

        let _lock = Lock::take(&self.paths)?;
        let mut state = State::load(&self.paths);
        if target.trial {
            let _ = sys.set_oneshot("");
            // Still the generation the trial left as default: now the new one is.
            // Anything else is a rebuild made since, which already holds the release.
            if let Some(generation) = target.generation {
                if sys.current_generation().ok() == target.previous_generation {
                    sys.switch_generation(generation)?;
                    if let Some(system) = sys.generation_system(generation) {
                        sys.write_boot_menu(&system)?;
                    }
                }
            }
        }
        let _ = std::fs::remove_file(self.paths.built_root());
        let outcome = if target.back {
            state.previous = None;
            Outcome::WentBack
        } else {
            state.previous = target.previous_generation.map(|generation| Previous {
                version: target.previous_version.clone(),
                generation,
            });
            Outcome::Installed
        };
        if state.available.as_ref().is_some_and(|offer| {
            offer
                .manifest
                .version()
                .ok()
                .zip(target.version.parse::<Version>().ok())
                .is_some_and(|(offered, installed)| offered <= installed)
        }) {
            state.available = None;
        }
        state.finish(&target.version, outcome, "");
        state.save(&self.paths)?;
        Ok(Some(outcome))
    }

    /// Makes the version before the last update the default again; the next start runs it.
    pub fn back(&self) -> Result<Target> {
        let _lock = Lock::take(&self.paths)?;
        let mut state = State::load(&self.paths);
        self.recover(&mut state)?;
        if state.phase == Phase::Restart {
            bail!("an update waits for a restart; cancel it instead (slateos-update cancel)");
        }
        let previous = state
            .previous
            .clone()
            .ok_or_else(|| anyhow!("there is no earlier version to go back to"))?;
        let sys = self.sys();
        let system = sys.generation_system(previous.generation).ok_or_else(|| {
            anyhow!(
                "SlateOS {} is no longer on this computer (its generation was removed)",
                previous.version
            )
        })?;
        let old_source = self.previous_source();
        if system::source_version(&old_source).ok().as_deref() != Some(previous.version.as_str()) {
            bail!(
                "the source of SlateOS {} is no longer in {}",
                previous.version,
                old_source.display()
            );
        }
        let current = sys.current_generation()?;
        let target = Target {
            version: previous.version.clone(),
            previous_version: self.config.version.clone(),
            swapped: true,
            system: Some(system.to_string_lossy().to_string()),
            generation: Some(previous.generation),
            previous_generation: Some(current),
            trial: false,
            back: true,
            boot: sys.boot_id(),
        };
        // The two sources change places; the boot menu's default is the old system.
        let aside = self.source().with_extension("aside");
        let _ = std::fs::remove_dir_all(&aside);
        std::fs::rename(self.source(), &aside)?;
        std::fs::rename(&old_source, self.source())?;
        std::fs::rename(&aside, &old_source)?;
        sys.switch_generation(previous.generation)?;
        sys.write_boot_menu(&system)?;
        state.skip = Some(self.config.version.clone());
        state.phase = Phase::Restart;
        state.target = Some(target.clone());
        state.save(&self.paths)?;
        Ok(target)
    }

    fn undo_back(&self, target: &Target) -> Result<()> {
        let sys = self.sys();
        let aside = self.source().with_extension("aside");
        let _ = std::fs::remove_dir_all(&aside);
        std::fs::rename(self.source(), &aside)?;
        std::fs::rename(self.previous_source(), self.source())?;
        std::fs::rename(&aside, self.previous_source())?;
        if let Some(generation) = target.previous_generation {
            sys.switch_generation(generation)?;
            if let Some(system) = sys.generation_system(generation) {
                sys.write_boot_menu(&system)?;
            }
        }
        Ok(())
    }
}
