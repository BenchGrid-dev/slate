//! The screen lock, and what the agent may do in windows while it is on.
//!
//! The person chooses in Settings, AI. The choice is `agentWhileLocked` in the
//! system's settings.json, which only root writes (`slateos-settings`, behind the
//! person's password), so the agent cannot choose for itself; a configuration that
//! sets `services.slate.desktop.agentWhileLocked` wins (`/etc/slate/system-settings.json`).
//!
//! - `pause` (the default, and without either file): look at windows, not use them.
//! - `elements`: operate them through accessibility (element actions) and manage them
//!   (launch, close, move); not the keyboard and pointer, which sway gives to the lock
//!   screen.
//! - `continue`: the keyboard and pointer too, on a compositor that exempts the
//!   agent's seat from the lock (SlateOS's SwayFX, `seat <name> lock_exempt yes`):
//!   its input reaches the windows behind the lock screen, never the lock screen.

use serde_json::Value;
use std::sync::atomic::{AtomicU8, Ordering};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Policy {
    Pause,
    Elements,
    Continue,
}

impl Policy {
    fn parse(s: &str) -> Policy {
        match s {
            "elements" => Policy::Elements,
            "continue" => Policy::Continue,
            _ => Policy::Pause,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Policy::Pause => "pause",
            Policy::Elements => "elements",
            Policy::Continue => "continue",
        }
    }
}

const SYSTEM: &str = "/etc/slate/system-settings.json";

fn read_json(path: &str) -> Value {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or(Value::Null)
}

/// The person's choice; `pause` unless the system says otherwise.
pub fn policy() -> Policy {
    if let Some(s) = read_json(SYSTEM)
        .pointer("/agent/whileLocked")
        .and_then(Value::as_str)
    {
        return Policy::parse(s);
    }
    let file = if std::path::Path::new("/etc/slateos").is_dir() {
        "/etc/slateos/settings.json"
    } else {
        "/etc/nixos/settings.json"
    };
    read_json(file)
        .get("agentWhileLocked")
        .and_then(Value::as_str)
        .map(Policy::parse)
        .unwrap_or(Policy::Pause)
}

/// Whether a screen locker of this user is running (the compositor does not say).
pub fn locked() -> bool {
    use std::os::unix::fs::MetadataExt;
    let uid = std::fs::metadata("/proc/self").map(|m| m.uid()).ok();
    let Ok(procs) = std::fs::read_dir("/proc") else {
        return false;
    };
    procs.flatten().any(|p| {
        let path = p.path();
        let comm = std::fs::read_to_string(path.join("comm")).unwrap_or_default();
        matches!(comm.trim(), "swaylock" | ".swaylock-wrapp")
            && std::fs::metadata(&path).map(|m| m.uid()).ok() == uid
    })
}

// What the compositor was last told about the agent seat: 0 nothing yet, 1 exempt,
// 2 not exempt (or it cannot exempt a seat).
static EXEMPT: AtomicU8 = AtomicU8::new(0);
static ASKED: AtomicU8 = AtomicU8::new(0);

/// Whether the agent seat's input currently reaches windows behind the lock.
pub fn exempt() -> bool {
    EXEMPT.load(Ordering::Relaxed) == 1
}

/// Tell sway whether the agent's seat (`seat`, its name) is exempt from the lock, when
/// the person's choice changed. Called by the daemon at start and every few seconds.
pub fn apply(seat: &str) {
    let want = policy() == Policy::Continue;
    let asked = if want { 1 } else { 2 };
    let before = ASKED.swap(asked, Ordering::Relaxed);
    if before == asked {
        return;
    }
    // A new seat is not exempt: nothing to tell sway until the person lets it be (every
    // seat command makes sway reassign input devices).
    if before == 0 && !want {
        EXEMPT.store(2, Ordering::Relaxed);
        return;
    }
    let cmd = format!(
        "seat {seat} lock_exempt {}",
        if want { "yes" } else { "no" }
    );
    match crate::sway::run_command(&cmd) {
        Ok(()) => EXEMPT.store(asked, Ordering::Relaxed),
        Err(e) => {
            EXEMPT.store(2, Ordering::Relaxed);
            if want {
                slate_proto::log!(
                    "slate-desktop daemon: the compositor cannot exempt the agent seat from the lock ({e:#}); acting as \"elements\""
                );
            }
        }
    }
}

/// What a desktop tool does to windows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    /// Reads, or touches nothing on screen.
    Look,
    /// Accessibility actions.
    Element,
    /// Through the compositor or a new process: launch, close, move, resize.
    Manage,
    /// The seat's keyboard or pointer (focusing a window is a click).
    Input,
}

fn kind(tool: &str) -> Kind {
    match tool {
        "desktop_windows"
        | "desktop_elements"
        | "desktop_read"
        | "desktop_screenshot"
        | "desktop_seats"
        | "desktop_status"
        | "desktop_takeover_cancel" => Kind::Look,
        "desktop_element_click" | "desktop_element_set_text" => Kind::Element,
        "desktop_click" | "desktop_type" | "desktop_key" | "desktop_scroll" | "desktop_move"
        | "desktop_focus" => Kind::Input,
        _ => Kind::Manage,
    }
}

/// Whether the agent seat's keyboard and pointer reach windows now.
pub fn input_reaches_windows(locked: bool, policy: Policy, exempt: bool) -> bool {
    !locked || (policy == Policy::Continue && exempt)
}

/// Why `tool` cannot run while the screen is locked, if it cannot.
pub fn refusal(
    tool: &str,
    user_seat: bool,
    locked: bool,
    policy: Policy,
    exempt: bool,
) -> Option<String> {
    let kind = kind(tool);
    if !locked || kind == Kind::Look {
        return None;
    }
    const LOOK: &str = "desktop_windows, desktop_elements, desktop_read and desktop_screenshot of a window still work";
    if user_seat {
        return Some(format!(
            "the screen is locked: the person's own keyboard and pointer go to the lock screen. {LOOK}."
        ));
    }
    match policy {
        Policy::Pause => Some(format!(
            "the screen is locked, and the person chose (Settings, AI) that the agent pauses then: it may look at windows but not use them until they unlock. {LOOK}; work outside the desktop (commands, files) is not affected. The choice is theirs: tell them what waits for them instead of working around it."
        )),
        _ if kind == Kind::Input && !exempt => {
            let why = if policy == Policy::Continue {
                "this compositor cannot exempt the agent's seat from the lock, so "
            } else {
                ""
            };
            Some(format!(
                "the screen is locked: {why}keyboard and pointer input cannot reach windows until the person unlocks it. Element actions (desktop_element_click, desktop_element_set_text) and launching, closing and moving windows still work, and {LOOK}."
            ))
        }
        _ => None,
    }
}

/// Where the daemon notes when the agent last used a desktop tool (unix seconds):
/// the idle lock waits for it when the person let the agent keep the screen awake.
fn activity_path() -> std::path::PathBuf {
    slate_proto::desktop_socket_path().with_file_name("agent-active")
}

static LAST_ACTIVITY: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub fn note_activity() {
    let now = now_secs();
    LAST_ACTIVITY.store(now, Ordering::Relaxed);
    let _ = std::fs::write(activity_path(), format!("{now}\n"));
}

/// Whether the agent used a desktop tool in the last `secs` seconds (in this process).
pub fn active_within(secs: u64) -> bool {
    let last = LAST_ACTIVITY.load(Ordering::Relaxed);
    last != 0 && now_secs().saturating_sub(last) <= secs
}

/// For `desktop_status`.
pub fn status() -> Value {
    serde_json::json!({"screen_locked": locked(), "while_locked": policy().name(), "seat_exempt": exempt()})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_is_refused_while_unlocked() {
        for p in [Policy::Pause, Policy::Elements, Policy::Continue] {
            assert!(refusal("desktop_type", false, false, p, false).is_none());
            assert!(refusal("desktop_type", true, false, p, false).is_none());
        }
    }

    #[test]
    fn pause_only_looks() {
        let r = |t| refusal(t, false, true, Policy::Pause, false);
        assert!(r("desktop_read").is_none());
        assert!(r("desktop_screenshot").is_none());
        assert!(r("desktop_element_click").is_some());
        assert!(r("desktop_launch").is_some());
        assert!(r("desktop_close").is_some());
        assert!(r("desktop_type").is_some());
    }

    #[test]
    fn elements_allows_element_actions_and_window_management() {
        let r = |t| refusal(t, false, true, Policy::Elements, false);
        assert!(r("desktop_element_click").is_none());
        assert!(r("desktop_element_set_text").is_none());
        assert!(r("desktop_launch").is_none());
        assert!(r("desktop_hide").is_none());
        assert!(r("desktop_click").is_some());
        assert!(r("desktop_focus").is_some());
    }

    #[test]
    fn continue_needs_the_exemption_for_input() {
        assert!(refusal("desktop_type", false, true, Policy::Continue, true).is_none());
        let r = refusal("desktop_type", false, true, Policy::Continue, false).unwrap();
        assert!(r.contains("cannot exempt"));
        assert!(refusal(
            "desktop_element_click",
            false,
            true,
            Policy::Continue,
            false
        )
        .is_none());
    }

    #[test]
    fn the_users_seat_never_acts_on_a_locked_screen() {
        for p in [Policy::Pause, Policy::Elements, Policy::Continue] {
            assert!(refusal("desktop_type", true, true, p, true).is_some());
            assert!(refusal("desktop_element_click", true, true, p, true).is_some());
            assert!(refusal("desktop_read", true, true, p, true).is_none());
        }
    }

    #[test]
    fn input_reaches_windows_only_when_exempt() {
        assert!(input_reaches_windows(false, Policy::Pause, false));
        assert!(!input_reaches_windows(true, Policy::Elements, false));
        assert!(!input_reaches_windows(true, Policy::Continue, false));
        assert!(input_reaches_windows(true, Policy::Continue, true));
    }
}
