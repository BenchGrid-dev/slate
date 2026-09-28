//! slate-desktop: background computer use for Linux.
//!
//! `slate-desktop serve` is an MCP server (stdio) that holds one Wayland
//! connection with the agent's own transient seat. The other subcommands are
//! for testing the same operations from a shell.

mod keymap;
mod mcp;
mod sway;
mod wayland;

use anyhow::{bail, Result};
use wayland::Seat;

/// Strip `--seat X` from args and return the seat.
fn take_seat(args: &mut Vec<String>) -> Seat {
    if let Some(i) = args.iter().position(|a| a == "--seat") {
        let v = args.get(i + 1).cloned().unwrap_or_default();
        args.drain(i..=(i + 1).min(args.len() - 1));
        return Seat::parse(&v);
    }
    Seat::Agent
}

fn usage() -> ! {
    eprintln!(
        "slate-desktop {}

usage:
  slate-desktop serve                     MCP server on stdio
  slate-desktop windows                   list windows (JSON)
  slate-desktop shot [IDENT] OUT.png      capture a window (or the output) to PNG
  slate-desktop click X Y [left|right|middle]
  slate-desktop move X Y
  slate-desktop type TEXT
  slate-desktop key COMBO                 e.g. ctrl+l, Return, alt+Tab
      input commands take --seat agent|user (user = borrow the human's seat, for GTK4 apps)
  slate-desktop launch CMD [ARGS...]      start a program on this display
  slate-desktop close WINDOW              close a window (id, app_id or title)
  slate-desktop focus WINDOW
  slate-desktop probe                     report compositor capabilities",
        slate_proto::VERSION
    );
    std::process::exit(2)
}

fn main() -> Result<()> {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let seat = take_seat(&mut args);
    let cmd = args.first().map(String::as_str).unwrap_or("");
    match cmd {
        "serve" => mcp::serve(),
        "probe" => mcp::probe(),
        "windows" => {
            let mut d = wayland::Desktop::connect()?;
            let wins = mcp::windows(&mut d)?;
            println!("{}", serde_json::to_string_pretty(&wins)?);
            Ok(())
        }
        "shot" => {
            let (ident, out) = match args.len() {
                2 => (None, args[1].clone()),
                3 => (Some(args[1].clone()), args[2].clone()),
                _ => usage(),
            };
            let mut d = wayland::Desktop::connect()?;
            let crop = match ident.as_deref() {
                Some(id) => mcp::windows(&mut d)?
                    .into_iter()
                    .find(|w| w.id == id)
                    .and_then(|w| match (w.width, w.height) {
                        (Some(cw), Some(ch)) if cw > 0 && ch > 0 => Some((cw as u32, ch as u32)),
                        _ => None,
                    }),
                None => None,
            };
            let (png, w, h) = d.capture(ident.as_deref(), crop)?;
            std::fs::write(&out, png)?;
            println!("{out}: {w}x{h}");
            Ok(())
        }
        "click" | "move" => {
            if args.len() < 3 {
                usage();
            }
            let x: f64 = args[1].parse()?;
            let y: f64 = args[2].parse()?;
            let mut d = wayland::Desktop::connect()?;
            d.settle()?;
            if cmd == "click" {
                d.click(
                    seat,
                    x,
                    y,
                    args.get(3).map(String::as_str).unwrap_or("left"),
                    1,
                )?;
            } else {
                d.pointer_move(seat, x, y)?;
            }
            // Keep the seat alive briefly so the compositor delivers the events.
            std::thread::sleep(std::time::Duration::from_millis(300));
            Ok(())
        }
        "type" => {
            let text = args.get(1).cloned().unwrap_or_default();
            let mut d = wayland::Desktop::connect()?;
            d.settle()?;
            d.type_text(seat, &text)?;
            std::thread::sleep(std::time::Duration::from_millis(300));
            Ok(())
        }
        "key" => {
            let combo = args.get(1).cloned().unwrap_or_default();
            let mut d = wayland::Desktop::connect()?;
            d.settle()?;
            d.key(seat, &combo)?;
            std::thread::sleep(std::time::Duration::from_millis(300));
            Ok(())
        }
        "close" | "focus" => {
            let target = args.get(1).cloned().unwrap_or_default();
            let mut d = wayland::Desktop::connect()?;
            let wins = mcp::windows(&mut d)?;
            let w = wins
                .iter()
                .find(|w| w.id == target || w.app_id == target || w.title.contains(&target))
                .ok_or_else(|| anyhow::anyhow!("no window matches {target:?}"))?;
            let con = w
                .con_id
                .ok_or_else(|| anyhow::anyhow!("no compositor handle"))?;
            sway::command_for_con(con, if cmd == "close" { "kill" } else { "focus" })?;
            println!("{cmd} {}", w.id);
            Ok(())
        }
        "launch" => {
            if args.len() < 2 {
                usage();
            }
            let child = mcp::launch(&args[1], &args[2..])?;
            println!("pid {child}");
            Ok(())
        }
        "" | "-h" | "--help" => usage(),
        "--version" | "-V" => {
            println!("slate-desktop {}", slate_proto::VERSION);
            Ok(())
        }
        other => bail!("unknown command {other:?}"),
    }
}
