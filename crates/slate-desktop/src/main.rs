//! slate-desktop: background computer use for Linux.
//! Placeholder main while the Wayland layer is being built; see probe.rs.

#[allow(dead_code)]
mod probe;

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("probe") => probe::run(),
        _ => {
            println!("slate-desktop {} (pre-alpha)", slate_proto::VERSION);
            println!(
                "usage: slate-desktop probe   # connect to the compositor and report capabilities"
            );
            Ok(())
        }
    }
}
