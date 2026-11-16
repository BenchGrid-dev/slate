//! Whole updates in a directory: the real updater, a release signed and published as
//! files, and stand-ins for Nix, the system profile, the boot loader and the firmware
//! that keep just enough state to show what a computer would start next.

use slate_update::config::{Config, Health, Paths};
use slate_update::manifest::{generate_key, sha256_hex, sign};
use slate_update::state::{Outcome, Phase, State, Target};
use slate_update::update::{Asked, Updater};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

const NIX: &str = r#"#!/bin/sh
root=$(cd "$(dirname "$0")/.." && pwd -P)
echo "nix $*" >> "$root/calls"
[ -e "$root/fail-nixpkgs" ] && { echo "error: unable to download 'https://github.com/NixOS/nixpkgs'" >&2; exit 1; }
printf '%s' "$root/nix/store/nixpkgs-source"
"#;

const NIX_INSTANTIATE: &str = r#"#!/bin/sh
root=$(cd "$(dirname "$0")/.." && pwd -P)
echo "nix-instantiate $*" >> "$root/calls"
[ -e "$root/fail-instantiate" ] && { echo "error: attribute 'slate' missing" >&2; exit 1; }
echo "warning: something harmless" >&2
echo "$root/nix/store/system.drv"
"#;

// Builds "the system" of whatever source is in place: a directory named after its
// version, with the boot menu writer every NixOS system carries.
const NIX_STORE: &str = r#"#!/bin/sh
root=$(cd "$(dirname "$0")/.." && pwd -P)
echo "nix-store $*" >> "$root/calls"
case "$*" in
  *--dry-run*)
    echo "these 2 derivations will be built:" >&2
    echo "  /nix/store/a.drv" >&2
    echo "  /nix/store/b.drv" >&2
    echo "these 3 paths will be fetched (1.00 MiB download, 4.00 MiB unpacked):" >&2
    exit 0;;
esac
[ -e "$root/fail-build" ] && { echo "error: builder for '/nix/store/a.drv' failed with exit code 1" >&2; exit 1; }
version=$(sed -n 's/^version = "\(.*\)"/\1/p' "$root/etc/slateos/slate/Cargo.toml")
system="$root/nix/store/system-$version"
mkdir -p "$system/bin"
cp "$root/bin/switch-to-configuration" "$system/bin/"
echo "copying path '/nix/store/x' from 'https://cache.nixos.org'..." >&2
echo "building '/nix/store/a.drv'..." >&2
while [ $# -gt 0 ]; do
  if [ "$1" = --add-root ]; then ln -sfn "$system" "$2"; echo "$2"; fi
  shift
done
"#;

const NIX_ENV: &str = r#"#!/bin/sh
root=$(cd "$(dirname "$0")/.." && pwd -P)
echo "nix-env $*" >> "$root/calls"
profile=
while [ $# -gt 0 ]; do
  case "$1" in
    -p) profile=$2; dir=$(dirname "$profile"); shift 2;;
    --set)
      # Like Nix: the path the current generation already has makes no new one.
      current=$(readlink "$profile" 2>/dev/null)
      if [ -n "$current" ] && [ "$(readlink "$dir/$current")" = "$2" ]; then exit 0; fi
      max=0
      for link in "$dir"/system-*-link; do
        [ -L "$link" ] || continue
        n=${link##*/system-}; n=${n%-link}
        [ "$n" -gt "$max" ] && max=$n
      done
      n=$((max + 1))
      ln -s "$2" "$dir/system-$n-link"
      ln -sfn "system-$n-link" "$profile"
      shift 2;;
    --switch-generation)
      [ -L "$dir/system-$2-link" ] || { echo "error: generation $2 does not exist" >&2; exit 1; }
      ln -sfn "system-$2-link" "$profile"
      shift 2;;
    --delete-generations)
      rm -f "$dir/system-$2-link"
      shift 2;;
    *) shift;;
  esac
done
"#;

// An entry for every generation; the default is the generation of the system whose
// writer runs, as NixOS's does it.
const SWITCH: &str = r#"#!/bin/sh
self=$(cd "$(dirname "$0")/.." && pwd -P)
root=$(cd "$self/../../.." && pwd -P)
echo "boot-menu $(basename "$self") $*" >> "$root/calls"
[ -e "$root/fail-boot-menu" ] && { echo "error: no space left on /boot" >&2; exit 1; }
entries="$root/boot/loader/entries"
mkdir -p "$entries"
rm -f "$entries"/nixos-generation-*.conf
for link in "$root"/nix/var/nix/profiles/system-*-link; do
  [ -L "$link" ] || continue
  n=${link##*/system-}; n=${n%-link}
  echo "title SlateOS" > "$entries/nixos-generation-$n.conf"
  if [ "$(cd "$link" && pwd -P)" = "$self" ]; then
    echo "default nixos-generation-$n.conf" > "$root/boot/loader/loader.conf"
  fi
done
"#;

const BOOTCTL: &str = r#"#!/bin/sh
root=$(cd "$(dirname "$0")/.." && pwd -P)
echo "bootctl $*" >> "$root/calls"
case "$1" in
  is-installed) [ -e "$root/no-systemd-boot" ] && exit 1; exit 0;;
  --print-boot-path) echo "$root/boot";;
  set-oneshot)
    [ -e "$root/fail-oneshot" ] && { echo "Failed to write EFI variable" >&2; exit 1; }
    printf '%s' "$2" > "$root/oneshot";;
esac
"#;

const SYSTEMCTL: &str = r#"#!/bin/sh
root=$(cd "$(dirname "$0")/.." && pwd -P)
cat "$root/system-state" 2>/dev/null || echo running
"#;

/// A computer with SlateOS 0.3.0 installed, and a release server next to it.
struct Machine {
    _dir: tempfile::TempDir,
    root: PathBuf,
    secret: String,
    public: String,
}

fn source(dir: &Path, version: &str) {
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(
        dir.join("Cargo.toml"),
        format!("[workspace]\n[workspace.package]\nversion = \"{version}\"\n"),
    )
    .unwrap();
    std::fs::write(dir.join("flake.nix"), "{ }\n").unwrap();
}

fn tarball(version: &str, claims: &str) -> Vec<u8> {
    let gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    let mut tar = tar::Builder::new(gz);
    let cargo = format!("[workspace]\n[workspace.package]\nversion = \"{claims}\"\n");
    for (name, body) in [("Cargo.toml", cargo.as_str()), ("flake.nix", "{ }\n")] {
        let mut header = tar::Header::new_gnu();
        header.set_size(body.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        tar.append_data(
            &mut header,
            format!("slateos-{version}/{name}"),
            body.as_bytes(),
        )
        .unwrap();
    }
    tar.into_inner().unwrap().finish().unwrap()
}

impl Machine {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let (secret, public) = generate_key().unwrap();
        let machine = Self {
            _dir: dir,
            root,
            secret,
            public,
        };
        for (name, body) in [
            ("nix", NIX),
            ("nix-instantiate", NIX_INSTANTIATE),
            ("nix-store", NIX_STORE),
            ("nix-env", NIX_ENV),
            ("switch-to-configuration", SWITCH),
            ("bootctl", BOOTCTL),
            ("systemctl", SYSTEMCTL),
        ] {
            let path = machine.path("bin").join(name);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, body).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        source(&machine.path("etc/slateos/slate"), "0.3.0");
        std::fs::write(machine.path("etc/slateos/configuration.nix"), "{ }\n").unwrap();
        // Firmware that keeps its variables: the boot entry the installation wrote.
        std::fs::create_dir_all(machine.path("sys/firmware/efi/efivars")).unwrap();
        std::fs::write(
            machine.path("sys/firmware/efi/efivars/Boot0001-8be4df61-93ca-11d2-aa0d-00e098032b8c"),
            "Linux Boot Manager"
                .encode_utf16()
                .flat_map(u16::to_le_bytes)
                .collect::<Vec<u8>>(),
        )
        .unwrap();
        std::fs::create_dir_all(machine.path("release")).unwrap();
        // The installed system: generation 1, the default, and running.
        let system = machine.path("nix/store/system-0.3.0");
        std::fs::create_dir_all(system.join("bin")).unwrap();
        std::fs::copy(
            machine.path("bin/switch-to-configuration"),
            system.join("bin/switch-to-configuration"),
        )
        .unwrap();
        let profiles = machine.path("nix/var/nix/profiles");
        std::fs::create_dir_all(&profiles).unwrap();
        std::os::unix::fs::symlink(&system, profiles.join("system-1-link")).unwrap();
        std::os::unix::fs::symlink("system-1-link", profiles.join("system")).unwrap();
        std::fs::create_dir_all(machine.path("run")).unwrap();
        std::fs::create_dir_all(machine.path("boot/loader/entries")).unwrap();
        std::fs::write(
            machine.path("boot/loader/entries/nixos-generation-1.conf"),
            "title SlateOS\n",
        )
        .unwrap();
        std::fs::write(
            machine.path("boot/loader/loader.conf"),
            "default nixos-generation-1.conf\n",
        )
        .unwrap();
        machine.run_generation(1);
        machine.new_boot();
        machine
    }

    fn path(&self, relative: &str) -> PathBuf {
        self.root.join(relative)
    }

    fn flag(&self, name: &str, on: bool) {
        if on {
            std::fs::write(self.path(name), "").unwrap();
        } else {
            let _ = std::fs::remove_file(self.path(name));
        }
    }

    /// Publishes a release: its source and the signed manifest that names it.
    fn publish(&self, version: &str) {
        self.publish_as(version, version, None);
    }

    /// A release that names a binary cache of its own.
    fn publish_with_cache(&self, version: &str) {
        self.publish(version);
        let path = self.path("release/slateos-update.json");
        let manifest = std::fs::read_to_string(&path).unwrap().replace(
            "\"notes\"",
            "\"cache\":{\"url\":\"https://cache.example.org/slateos\",\"key\":\"slateos-1:AAAA=\"},\"notes\"",
        );
        std::fs::write(&path, &manifest).unwrap();
        std::fs::write(
            self.path("release/slateos-update.json.sig"),
            sign(&self.secret, manifest.as_bytes()).unwrap(),
        )
        .unwrap();
    }

    fn publish_as(&self, version: &str, claims: &str, wrong_hash: Option<&str>) {
        let archive = tarball(version, claims);
        let name = format!("slateos-{version}-source.tar.gz");
        std::fs::write(self.path("release").join(&name), &archive).unwrap();
        let hash = wrong_hash
            .map(String::from)
            .unwrap_or_else(|| sha256_hex(&archive));
        let manifest = format!(
            r#"{{"format":1,"version":"{version}","released":"2026-12-01","summary":"Faster.",
               "notes":"https://example.org/notes","source":{{"url":"{name}","sha256":"{hash}","size":{}}}}}"#,
            archive.len()
        );
        std::fs::write(self.path("release/slateos-update.json"), &manifest).unwrap();
        std::fs::write(
            self.path("release/slateos-update.json.sig"),
            sign(&self.secret, manifest.as_bytes()).unwrap(),
        )
        .unwrap();
    }

    /// The updater of the system that runs: its version is the running system's.
    fn updater(&self) -> Updater {
        let running = std::fs::canonicalize(self.path("run/current-system")).unwrap();
        let version = running
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .trim_start_matches("system-")
            .to_string();
        let config = Config {
            version,
            url: format!(
                "file://{}",
                self.path("release/slateos-update.json").display()
            ),
            keys: vec![self.public.clone()],
            config: "/etc/slateos".into(),
            health: None,
            trial: true,
            free_gib: 0,
        };
        let mut updater = Updater::with(Paths::under(&self.root), config);
        updater.settle = Duration::from_millis(10);
        updater
    }

    /// Every start of the computer has its own id.
    fn new_boot(&self) {
        let file = self.path("proc/sys/kernel/random/boot_id");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        let n = std::fs::read_to_string(&file)
            .ok()
            .and_then(|s| s.trim().parse::<u64>().ok())
            .unwrap_or(0);
        std::fs::write(&file, format!("{}\n", n + 1)).unwrap();
    }

    fn run_generation(&self, generation: u64) {
        let link = self
            .path("nix/var/nix/profiles")
            .join(format!("system-{generation}-link"));
        let system = std::fs::canonicalize(link).unwrap();
        let current = self.path("run/current-system");
        let _ = std::fs::remove_file(&current);
        std::os::unix::fs::symlink(system, current).unwrap();
    }

    fn default_generation(&self) -> u64 {
        let conf = std::fs::read_to_string(self.path("boot/loader/loader.conf")).unwrap();
        conf.trim()
            .trim_start_matches("default nixos-generation-")
            .trim_end_matches(".conf")
            .parse()
            .unwrap()
    }

    fn oneshot(&self) -> String {
        std::fs::read_to_string(self.path("oneshot")).unwrap_or_default()
    }

    /// What the firmware and systemd-boot do at a start: the entry named for one start
    /// if there is one (and it is forgotten), else the default. `comes_up` false is a
    /// system that hangs: the person turns the computer off and on, which starts the
    /// default.
    fn restart(&self, comes_up: bool) -> u64 {
        let once = self.oneshot();
        let _ = std::fs::remove_file(self.path("oneshot"));
        let entry_of = |name: &str| -> Option<u64> {
            self.path("boot/loader/entries")
                .join(name)
                .exists()
                .then(|| {
                    name.trim_start_matches("nixos-generation-")
                        .trim_end_matches(".conf")
                        .parse()
                        .unwrap()
                })
        };
        let first = (!once.is_empty())
            .then(|| entry_of(&once))
            .flatten()
            .unwrap_or_else(|| self.default_generation());
        let generation = if comes_up {
            first
        } else {
            self.default_generation()
        };
        self.run_generation(generation);
        self.new_boot();
        generation
    }

    fn state(&self) -> State {
        State::load(&Paths::under(&self.root))
    }

    fn version_in(&self, dir: &str) -> Option<String> {
        slate_update::system::source_version(&self.path("etc/slateos").join(dir)).ok()
    }

    fn generations(&self) -> Vec<u64> {
        let mut all: Vec<u64> = std::fs::read_dir(self.path("nix/var/nix/profiles"))
            .unwrap()
            .flatten()
            .filter_map(|e| {
                e.file_name()
                    .to_str()?
                    .strip_prefix("system-")?
                    .strip_suffix("-link")?
                    .parse()
                    .ok()
            })
            .collect();
        all.sort_unstable();
        all
    }

    fn current_generation(&self) -> u64 {
        std::fs::read_link(self.path("nix/var/nix/profiles/system"))
            .unwrap()
            .to_str()
            .unwrap()
            .trim_start_matches("system-")
            .trim_end_matches("-link")
            .parse()
            .unwrap()
    }

    fn calls(&self) -> String {
        std::fs::read_to_string(self.path("calls")).unwrap_or_default()
    }

    /// Everything is as it was before an update was tried.
    fn assert_untouched(&self) {
        assert_eq!(self.version_in("slate").as_deref(), Some("0.3.0"));
        assert_eq!(self.version_in("slate.previous"), None);
        assert_eq!(self.version_in("slate.next"), None);
        assert_eq!(self.generations(), vec![1]);
        assert_eq!(self.current_generation(), 1);
        assert_eq!(self.default_generation(), 1);
        assert_eq!(self.oneshot(), "");
        assert_eq!(self.state().phase, Phase::Idle);
        assert!(self.state().target.is_none());
    }
}

fn install(machine: &Machine, asked: Asked) -> anyhow::Result<Option<Target>> {
    let mut said = Vec::new();
    machine
        .updater()
        .install(asked, &mut |line| said.push(line.to_string()))
}

#[test]
fn an_update_is_started_once_and_becomes_the_default_when_it_runs() {
    let machine = Machine::new();
    machine.publish("0.3.1");
    assert_eq!(
        machine
            .updater()
            .check(true)
            .unwrap()
            .unwrap()
            .manifest
            .version,
        "0.3.1"
    );
    assert_eq!(
        machine.state().available.unwrap().manifest.summary,
        "Faster."
    );

    let target = install(&machine, Asked::Person).unwrap().unwrap();
    assert_eq!(target.version, "0.3.1");
    assert_eq!(target.previous_version, "0.3.0");
    assert!(target.trial);
    assert_eq!(
        (target.previous_generation, target.generation),
        (Some(1), Some(2))
    );
    // The new source is in place and the old one kept; the new system is a generation,
    // but what starts by default is still the version that runs.
    assert_eq!(machine.version_in("slate").as_deref(), Some("0.3.1"));
    assert_eq!(
        machine.version_in("slate.previous").as_deref(),
        Some("0.3.0")
    );
    assert_eq!(machine.generations(), vec![1, 2]);
    assert_eq!(machine.current_generation(), 1);
    assert_eq!(machine.default_generation(), 1);
    assert_eq!(machine.oneshot(), "nixos-generation-2.conf");
    let state = machine.state();
    assert_eq!(state.phase, Phase::Restart);
    assert!(state.progress.is_none());
    // The system was built against the nixpkgs the new source is locked to.
    let calls = machine.calls();
    assert!(calls.contains("inputs.nixpkgs.outPath"), "{calls}");
    assert!(calls.contains(&format!(
        "-I nixpkgs={}",
        machine.path("nix/store/nixpkgs-source").display()
    )));
    assert!(calls.contains("nixos-config="), "{calls}");
    // A second request says what is the case instead of building again.
    let again = install(&machine, Asked::Person).unwrap_err().to_string();
    assert!(again.contains("starts at the next restart"), "{again}");
    // The system's units are started anew by a rebuild: asked before the restart which
    // version came up, the answer is "none yet", not "the old one, so undo".
    assert_eq!(machine.updater().boot().unwrap(), None);
    assert_eq!(machine.state().phase, Phase::Restart);
    assert_eq!(machine.oneshot(), "nixos-generation-2.conf");
    assert_eq!(machine.version_in("slate").as_deref(), Some("0.3.1"));

    assert_eq!(machine.restart(true), 2);
    assert_eq!(machine.updater().boot().unwrap(), Some(Outcome::Installed));
    assert_eq!(machine.current_generation(), 2);
    assert_eq!(machine.default_generation(), 2);
    let state = machine.state();
    assert_eq!(state.phase, Phase::Idle);
    assert!(state.available.is_none());
    assert_eq!(state.last.as_ref().unwrap().outcome, Outcome::Installed);
    assert_eq!(state.previous.as_ref().unwrap().version, "0.3.0");
    // The start after that is the new version too, and there is nothing left to do.
    assert_eq!(machine.restart(true), 2);
    assert_eq!(machine.updater().boot().unwrap(), None);
    assert!(install(&machine, Asked::Person).unwrap().is_none());
}

#[test]
fn a_release_brings_its_binary_cache_along() {
    let machine = Machine::new();
    machine.publish("0.3.1");
    install(&machine, Asked::Person).unwrap().unwrap();
    assert!(!machine.calls().contains("extra-substituters"));

    let machine = Machine::new();
    machine.publish_with_cache("0.3.1");
    install(&machine, Asked::Person).unwrap().unwrap();
    let calls = machine.calls();
    // Asked for the estimate and for the build, with the key its paths are signed with.
    let with_cache = calls
        .lines()
        .filter(|l| {
            l.contains("--option extra-substituters https://cache.example.org/slateos")
                && l.contains("--option extra-trusted-public-keys slateos-1:AAAA=")
        })
        .count();
    assert_eq!(with_cache, 2, "{calls}");
}

#[test]
fn a_version_that_does_not_come_up_is_gone_at_the_next_start() {
    let machine = Machine::new();
    machine.publish("0.3.1");
    install(&machine, Asked::Person).unwrap().unwrap();

    // It hangs; the computer is turned off and on, and the default is the old version.
    assert_eq!(machine.restart(false), 1);
    assert_eq!(machine.updater().boot().unwrap(), Some(Outcome::RolledBack));
    machine.assert_untouched();
    let last = machine.state().last.unwrap();
    assert_eq!(last.outcome, Outcome::RolledBack);
    assert!(last.message.contains("did not start"), "{}", last.message);
    assert!(!machine
        .path("boot/loader/entries/nixos-generation-2.conf")
        .exists());

    // The timer does not try the same version again by itself; the person may.
    assert!(install(&machine, Asked::Timer).unwrap().is_none());
    machine.assert_untouched();
    // (Like Nix, the stand-in numbers the new generation after the newest that exists.)
    assert!(install(&machine, Asked::Person).unwrap().is_some());
    assert_eq!(machine.restart(true), 2);
    assert_eq!(machine.updater().boot().unwrap(), Some(Outcome::Installed));
    assert_eq!(machine.default_generation(), 2);
}

#[test]
fn what_cannot_be_built_changes_nothing() {
    for failure in [
        "fail-nixpkgs",
        "fail-instantiate",
        "fail-build",
        "fail-boot-menu",
    ] {
        let machine = Machine::new();
        machine.publish("0.3.1");
        machine.flag(failure, true);
        let error = format!("{:#}", install(&machine, Asked::Person).unwrap_err());
        machine.flag(failure, false);
        // The boot menu is rewritten by the old system once the fault is gone; while
        // it lasts, nothing else may be left changed.
        assert_eq!(
            machine.version_in("slate").as_deref(),
            Some("0.3.0"),
            "{failure}"
        );
        assert_eq!(machine.version_in("slate.previous"), None, "{failure}");
        assert_eq!(machine.generations(), vec![1], "{failure}");
        assert_eq!(machine.default_generation(), 1, "{failure}");
        assert_eq!(machine.oneshot(), "", "{failure}");
        let state = machine.state();
        assert_eq!(state.phase, Phase::Idle, "{failure}");
        let last = state.last.unwrap();
        assert_eq!(last.outcome, Outcome::Failed);
        assert_eq!(last.version, "0.3.1");
        // What Nix said last is what the person is told.
        let expected = match failure {
            "fail-nixpkgs" => "unable to download",
            "fail-instantiate" => "attribute 'slate' missing",
            "fail-build" => "builder for '/nix/store/a.drv' failed",
            _ => "no space left on /boot",
        };
        assert!(error.contains(expected), "{failure}: {error}");
        assert!(
            last.message.contains(expected),
            "{failure}: {}",
            last.message
        );
        // And it works when asked again.
        assert!(
            install(&machine, Asked::Person).unwrap().is_some(),
            "{failure}"
        );
    }
}

#[test]
fn a_source_that_is_not_the_releases_is_refused() {
    let machine = Machine::new();
    machine.publish_as("0.3.1", "0.3.1", Some(&"0".repeat(64)));
    let error = install(&machine, Asked::Person).unwrap_err().to_string();
    assert!(error.contains("checksum"), "{error}");
    machine.assert_untouched();

    // The right bytes of the wrong version.
    machine.publish_as("0.3.1", "0.2.9", None);
    let error = install(&machine, Asked::Person).unwrap_err().to_string();
    assert!(error.contains("0.2.9"), "{error}");
    machine.assert_untouched();

    // A manifest somebody else signed.
    machine.publish("0.3.1");
    let (stranger, _) = generate_key().unwrap();
    let manifest = std::fs::read(machine.path("release/slateos-update.json")).unwrap();
    std::fs::write(
        machine.path("release/slateos-update.json.sig"),
        sign(&stranger, &manifest).unwrap(),
    )
    .unwrap();
    let error = format!("{:#}", install(&machine, Asked::Person).unwrap_err());
    assert!(
        error.contains("not signed with a key this system trusts"),
        "{error}"
    );
    machine.assert_untouched();
    assert!(machine.state().check_error.is_some());
    assert!(!machine.calls().contains("nix-store"));
}

#[test]
fn without_a_trial_the_new_version_is_the_default_at_once() {
    for obstacle in [
        "no-efivars",
        "forgetful-firmware",
        "no-systemd-boot",
        "fail-oneshot",
    ] {
        let machine = Machine::new();
        machine.publish("0.3.1");
        match obstacle {
            "no-efivars" => std::fs::remove_dir_all(machine.path("sys/firmware/efi")).unwrap(),
            // The variables are there to write, but the entry the installation made is not.
            "forgetful-firmware" => {
                let vars = machine.path("sys/firmware/efi/efivars");
                std::fs::remove_dir_all(&vars).unwrap();
                std::fs::create_dir_all(&vars).unwrap();
            }
            _ => machine.flag(obstacle, true),
        }
        let target = install(&machine, Asked::Person).unwrap().unwrap();
        assert!(!target.trial, "{obstacle}");
        assert_eq!(machine.current_generation(), 2, "{obstacle}");
        assert_eq!(machine.default_generation(), 2, "{obstacle}");
        assert_eq!(machine.oneshot(), "", "{obstacle}");
        assert_eq!(machine.restart(true), 2);
        assert_eq!(machine.updater().boot().unwrap(), Some(Outcome::Installed));
        assert_eq!(machine.default_generation(), 2);
    }

    // Without a trial a version that hangs is the default; the person picks the old
    // entry in the boot menu, and that start puts everything back.
    let machine = Machine::new();
    machine.publish("0.3.1");
    machine.flag("no-systemd-boot", true);
    install(&machine, Asked::Person).unwrap().unwrap();
    machine.run_generation(1);
    machine.new_boot();
    assert_eq!(machine.updater().boot().unwrap(), Some(Outcome::RolledBack));
    machine.assert_untouched();
}

#[test]
fn an_update_can_be_taken_back_before_its_restart() {
    let machine = Machine::new();
    machine.publish("0.3.1");
    assert!(
        machine.updater().cancel().is_err(),
        "nothing to take back yet"
    );
    install(&machine, Asked::Person).unwrap().unwrap();
    assert_eq!(machine.updater().cancel().unwrap(), "0.3.1");
    machine.assert_untouched();
    assert_eq!(machine.state().last.unwrap().outcome, Outcome::Cancelled);
    assert_eq!(machine.restart(true), 1);
    assert_eq!(machine.updater().boot().unwrap(), None);
}

#[test]
fn an_installation_the_power_cut_short_is_put_back_at_the_next_start() {
    let machine = Machine::new();
    machine.publish("0.3.1");
    // As the updater leaves things while it builds: sources swapped, state says so.
    let config = machine.path("etc/slateos");
    std::fs::rename(config.join("slate"), config.join("slate.previous")).unwrap();
    source(&config.join("slate"), "0.3.1");
    let mut state = State {
        phase: Phase::Building,
        target: Some(Target {
            version: "0.3.1".into(),
            previous_version: "0.3.0".into(),
            swapped: true,
            ..Default::default()
        }),
        ..Default::default()
    };
    state.save(&Paths::under(&machine.root)).unwrap();

    assert_eq!(machine.updater().boot().unwrap(), Some(Outcome::Failed));
    machine.assert_untouched();
    assert!(machine
        .state()
        .last
        .unwrap()
        .message
        .contains("interrupted"));
}

#[test]
fn the_version_before_can_be_gone_back_to() {
    let machine = Machine::new();
    machine.publish("0.3.1");
    assert!(machine.updater().back().is_err(), "nothing before yet");
    install(&machine, Asked::Person).unwrap().unwrap();
    machine.restart(true);
    machine.updater().boot().unwrap();

    let target = machine.updater().back().unwrap();
    assert_eq!(target.version, "0.3.0");
    assert_eq!(machine.version_in("slate").as_deref(), Some("0.3.0"));
    assert_eq!(
        machine.version_in("slate.previous").as_deref(),
        Some("0.3.1")
    );
    assert_eq!(machine.default_generation(), 1);
    assert_eq!(machine.restart(true), 1);
    assert_eq!(machine.updater().boot().unwrap(), Some(Outcome::WentBack));
    let state = machine.state();
    assert_eq!(state.skip.as_deref(), Some("0.3.1"));
    assert!(state.previous.is_none());

    // The timer leaves the version alone the person went back from; asking installs it.
    assert!(install(&machine, Asked::Timer).unwrap().is_none());
    assert_eq!(machine.version_in("slate").as_deref(), Some("0.3.0"));
    let target = install(&machine, Asked::Person).unwrap().unwrap();
    assert_eq!(target.version, "0.3.1");
    assert!(machine.state().skip.is_none());

    // Going back can itself be taken back before the restart.
    machine.restart(true);
    machine.updater().boot().unwrap();
    machine.updater().back().unwrap();
    assert_eq!(machine.updater().cancel().unwrap(), "0.3.0");
    assert_eq!(machine.version_in("slate").as_deref(), Some("0.3.1"));
    assert_eq!(
        machine.version_in("slate.previous").as_deref(),
        Some("0.3.0")
    );
    assert_eq!(machine.default_generation(), machine.current_generation(),);
}

#[test]
fn a_new_version_counts_once_the_session_stays_up() {
    let machine = Machine::new();
    machine.publish("0.3.1");
    install(&machine, Asked::Person).unwrap().unwrap();
    machine.restart(true);

    std::fs::create_dir_all(machine.path("etc")).unwrap();
    std::fs::write(
        machine.path("etc/passwd"),
        "mira:x:1000:100::/home/mira:/bin/sh\n",
    )
    .unwrap();
    std::fs::write(machine.path("system-state"), "starting\n").unwrap();
    let mut updater = machine.updater();
    updater.config.health = Some(Health {
        user: "mira".into(),
        processes: vec!["sway".into(), "slate-desktop".into()],
    });
    let root = machine.root.clone();
    let session = std::thread::spawn(move || {
        let process = |pid: u32, name: &str| {
            let dir = root.join("proc").join(pid.to_string());
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(
                dir.join("status"),
                format!("Name:\t{name}\nUid:\t1000\t1000\t1000\t1000\n"),
            )
            .unwrap();
        };
        std::thread::sleep(Duration::from_millis(150));
        // Still on trial: the system is starting, then the compositor alone is up.
        assert_eq!(State::load(&Paths::under(&root)).phase, Phase::Restart);
        std::fs::write(root.join("system-state"), "degraded\n").unwrap();
        process(700, "sway");
        std::thread::sleep(Duration::from_millis(150));
        assert_eq!(State::load(&Paths::under(&root)).phase, Phase::Restart);
        process(710, "slate-desktop");
    });
    assert_eq!(updater.boot().unwrap(), Some(Outcome::Installed));
    session.join().unwrap();
    assert_eq!(machine.default_generation(), 2);
}

#[test]
fn the_timer_waits_for_the_charger() {
    let machine = Machine::new();
    machine.publish("0.3.1");
    let battery = machine.path("sys/class/power_supply/BAT0");
    std::fs::create_dir_all(&battery).unwrap();
    std::fs::write(battery.join("type"), "Battery\n").unwrap();
    std::fs::write(battery.join("status"), "Discharging\n").unwrap();
    assert!(install(&machine, Asked::Timer).unwrap().is_none());
    machine.assert_untouched();
    // The check itself was recorded, so Settings can offer the update.
    assert_eq!(machine.state().available.unwrap().manifest.version, "0.3.1");
    assert!(install(&machine, Asked::Person).unwrap().is_some());
}

#[test]
fn a_system_that_follows_its_own_configuration_is_left_alone() {
    let machine = Machine::new();
    machine.publish("0.3.1");
    std::fs::remove_file(machine.path("etc/slateos/configuration.nix")).unwrap();
    let error = install(&machine, Asked::Person).unwrap_err().to_string();
    assert!(error.contains("follows its own configuration"), "{error}");
    assert_eq!(machine.generations(), vec![1]);
}
