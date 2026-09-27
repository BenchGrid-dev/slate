//! slated: the Slate daemon.
//!
//! Approval broker, tool-call policy, audit log, snapshots and undo.
//! Runs one instance per user, listening on `slate_proto::socket_path()`.

mod audit;
mod memory;
mod policy;
mod server;
mod snapshot;
mod tasks;

use anyhow::Result;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--version" || a == "-V") {
        println!("slated {}", slate_proto::VERSION);
        return Ok(());
    }
    let state_dir = slate_proto::state_dir();
    std::fs::create_dir_all(&state_dir)?;
    let audit = audit::Audit::open(&state_dir)?;
    let tasks = tasks::TaskStore::open(&state_dir)?;
    let memories = memory::MemoryStore::open(&state_dir)?;

    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"));
    let snap_root = std::env::var_os("SLATE_SNAPSHOT_ROOT")
        .map(PathBuf::from)
        .unwrap_or(home);
    let snapshots =
        match snapshot::Snapshotter::detect(snap_root.clone(), state_dir.join("snapshots")) {
            Ok(s) => {
                eprintln!("slated: snapshots enabled for {}", snap_root.display());
                Some(s)
            }
            Err(e) => {
                eprintln!("slated: snapshots disabled: {e:#}");
                None
            }
        };

    let state = Arc::new(Mutex::new(server::State::new(
        audit, tasks, memories, snapshots,
    )));
    server::serve(slate_proto::socket_path(), state)
}
