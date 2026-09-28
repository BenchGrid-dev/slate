//! slate-desktop: background computer use for Linux.
//!
//! `slate-desktop serve` is an MCP server (stdio) that holds one Wayland
//! connection with the agent's own transient seat. The other subcommands are
//! for testing the same operations from a shell.

mod daemon;
mod keymap;
mod mcp;
mod sway;
mod wayland;

use anyhow::{bail, Result};
use serde_json::json;
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
  slate-desktop daemon                    own the agent seat for the whole session (start it from the compositor)
  slate-desktop serve                     MCP server on stdio (uses the daemon when it runs)
  slate-desktop windows                   list windows (JSON)
  slate-desktop shot [IDENT] OUT.png      capture a window (or the output) to PNG
  slate-desktop click X Y [left|right|middle]
  slate-desktop move X Y
  slate-desktop type TEXT
  slate-desktop key COMBO                 e.g. ctrl+l, Return, alt+Tab
      input commands take --seat agent|user (user = borrow the human's seat, for GTK4 apps)
  slate-desktop launch CMD [ARGS...]      start a program on this display
  slate-desktop seats                     which window each seat has focused
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
        "daemon" => daemon::run(),
        "serve" => mcp::serve(),
        "probe" => mcp::probe(),
        "windows" => {
            let r = mcp::cli_call("desktop_windows", json!({}))?;
            println!("{}", r["content"][0]["text"].as_str().unwrap_or(""));
            Ok(())
        }
        "shot" => {
            let (ident, out) = match args.len() {
                2 => (None, args[1].clone()),
                3 => (Some(args[1].clone()), args[2].clone()),
                _ => usage(),
            };
            let mut a = json!({});
            if let Some(i) = ident {
                a["window"] = json!(i);
            }
            let r = mcp::cli_call("desktop_screenshot", a)?;
            let img = r["content"]
                .as_array()
                .and_then(|c| c.iter().find(|x| x["type"] == "image"))
                .and_then(|x| x["data"].as_str())
                .ok_or_else(|| anyhow::anyhow!("no image returned"))?;
            use base64::Engine;
            std::fs::write(&out, base64::engine::general_purpose::STANDARD.decode(img)?)?;
            println!("{out}: {}", r["content"][0]["text"].as_str().unwrap_or(""));
            Ok(())
        }
        "click" | "move" => {
            if args.len() < 3 {
                usage();
            }
            let x: f64 = args[1].parse()?;
            let y: f64 = args[2].parse()?;
            let seat_s = if seat == Seat::User { "user" } else { "agent" };
            let r = if cmd == "click" {
                mcp::cli_call(
                    "desktop_click",
                    json!({"x": x, "y": y, "button": args.get(3).cloned().unwrap_or_else(|| "left".into()), "seat": seat_s, "verify": false}),
                )?
            } else {
                mcp::cli_call("desktop_move", json!({"x": x, "y": y, "seat": seat_s}))?
            };
            println!("{}", r["content"][0]["text"].as_str().unwrap_or(""));
            Ok(())
        }
        "type" => {
            let text = args.get(1).cloned().unwrap_or_default();
            let seat_s = if seat == Seat::User { "user" } else { "agent" };
            let r = mcp::cli_call(
                "desktop_type",
                json!({"text": text, "seat": seat_s, "verify": false}),
            )?;
            println!("{}", r["content"][0]["text"].as_str().unwrap_or(""));
            Ok(())
        }
        "key" => {
            let combo = args.get(1).cloned().unwrap_or_default();
            let seat_s = if seat == Seat::User { "user" } else { "agent" };
            let r = mcp::cli_call(
                "desktop_key",
                json!({"combo": combo, "seat": seat_s, "verify": false}),
            )?;
            println!("{}", r["content"][0]["text"].as_str().unwrap_or(""));
            Ok(())
        }
        "seats" => {
            let r = mcp::cli_call("desktop_seats", json!({}))?;
            println!("{}", r["content"][0]["text"].as_str().unwrap_or(""));
            Ok(())
        }
        "close" | "focus" => {
            let target = args.get(1).cloned().unwrap_or_default();
            let r = mcp::cli_call(
                if cmd == "close" {
                    "desktop_close"
                } else {
                    "desktop_focus"
                },
                json!({"window": target}),
            )?;
            println!("{}", r["content"][0]["text"].as_str().unwrap_or(""));
            Ok(())
        }
        "launch" => {
            if args.len() < 2 {
                usage();
            }
            let r = mcp::cli_call(
                "desktop_launch",
                json!({"command": args[1], "args": args[2..]}),
            )?;
            println!("{}", r["content"][0]["text"].as_str().unwrap_or(""));
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
