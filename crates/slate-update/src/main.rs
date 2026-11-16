//! slateos-update: SlateOS's software update, from a terminal. Settings, Software Update
//! runs the same commands.

use anyhow::{bail, Context, Result};
use slate_update::config::{Automatic, Config, Paths};
use slate_update::manifest;
use slate_update::state::{now, Outcome, Phase, State};
use slate_update::system::Sys;
use slate_update::update::{Asked, Updater};
use std::path::Path;

fn usage() -> ! {
    eprintln!(
        "slateos-update {}

usage:
  slateos-update [status] [--json]  the version that runs, and what is known about a newer one
  slateos-update check [--json]     ask the release server now
  slateos-update install            download and build the newest release; the next restart runs it
  slateos-update cancel             take back an update that waits for its restart
  slateos-update back               make the version before the last update the default again

Installing, cancelling and going back need root. A new version is started once, on
trial: if it does not come up, the next start is the old version again by itself.
--config FILE reads another description of this system than /etc/slate/update.json.

for the system:
  slateos-update boot               at every start: finish or undo the last update
  slateos-update auto               the daily timer: check, and download if Settings says so
  slateos-update pinned-nixpkgs [DIR]  the nixpkgs a Slate source is locked to (for slateos-rebuild)
  slateos-update activate SYSTEM    second half of install, run from the release being installed
  slateos-update waiting            succeeds, with a sentence, when an update waits for its restart

for making a release:
  slateos-update keygen FILE        write a new signing key to FILE and print its public half
  slateos-update public-key FILE    print the public half of the key in FILE
  slateos-update sign KEYFILE FILE  print the signature line of FILE (for FILE.sig)
  slateos-update verify FILE KEY…   succeeds when FILE.sig holds a signature of FILE by one of
                                    the public keys",
        env!("CARGO_PKG_VERSION")
    );
    std::process::exit(2)
}

fn need_root(paths: &Paths, what: &str) -> Result<()> {
    // SAFETY: geteuid has no preconditions.
    if paths.is_real() && unsafe { libc::geteuid() } != 0 {
        bail!("{what} needs root: sudo slateos-update {what}");
    }
    Ok(())
}

fn ago(when: u64) -> String {
    let seconds = now().saturating_sub(when);
    match seconds {
        0..=119 => "just now".into(),
        120..=7199 => format!("{} minutes ago", seconds / 60),
        7200..=172_799 => format!("{} hours ago", seconds / 3600),
        _ => format!("{} days ago", seconds / 86_400),
    }
}

fn status(updater: &Updater, json: bool) -> Result<()> {
    let state = State::load(&updater.paths);
    let automatic = match updater.config.automatic(&updater.paths) {
        Automatic::Off => "off",
        Automatic::Check => "check",
        Automatic::Download => "download",
    };
    if json {
        let mut value = serde_json::to_value(&state)?;
        value["current"] = updater.config.version.clone().into();
        value["automatic"] = automatic.into();
        println!("{}", serde_json::to_string_pretty(&value)?);
        return Ok(());
    }
    println!("SlateOS {}", updater.config.version);
    match (state.phase, &state.target) {
        (Phase::Restart, Some(target)) if target.back => {
            println!(
                "SlateOS {} starts again at the next restart.",
                target.version
            )
        }
        (Phase::Restart, Some(target)) => println!(
            "SlateOS {} is installed and starts at the next restart{}.",
            target.version,
            if target.trial {
                " (once, on trial: if it does not come up, the restart after is this version again)"
            } else {
                ""
            }
        ),
        (Phase::Downloading | Phase::Building, Some(target)) => {
            let progress = state.progress.clone().unwrap_or_default();
            let count = if progress.total > 0 {
                format!(" ({} of {})", progress.done, progress.total)
            } else {
                String::new()
            };
            println!(
                "Installing SlateOS {}: {}{count}",
                target.version, progress.text
            );
        }
        _ => {
            match &state.available {
                Some(offer) => {
                    println!("SlateOS {} is available.", offer.manifest.version);
                    if !offer.manifest.summary.is_empty() {
                        println!("  {}", offer.manifest.summary);
                    }
                    if !offer.manifest.notes.is_empty() {
                        println!("  Release notes: {}", offer.manifest.notes);
                    }
                    println!("  Install it: sudo slateos-update install");
                }
                None => match state.checked {
                    Some(when) => println!("Up to date (checked {}).", ago(when)),
                    None => println!("Not checked yet: slateos-update check"),
                },
            }
            if let Some(error) = &state.check_error {
                println!("The last check got no answer: {error}");
            }
        }
    }
    if let Some(last) = &state.last {
        let what = match last.outcome {
            Outcome::Installed => format!("SlateOS {} was installed", last.version),
            Outcome::Failed => format!("SlateOS {} could not be installed", last.version),
            Outcome::RolledBack => format!("SlateOS {} did not start", last.version),
            Outcome::Cancelled => {
                format!("SlateOS {} was taken back before its restart", last.version)
            }
            Outcome::WentBack => format!("Went back to SlateOS {}", last.version),
        };
        println!("{what} {}.", ago(last.when));
        if !last.message.is_empty() && last.outcome != Outcome::Installed {
            println!("  {}", last.message.replace('\n', "\n  "));
        }
    }
    if let Some(previous) = &state.previous {
        println!(
            "The version before, {}, is still here: sudo slateos-update back",
            previous.version
        );
    }
    println!("Automatic: {automatic} (Settings, Software Update)");
    Ok(())
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut json = false;
    let mut config_file = None;
    let mut words: Vec<&str> = Vec::new();
    let mut rest = args.iter().map(String::as_str);
    while let Some(arg) = rest.next() {
        match arg {
            "--json" => json = true,
            "--config" => config_file = Some(rest.next().unwrap_or_else(|| usage())),
            "--help" => usage(),
            _ if arg.starts_with("--") => usage(),
            _ => words.push(arg),
        }
    }
    let command = words.first().copied().unwrap_or("status");
    let paths = Paths::system();
    if paths.is_real() {
        // Under sudo and in a unit alike: the system's own tools first.
        let path = std::env::var("PATH").unwrap_or_default();
        std::env::set_var(
            "PATH",
            format!("/run/current-system/sw/bin:/run/wrappers/bin:{path}"),
        );
    }

    match command {
        "keygen" => {
            let file = words.get(1).unwrap_or_else(|| usage());
            if Path::new(file).exists() {
                bail!("{file} exists; a key is not overwritten");
            }
            let (secret, public) = manifest::generate_key()?;
            {
                use std::io::Write;
                use std::os::unix::fs::OpenOptionsExt;
                let mut out = std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .mode(0o600)
                    .open(file)?;
                writeln!(out, "{secret}")?;
            }
            println!("{public}");
            return Ok(());
        }
        "public-key" => {
            let file = words.get(1).unwrap_or_else(|| usage());
            println!("{}", manifest::public_key(&std::fs::read_to_string(file)?)?);
            return Ok(());
        }
        "sign" => {
            let (key, file) = match words[1..] {
                [key, file] => (key, file),
                _ => usage(),
            };
            let secret = std::fs::read_to_string(key).with_context(|| format!("reading {key}"))?;
            let message = std::fs::read(file).with_context(|| format!("reading {file}"))?;
            println!("{}", manifest::sign(&secret, &message)?);
            return Ok(());
        }
        "verify" => {
            let (file, keys) = match words[1..] {
                [file, ref keys @ ..] if !keys.is_empty() => (file, keys),
                _ => usage(),
            };
            let message = std::fs::read(file).with_context(|| format!("reading {file}"))?;
            let signatures = std::fs::read_to_string(format!("{file}.sig"))
                .with_context(|| format!("reading {file}.sig"))?;
            let keys: Vec<String> = keys.iter().map(|k| k.to_string()).collect();
            manifest::verify(&message, &signatures, &keys)?;
            println!("{file} is signed with a key named here");
            return Ok(());
        }
        "pinned-nixpkgs" => {
            let dir = words.get(1).copied().unwrap_or("/etc/slateos/slate");
            println!("{}", Sys::new(&paths).pinned_nixpkgs(Path::new(dir))?);
            return Ok(());
        }
        "-h" | "help" => usage(),
        _ => {}
    }

    // --config: another description of the system than the module's (a test, a fork
    // trying its own releases); the state and everything else stay where they are.
    let updater = match config_file {
        Some(file) => Updater::with(paths, Config::load(Path::new(file))?),
        None => Updater::new(paths)?,
    };
    match command {
        "status" => status(&updater, json)?,
        "check" => {
            // SAFETY: geteuid has no preconditions.
            let root = !updater.paths.is_real() || unsafe { libc::geteuid() } == 0;
            let offer = updater.check(root)?;
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "current": updater.config.version,
                        "available": offer,
                    }))?
                );
            } else {
                match offer {
                    Some(offer) => {
                        println!(
                            "SlateOS {} is available (this is {}).",
                            offer.manifest.version, updater.config.version
                        );
                        if !offer.manifest.summary.is_empty() {
                            println!("  {}", offer.manifest.summary);
                        }
                        println!("  Install it: sudo slateos-update install");
                    }
                    None => println!("SlateOS {} is the newest.", updater.config.version),
                }
            }
        }
        "install" => {
            need_root(&updater.paths, "install")?;
            match updater.install(Asked::Person, &mut |line| println!("{line}"))? {
                Some(target) => println!(
                    "SlateOS {} is installed. Restart to use it{}.",
                    target.version,
                    if target.trial {
                        "; if it does not come up, the restart after that is this version again"
                    } else {
                        ""
                    }
                ),
                None => println!("SlateOS {} is the newest.", updater.config.version),
            }
        }
        "auto" => {
            need_root(&updater.paths, "auto")?;
            match updater.config.automatic(&updater.paths) {
                Automatic::Off => {}
                // No answer (no network) is not a failure of the timer; Settings shows it.
                Automatic::Check => {
                    if let Err(e) = updater.check(true) {
                        eprintln!("no answer: {e:#}");
                    }
                }
                Automatic::Download => {
                    match updater.install(Asked::Timer, &mut |line| println!("{line}")) {
                        Ok(Some(target)) => {
                            println!("SlateOS {} waits for a restart", target.version)
                        }
                        Ok(None) => {}
                        Err(e) => eprintln!("not installed: {e:#}"),
                    }
                }
            }
        }
        "cancel" => {
            need_root(&updater.paths, "cancel")?;
            let version = updater.cancel()?;
            println!(
                "SlateOS {version} was taken back; this stays {}.",
                updater.config.version
            );
        }
        "back" => {
            need_root(&updater.paths, "back")?;
            let target = updater.back()?;
            println!(
                "SlateOS {} starts again at the next restart.",
                target.version
            );
        }
        "boot" => {
            need_root(&updater.paths, "boot")?;
            match updater.boot()? {
                Some(Outcome::Installed) => println!("SlateOS {} runs", updater.config.version),
                Some(outcome) => println!("{outcome:?}"),
                None => {}
            }
        }
        // Exit status 0 and a sentence when an update waits for its restart.
        "waiting" => {
            let state = State::load(&updater.paths);
            match (state.phase, state.target) {
                (Phase::Restart, Some(target)) => println!(
                    "SlateOS {} is installed and waits for a restart.",
                    target.version
                ),
                _ => std::process::exit(1),
            }
        }
        "activate" => {
            need_root(&updater.paths, "activate")?;
            let system = words.get(1).unwrap_or_else(|| usage());
            updater.activate_recorded(Path::new(system))?;
        }
        _ => usage(),
    }
    Ok(())
}
