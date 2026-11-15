//! The computer under the updater: Nix, the system profile and its generations, the
//! boot loader, and what tells a working system from one that did not come up.

use crate::config::{Health, Paths};
use anyhow::{anyhow, bail, Context, Result};
use std::ffi::OsStr;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// What a build will take, from Nix's dry run.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Plan {
    /// Derivations to build here.
    pub build: u64,
    /// Store paths to download.
    pub fetch: u64,
    pub download_bytes: u64,
    pub unpacked_bytes: u64,
}

pub struct Sys<'a> {
    pub paths: &'a Paths,
}

fn tail(text: &str, lines: usize) -> String {
    let all: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    all[all.len().saturating_sub(lines)..].join("\n")
}

impl<'a> Sys<'a> {
    pub fn new(paths: &'a Paths) -> Self {
        Self { paths }
    }

    /// One more line in the log Settings shows.
    pub fn log(&self, line: &str) {
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.paths.log_file())
        {
            let _ = writeln!(file, "{line}");
        }
    }

    pub fn start_log(&self) {
        let _ = std::fs::create_dir_all(self.paths.state_dir());
        let _ = std::fs::write(self.paths.log_file(), "");
    }

    fn command(&self, program: impl AsRef<OsStr>, args: &[&str]) -> Command {
        let mut command = Command::new(program);
        command.args(args).stdin(Stdio::null());
        // Left over from an installation, it would make NixOS reinstall the boot loader.
        command.env_remove("NIXOS_INSTALL_BOOTLOADER");
        command
    }

    /// Runs to the end; its output, or an error that ends with what it printed last.
    fn run(&self, program: impl AsRef<OsStr>, args: &[&str]) -> Result<String> {
        let name = program.as_ref().to_string_lossy().to_string();
        self.log(&format!("$ {name} {}", args.join(" ")));
        let output = self
            .command(&program, args)
            .output()
            .with_context(|| format!("could not run {name}"))?;
        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        if !stderr.trim().is_empty() {
            self.log(stderr.trim_end());
        }
        if !output.status.success() {
            bail!(
                "{name} failed:\n{}",
                tail(&format!("{stdout}\n{stderr}"), 12)
            );
        }
        Ok(stdout)
    }

    fn tool(&self, name: &str) -> PathBuf {
        self.paths.tool(name)
    }

    /// The nixpkgs tree the Slate source in `dir` is locked to, as a store path. Nix
    /// fetches it when the store does not hold it yet.
    pub fn pinned_nixpkgs(&self, dir: &Path) -> Result<String> {
        let dir = dir.to_string_lossy();
        if dir.contains(['"', '\\', '$']) {
            bail!("{dir} cannot be named in a Nix expression");
        }
        let expr = format!("(builtins.getFlake \"path:{dir}\").inputs.nixpkgs.outPath");
        let out = self
            .run(
                self.tool("nix"),
                &[
                    "--extra-experimental-features",
                    "nix-command flakes",
                    "eval",
                    "--impure",
                    "--raw",
                    "--option",
                    "flake-registry",
                    "",
                    "--expr",
                    &expr,
                ],
            )
            .context("the nixpkgs this release is built on could not be fetched")?;
        let path = out.trim().to_string();
        if path.is_empty() {
            bail!("Nix named no nixpkgs for this release");
        }
        Ok(path)
    }

    /// Evaluates the system configuration against `nixpkgs`; the derivation of the system.
    pub fn instantiate(&self, nixpkgs: &str, configuration: &Path) -> Result<String> {
        let out = self
            .run(
                self.tool("nix-instantiate"),
                &[
                    "<nixpkgs/nixos>",
                    "-A",
                    "system",
                    "-I",
                    &format!("nixpkgs={nixpkgs}"),
                    "-I",
                    &format!("nixos-config={}", configuration.display()),
                    "--option",
                    "extra-experimental-features",
                    "nix-command flakes",
                    "--option",
                    "flake-registry",
                    "",
                ],
            )
            .context("the system configuration does not evaluate with the new release")?;
        let drv = out
            .lines()
            .map(str::trim)
            .find(|l| l.ends_with(".drv") || l.contains(".drv!"))
            .map(|l| l.split('!').next().unwrap_or(l).to_string())
            .ok_or_else(|| anyhow!("Nix named no derivation for the system"))?;
        Ok(drv)
    }

    /// What building `drv` will download and build, without doing it.
    pub fn plan(&self, drv: &str) -> Result<Plan> {
        let name = self.tool("nix-store");
        self.log(&format!("$ nix-store --realise --dry-run {drv}"));
        let output = self
            .command(&name, &["--realise", "--dry-run", drv])
            .output()
            .context("could not run nix-store")?;
        let text = format!(
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        if !output.status.success() {
            bail!("nix-store failed:\n{}", tail(&text, 12));
        }
        Ok(parse_plan(&text))
    }

    /// Downloads and builds `drv`; the system's store path. `line` hears what Nix prints.
    pub fn realise(&self, drv: &str, line: &mut dyn FnMut(&str)) -> Result<String> {
        let root = self.paths.built_root();
        let _ = std::fs::remove_file(&root);
        let root_text = root.to_string_lossy().to_string();
        let args = [
            "--realise",
            drv,
            "--add-root",
            &root_text,
            // What the caches lack is built here; what they hold is copied even where
            // NixOS marks a small file as "always build".
            "--option",
            "fallback",
            "true",
            "--option",
            "always-allow-substitutes",
            "true",
        ];
        self.log(&format!("$ nix-store {}", args.join(" ")));
        let mut child = self
            .command(self.tool("nix-store"), &args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .context("could not run nix-store")?;
        let stderr = child.stderr.take().expect("piped");
        let mut last = Vec::new();
        for text in BufReader::new(stderr).lines().map_while(Result::ok) {
            self.log(&text);
            line(&text);
            last.push(text);
            if last.len() > 12 {
                last.remove(0);
            }
        }
        let output = child.wait_with_output()?;
        if !output.status.success() {
            bail!("the new system could not be built:\n{}", last.join("\n"));
        }
        // With --add-root Nix prints the root; the system is what it points to.
        let system = std::fs::read_link(&root)
            .ok()
            .or_else(|| {
                String::from_utf8_lossy(&output.stdout)
                    .lines()
                    .last()
                    .map(|l| PathBuf::from(l.trim()))
            })
            .ok_or_else(|| anyhow!("Nix built the system and did not say where"))?;
        Ok(system.to_string_lossy().to_string())
    }

    fn profile_text(&self) -> String {
        self.paths.profile().to_string_lossy().to_string()
    }

    /// The generation the system profile points to: what starts by default.
    pub fn current_generation(&self) -> Result<u64> {
        let link = std::fs::read_link(self.paths.profile())
            .with_context(|| format!("reading {}", self.paths.profile().display()))?;
        generation_number(&link).ok_or_else(|| {
            anyhow!(
                "{} points to {}, which is not a generation",
                self.paths.profile().display(),
                link.display()
            )
        })
    }

    pub fn generation_link(&self, generation: u64) -> PathBuf {
        self.paths
            .profile()
            .with_file_name(format!("system-{generation}-link"))
    }

    /// The store path of a generation; None when it is gone.
    pub fn generation_system(&self, generation: u64) -> Option<PathBuf> {
        std::fs::canonicalize(self.generation_link(generation)).ok()
    }

    /// Makes `system` the newest generation and the profile's current one; its number.
    pub fn add_generation(&self, system: &str) -> Result<u64> {
        self.run(
            self.tool("nix-env"),
            &["-p", &self.profile_text(), "--set", system],
        )?;
        self.current_generation()
    }

    pub fn switch_generation(&self, generation: u64) -> Result<()> {
        self.run(
            self.tool("nix-env"),
            &[
                "-p",
                &self.profile_text(),
                "--switch-generation",
                &generation.to_string(),
            ],
        )?;
        Ok(())
    }

    pub fn delete_generation(&self, generation: u64) -> Result<()> {
        self.run(
            self.tool("nix-env"),
            &[
                "-p",
                &self.profile_text(),
                "--delete-generations",
                &generation.to_string(),
            ],
        )?;
        Ok(())
    }

    /// Writes the boot menu from `system`'s point of view: an entry for every
    /// generation, and `system` as the default.
    pub fn write_boot_menu(&self, system: &Path) -> Result<()> {
        self.run(system.join("bin/switch-to-configuration"), &["boot"])
            .context("the boot menu could not be written")?;
        Ok(())
    }

    /// Lets the updater of the system that was just built enter it in the boot menu:
    /// a release knows best how it wants to be started, and a fault in this step can
    /// then be mended by the release that is being installed.
    pub fn hand_over(&self, updater: &Path, system: &str) -> Result<()> {
        self.run(updater, &["activate", system])?;
        Ok(())
    }

    /// A trial start is possible: systemd-boot starts this computer, and the firmware
    /// keeps the variables it is given. The second is seen from the boot entry the
    /// installation wrote ("Linux Boot Manager"): firmware that forgets variables at a
    /// restart (QEMU without a variable store) has lost that one too, and would forget
    /// which version to try, so that every update looked like one that did not start.
    pub fn can_try_once(&self) -> bool {
        let installed = self
            .command(self.tool("bootctl"), &["is-installed"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success());
        if !installed {
            self.log("no trial start: systemd-boot does not start this computer");
            return false;
        }
        if !self.firmware_keeps_variables() {
            self.log("no trial start: the firmware does not keep the boot loader's variables");
            return false;
        }
        true
    }

    fn firmware_keeps_variables(&self) -> bool {
        let Ok(entries) = std::fs::read_dir(self.paths.efivars()) else {
            return false;
        };
        let wanted: Vec<u8> = "Linux Boot Manager"
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect();
        entries.flatten().any(|entry| {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            // Boot0001-8be4df61-…: the firmware's own boot entries.
            name.len() > 9
                && name.starts_with("Boot")
                && name[4..8].bytes().all(|b| b.is_ascii_hexdigit())
                && name.as_bytes()[8] == b'-'
                && std::fs::read(entry.path())
                    .is_ok_and(|bytes| bytes.windows(wanted.len()).any(|w| w == wanted.as_slice()))
        })
    }

    /// The boot menu entry of a generation, as NixOS names it.
    pub fn entry(generation: u64) -> String {
        format!("nixos-generation-{generation}.conf")
    }

    pub fn entry_exists(&self, generation: u64) -> bool {
        self.run(self.tool("bootctl"), &["--print-boot-path"])
            .map(|out| {
                Path::new(out.trim())
                    .join("loader/entries")
                    .join(Self::entry(generation))
                    .exists()
            })
            .unwrap_or(false)
    }

    /// The entry the next start takes, once, whatever the default is. "" for none.
    pub fn set_oneshot(&self, entry: &str) -> Result<()> {
        self.run(self.tool("bootctl"), &["set-oneshot", entry])?;
        Ok(())
    }

    /// Free bytes where the store is.
    pub fn free_bytes(&self) -> Option<u64> {
        let mut dir = self.paths.at("/nix");
        while !dir.exists() {
            dir = dir.parent()?.to_path_buf();
        }
        let path = std::ffi::CString::new(dir.to_string_lossy().as_bytes()).ok()?;
        // SAFETY: statvfs fills the struct it is given; the path is a valid C string.
        let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
        if unsafe { libc::statvfs(path.as_ptr(), &mut stat) } != 0 {
            return None;
        }
        Some(stat.f_bavail as u64 * stat.f_frsize as u64)
    }

    /// Running from a battery: an unattended download waits for the charger.
    pub fn on_battery(&self) -> bool {
        let Ok(entries) = std::fs::read_dir(self.paths.at("/sys/class/power_supply")) else {
            return false;
        };
        let read = |dir: &Path, name: &str| {
            std::fs::read_to_string(dir.join(name))
                .map(|s| s.trim().to_string())
                .unwrap_or_default()
        };
        let mut discharging = false;
        for entry in entries.flatten() {
            let dir = entry.path();
            match read(&dir, "type").as_str() {
                "Battery" => discharging |= read(&dir, "status") == "Discharging",
                _ if read(&dir, "online") == "1" => return false,
                _ => {}
            }
        }
        discharging
    }

    /// The system finished starting, and the session the person is logged into by
    /// default runs. The programs found, so that two looks can be compared: the same
    /// ones a while later is a session that stays up.
    pub fn healthy(&self, health: Option<&Health>) -> Option<Vec<(String, u32)>> {
        let state = self
            .command(self.tool("systemctl"), &["is-system-running"])
            .stderr(Stdio::null())
            .output()
            .ok()?;
        // "degraded" is a unit that failed, which most computers have; "starting" is not done.
        let state = String::from_utf8_lossy(&state.stdout).trim().to_string();
        if state != "running" && state != "degraded" {
            return None;
        }
        let Some(health) = health else {
            return Some(Vec::new());
        };
        let uid = self.uid(&health.user)?;
        let mut found = Vec::new();
        for name in &health.processes {
            found.push((name.clone(), self.process(uid, name)?));
        }
        Some(found)
    }

    fn uid(&self, user: &str) -> Option<u32> {
        std::fs::read_to_string(self.paths.at("/etc/passwd"))
            .ok()?
            .lines()
            .map(|line| line.split(':').collect::<Vec<_>>())
            .find(|fields| fields.first() == Some(&user))
            .and_then(|fields| fields.get(2)?.parse().ok())
    }

    /// The lowest pid of a process called `name` that belongs to `uid`.
    fn process(&self, uid: u32, name: &str) -> Option<u32> {
        let mut pids: Vec<u32> = std::fs::read_dir(self.paths.at("/proc"))
            .ok()?
            .flatten()
            .filter_map(|entry| {
                let pid: u32 = entry.file_name().to_str()?.parse().ok()?;
                let status = std::fs::read_to_string(entry.path().join("status")).ok()?;
                let field = |key: &str| {
                    status
                        .lines()
                        .find_map(|l| l.strip_prefix(key))
                        .map(|v| v.trim().to_string())
                };
                let owner: u32 = field("Uid:")?.split_whitespace().next()?.parse().ok()?;
                (field("Name:")? == name && owner == uid).then_some(pid)
            })
            .collect();
        pids.sort_unstable();
        pids.first().copied()
    }
}

fn generation_number(link: &Path) -> Option<u64> {
    link.file_name()?
        .to_str()?
        .strip_prefix("system-")?
        .strip_suffix("-link")?
        .parse()
        .ok()
}

fn size(number: &str, unit: &str) -> u64 {
    let factor = match unit {
        "KiB" => 1u64 << 10,
        "MiB" => 1 << 20,
        "GiB" => 1 << 30,
        "TiB" => 1 << 40,
        _ => 1,
    };
    (number.parse::<f64>().unwrap_or(0.0) * factor as f64) as u64
}

/// Reads Nix's "these 3 paths will be fetched (1.20 MiB download, 4.00 MiB unpacked):"
/// and "these 2 derivations will be built:" (and their singular forms).
pub fn parse_plan(text: &str) -> Plan {
    let mut plan = Plan::default();
    for line in text.lines().map(str::trim) {
        let count = || -> u64 {
            let words: Vec<&str> = line.split_whitespace().collect();
            match words.first() {
                Some(&"this") => 1,
                _ => words.get(1).and_then(|n| n.parse().ok()).unwrap_or(0),
            }
        };
        if line.contains("will be built") {
            plan.build += count();
        } else if line.contains("will be fetched") {
            plan.fetch += count();
            if let (Some(open), Some(close)) = (line.find('('), line.rfind(')')) {
                for part in line[open + 1..close].split(',') {
                    let words: Vec<&str> = part.split_whitespace().collect();
                    if let [number, unit, kind] = words[..] {
                        match kind {
                            "download" => plan.download_bytes += size(number, unit),
                            "unpacked" => plan.unpacked_bytes += size(number, unit),
                            _ => {}
                        }
                    }
                }
            }
        }
    }
    plan
}

/// The SlateOS version a source tree is: the workspace version in its Cargo.toml.
pub fn source_version(dir: &Path) -> Result<String> {
    let file = dir.join("Cargo.toml");
    let text =
        std::fs::read_to_string(&file).with_context(|| format!("reading {}", file.display()))?;
    let value: toml::Value = toml::from_str(&text)?;
    value
        .get("workspace")
        .and_then(|w| w.get("package"))
        .and_then(|p| p.get("version"))
        .and_then(|v| v.as_str())
        .map(String::from)
        .ok_or_else(|| anyhow!("{} names no version", file.display()))
}

/// Unpacks a release's source (a gzipped tar with one directory at its top) as `dest`.
pub fn unpack(archive: &Path, dest: &Path) -> Result<()> {
    let staging = dest.with_extension("unpacking");
    let _ = std::fs::remove_dir_all(&staging);
    let _ = std::fs::remove_dir_all(dest);
    std::fs::create_dir_all(&staging)?;
    let file = std::fs::File::open(archive)?;
    let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(file));
    // The archive's own owners and timestamps mean nothing here; names that would
    // leave the directory are refused by the tar crate.
    tar.set_preserve_mtime(false);
    tar.unpack(&staging)
        .context("the downloaded source could not be unpacked")?;
    let mut tops = std::fs::read_dir(&staging)?
        .flatten()
        .map(|e| e.path())
        .collect::<Vec<_>>();
    let top = match (tops.pop(), tops.is_empty()) {
        (Some(top), true) if top.is_dir() => top,
        _ => {
            let _ = std::fs::remove_dir_all(&staging);
            bail!("the downloaded source is not one directory");
        }
    };
    std::fs::rename(&top, dest)?;
    let _ = std::fs::remove_dir_all(&staging);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nix_says_what_a_build_takes() {
        let plan = parse_plan(
            "these 2 derivations will be built:\n  /nix/store/a.drv\n  /nix/store/b.drv\n\
             these 340 paths will be fetched (812.50 MiB download, 2.75 GiB unpacked):\n  /nix/store/x\n",
        );
        assert_eq!(plan.build, 2);
        assert_eq!(plan.fetch, 340);
        assert_eq!(plan.download_bytes, 851_968_000);
        assert_eq!(plan.unpacked_bytes, 2_952_790_016);

        let plan = parse_plan(
            "this derivation will be built:\n  /nix/store/a.drv\n\
             this path will be fetched (131.4 KiB download, 595.3 KiB unpacked):\n  /nix/store/x\n",
        );
        assert_eq!((plan.build, plan.fetch), (1, 1));
        assert_eq!(plan.download_bytes, 134_553);
        assert_eq!(parse_plan(""), Plan::default());
    }

    #[test]
    fn a_generation_is_named_by_its_link() {
        assert_eq!(generation_number(Path::new("system-41-link")), Some(41));
        assert_eq!(
            generation_number(Path::new("/nix/var/nix/profiles/system-7-link")),
            Some(7)
        );
        assert_eq!(generation_number(Path::new("system")), None);
        assert_eq!(Sys::entry(41), "nixos-generation-41.conf");
    }

    fn archive(dir: &Path, entries: &[(&str, &str)]) -> PathBuf {
        let path = dir.join("source.tar.gz");
        let gz = flate2::write::GzEncoder::new(
            std::fs::File::create(&path).unwrap(),
            flate2::Compression::default(),
        );
        let mut tar = tar::Builder::new(gz);
        for (name, body) in entries {
            let mut header = tar::Header::new_gnu();
            header.set_size(body.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            tar.append_data(&mut header, name, body.as_bytes()).unwrap();
        }
        tar.into_inner().unwrap().finish().unwrap();
        path
    }

    #[test]
    fn a_source_unpacks_as_one_directory_and_names_its_version() {
        let dir = tempfile::tempdir().unwrap();
        let tar = archive(
            dir.path(),
            &[
                (
                    "slateos-0.3.1/Cargo.toml",
                    "[workspace]\n[workspace.package]\nversion = \"0.3.1\"\n",
                ),
                ("slateos-0.3.1/flake.nix", "{}"),
            ],
        );
        let dest = dir.path().join("slate.next");
        std::fs::create_dir_all(dest.join("stale")).unwrap();
        unpack(&tar, &dest).unwrap();
        assert!(dest.join("flake.nix").is_file());
        assert!(!dest.join("stale").exists());
        assert_eq!(source_version(&dest).unwrap(), "0.3.1");
        assert!(!dir.path().join("slate.unpacking").exists());

        let two = archive(dir.path(), &[("a/x", "1"), ("b/y", "2")]);
        assert!(unpack(&two, &dir.path().join("other")).is_err());
        assert!(!dir.path().join("other").exists());
    }

    #[test]
    fn a_trial_needs_firmware_that_remembers() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::under(dir.path());
        let sys = Sys::new(&paths);
        assert!(
            !sys.firmware_keeps_variables(),
            "no firmware variables at all"
        );
        let vars = paths.efivars();
        std::fs::create_dir_all(&vars).unwrap();
        let utf16 =
            |text: &str| -> Vec<u8> { text.encode_utf16().flat_map(u16::to_le_bytes).collect() };
        let guid = "8be4df61-93ca-11d2-aa0d-00e098032b8c";
        std::fs::write(vars.join(format!("Boot0000-{guid}")), utf16("UiApp")).unwrap();
        std::fs::write(
            vars.join(format!("BootOrder-{guid}")),
            utf16("Linux Boot Manager"),
        )
        .unwrap();
        assert!(
            !sys.firmware_keeps_variables(),
            "only the firmware's own entries"
        );
        let mut entry = vec![7, 0, 0, 0, 1, 0, 0, 0, 0x62, 0];
        entry.extend(utf16("Linux Boot Manager"));
        entry.extend([0, 0, 4, 1]);
        std::fs::write(vars.join(format!("Boot0003-{guid}")), entry).unwrap();
        assert!(sys.firmware_keeps_variables());
    }

    #[test]
    fn the_charger_decides_about_unattended_downloads() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::under(dir.path());
        let sys = Sys::new(&paths);
        assert!(!sys.on_battery(), "a desktop has no power supplies listed");
        let supply = |name: &str, files: &[(&str, &str)]| {
            let d = paths.at("/sys/class/power_supply").join(name);
            std::fs::create_dir_all(&d).unwrap();
            for (file, value) in files {
                std::fs::write(d.join(file), format!("{value}\n")).unwrap();
            }
        };
        supply("BAT0", &[("type", "Battery"), ("status", "Discharging")]);
        supply("AC", &[("type", "Mains"), ("online", "0")]);
        assert!(sys.on_battery());
        supply("AC", &[("type", "Mains"), ("online", "1")]);
        assert!(!sys.on_battery());
    }

    #[test]
    fn a_session_is_the_persons_programs() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::under(dir.path());
        let sys = Sys::new(&paths);
        std::fs::create_dir_all(paths.at("/etc")).unwrap();
        std::fs::write(
            paths.at("/etc/passwd"),
            "root:x:0:0::/root:/bin/sh\nmira:x:1000:100::/home/mira:/bin/sh\n",
        )
        .unwrap();
        let process = |pid: u32, name: &str, uid: u32| {
            let d = paths.at("/proc").join(pid.to_string());
            std::fs::create_dir_all(&d).unwrap();
            std::fs::write(
                d.join("status"),
                format!("Name:\t{name}\nState:\tS\nUid:\t{uid}\t{uid}\t{uid}\t{uid}\n"),
            )
            .unwrap();
        };
        process(700, "sway", 1000);
        process(650, "sway", 0);
        process(900, "sway", 1000);
        assert_eq!(sys.uid("mira"), Some(1000));
        assert_eq!(sys.uid("nobody-here"), None);
        assert_eq!(sys.process(1000, "sway"), Some(700));
        assert_eq!(sys.process(1000, "slate-desktop"), None);
    }
}
