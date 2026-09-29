//! slate-desktop: background computer use for Linux.
//!
//! `slate-desktop serve` is an MCP server (stdio) that holds one Wayland
//! connection with the agent's own transient seat. The other subcommands are
//! for testing the same operations from a shell.

mod a11y;
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

/// `--window W` anywhere in the arguments: the window an input command targets.
fn take_window(args: &mut Vec<String>) -> Option<String> {
    let i = args.iter().position(|a| a == "--window")?;
    let v = args.get(i + 1).cloned();
    args.drain(i..=(i + 1).min(args.len() - 1));
    v
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
      and --window W (focus that window first; click/move coordinates become window-relative)
  slate-desktop elements WINDOW [QUERY]   interactive elements from the accessibility tree
  slate-desktop read WINDOW               readable text from the accessibility tree
  slate-desktop element-click ID [ACTION] activate an element (action, or pointer fallback)
  slate-desktop element-set-text ID TEXT  set an editable element's text
  slate-desktop launch [--here] CMD [ARGS...]  start a program (background screen unless --here)
  slate-desktop show [WINDOW]             bring background windows to the user's screen
  slate-desktop hide [WINDOW]             move windows to the background screen
  slate-desktop seats                     which window each seat has focused
  slate-desktop status                    is the agent controlling the user's seat right now
  slate-desktop takeover-cancel           stop borrowing the user's seat (bound to Esc in sway's controlling mode)
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
    let window = take_window(&mut args);
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
                    json!({"x": x, "y": y, "button": args.get(3).cloned().unwrap_or_else(|| "left".into()), "seat": seat_s, "verify": false, "window": window}),
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
                json!({"text": text, "seat": seat_s, "verify": false, "window": window}),
            )?;
            println!("{}", r["content"][0]["text"].as_str().unwrap_or(""));
            Ok(())
        }
        "key" => {
            let combo = args.get(1).cloned().unwrap_or_default();
            let seat_s = if seat == Seat::User { "user" } else { "agent" };
            let r = mcp::cli_call(
                "desktop_key",
                json!({"combo": combo, "seat": seat_s, "verify": false, "window": window}),
            )?;
            println!("{}", r["content"][0]["text"].as_str().unwrap_or(""));
            Ok(())
        }
        "elements" => {
            let window = args.get(1).cloned().unwrap_or_default();
            let query = args.get(2).cloned();
            let r = mcp::cli_call(
                "desktop_elements",
                json!({"window": window, "query": query}),
            )?;
            println!("{}", r["content"][0]["text"].as_str().unwrap_or(""));
            Ok(())
        }
        "read" => {
            let window = args.get(1).cloned().unwrap_or_default();
            let r = mcp::cli_call("desktop_read", json!({"window": window}))?;
            println!("{}", r["content"][0]["text"].as_str().unwrap_or(""));
            Ok(())
        }
        "element-click" => {
            let id = args.get(1).cloned().unwrap_or_default();
            let seat_s = if seat == Seat::User { "user" } else { "agent" };
            let r = mcp::cli_call(
                "desktop_element_click",
                json!({"id": id, "action": args.get(2).cloned(), "seat": seat_s, "verify": false}),
            )?;
            println!("{}", r["content"][0]["text"].as_str().unwrap_or(""));
            Ok(())
        }
        "element-set-text" => {
            let id = args.get(1).cloned().unwrap_or_default();
            let text = args.get(2).cloned().unwrap_or_default();
            let seat_s = if seat == Seat::User { "user" } else { "agent" };
            let r = mcp::cli_call(
                "desktop_element_set_text",
                json!({"id": id, "text": text, "seat": seat_s, "verify": false}),
            )?;
            println!("{}", r["content"][0]["text"].as_str().unwrap_or(""));
            Ok(())
        }
        "show" | "hide" => {
            let r = mcp::cli_call(
                if cmd == "show" {
                    "desktop_show"
                } else {
                    "desktop_hide"
                },
                json!({"window": args.get(1).cloned()}),
            )?;
            println!("{}", r["content"][0]["text"].as_str().unwrap_or(""));
            Ok(())
        }
        "takeover-cancel" => {
            let r = mcp::cli_call("desktop_takeover_cancel", json!({}))?;
            println!("{}", r["content"][0]["text"].as_str().unwrap_or(""));
            Ok(())
        }
        "status" => {
            let r = mcp::cli_call("desktop_status", json!({}))?;
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
            let mut rest: Vec<String> = args[1..].to_vec();
            let here = rest.first().map(|a| a == "--here").unwrap_or(false);
            if here {
                rest.remove(0);
            }
            if rest.is_empty() {
                usage();
            }
            let r = mcp::cli_call(
                "desktop_launch",
                json!({"command": rest[0], "args": rest[1..], "where": if here { "here" } else { "background" }}),
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
