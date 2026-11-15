//! SlateOS's software update.
//!
//! A release is the Slate source at a tag. Its flake lock names the nixpkgs revision it
//! was tested on, so a release fixes the whole system, not only Slate's own programs.
//! Installed systems keep that source next to their configuration (`/etc/slateos/slate`)
//! and are built from it; an update replaces it and builds again.
//!
//! - [`manifest`]: the signed file that names the newest release and its source.
//! - [`update`]: checking, installing, the trial start, going back.
//! - [`system`]: Nix, the system profile, the boot loader.
//! - [`state`]: what is kept between runs, for Settings to show.
//! - [`config`]: where things are and what the running system was set up with.
//!
//! docs/software-update.md describes it for people; docs/decisions/0009 says why it is
//! built this way.

pub mod config;
pub mod manifest;
pub mod state;
pub mod system;
pub mod update;
