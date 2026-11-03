//! What the person sees, sent along with a request from the desktop prompt.
//!
//! When the prompt opens, it tells slash which window the person was looking at
//! (`look`). While they type, slash asks slate-desktop for that window and the windows
//! they can see (`glance`), then for that window's text (`read`). A request then starts
//! with a `<screen>` block: always the windows, whose ids work in the desktop tools as
//! written, so the agent need not list them first; and, when the request is about what
//! is on screen ("summarize this page", "翻译这段"), the window's text and the person's
//! text selection, so it can answer without reading the window itself. Each saves the
//! agent a model turn.

use serde_json::Value;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

/// Text longer than this is cut (about 2k tokens).
const TEXT_MAX: usize = 8000;
const SELECTION_MAX: usize = 4000;

#[derive(Default)]
struct Slot {
    generation: u64,
    opened: Option<Instant>,
    glance: Option<Value>,
    glance_done: bool,
    text: Option<String>,
    text_done: bool,
}

#[derive(Clone, Default)]
pub struct Screen {
    slot: Arc<(Mutex<Slot>, Condvar)>,
    /// The selection sent last: one still there at the next request is not news.
    last_selection: Arc<Mutex<Option<String>>>,
}

impl Screen {
    /// The prompt opened; `con` is the sway container the person was looking at.
    pub fn look(&self, desktop: &Path, con: Option<i64>) {
        let generation = {
            let mut s = self.lock();
            s.generation += 1;
            let g = s.generation;
            *s = Slot {
                generation: g,
                opened: Some(Instant::now()),
                ..Slot::default()
            };
            g
        };
        let me = self.clone();
        let desktop: PathBuf = desktop.to_path_buf();
        std::thread::spawn(move || {
            let mut args = vec!["glance".to_string()];
            if let Some(c) = con {
                args.push(c.to_string());
            }
            let glance = run(&desktop, &args, Duration::from_secs(5))
                .and_then(|out| serde_json::from_str::<Value>(&out).ok());
            let full_id = glance
                .as_ref()
                .and_then(|g| g.get("looking_at_id"))
                .and_then(Value::as_str)
                .map(str::to_string);
            me.update(generation, |s| {
                s.glance = glance;
                s.glance_done = true;
            });
            let text = full_id
                .and_then(|id| run(&desktop, &["read".to_string(), id], Duration::from_secs(8)));
            me.update(generation, |s| {
                s.text = text;
                s.text_done = true;
            });
        });
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Slot> {
        self.slot.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn update(&self, generation: u64, f: impl FnOnce(&mut Slot)) {
        let mut s = self.lock();
        if s.generation == generation {
            f(&mut s);
            self.slot.1.notify_all();
        }
    }

    /// The `<screen>` block for `request`; empty when the prompt did not look first.
    pub fn block(&self, request: &str) -> String {
        let about_screen = refers_to_screen(request);
        let mut s = self.lock();
        match s.opened {
            Some(t) if t.elapsed() < Duration::from_secs(900) => {}
            _ => return String::new(),
        }
        // The windows arrive in milliseconds; a long page's text can take seconds, worth
        // waiting for when the request is about it (a model turn takes longer).
        let wait = if about_screen { 3000 } else { 400 };
        let deadline = Instant::now() + Duration::from_millis(wait);
        while !(s.glance_done && (!about_screen || s.text_done)) {
            let now = Instant::now();
            if now >= deadline {
                break;
            }
            s = self
                .slot
                .1
                .wait_timeout(s, deadline - now)
                .map(|(g, _)| g)
                .unwrap_or_else(|e| e.into_inner().0);
        }
        let glance = s.glance.clone();
        let text = if about_screen { s.text.clone() } else { None };
        drop(s);
        let selection = if about_screen { self.selection() } else { None };
        format_block(glance.as_ref(), text.as_deref(), selection.as_deref())
    }

    fn selection(&self) -> Option<String> {
        let sel = run(
            Path::new("wl-paste"),
            &["--primary".into(), "--no-newline".into()],
            Duration::from_secs(1),
        )?;
        let sel = sel.trim();
        if sel.is_empty() {
            return None;
        }
        let mut last = self
            .last_selection
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if last.as_deref() == Some(sel) {
            return None;
        }
        *last = Some(sel.to_string());
        Some(cut(sel, SELECTION_MAX))
    }
}

/// Run a program for its output, giving up after `limit`.
fn run(program: &Path, args: &[String], limit: Duration) -> Option<String> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    // Read while it runs: a long output would fill the pipe and stall it.
    let mut stdout = child.stdout.take()?;
    let reader = std::thread::spawn(move || {
        let mut out = String::new();
        let _ = stdout.read_to_string(&mut out);
        out
    });
    let deadline = Instant::now() + limit;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let out = reader.join().ok()?;
                return status.success().then_some(out);
            }
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
}

fn cut(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut c: String = s.chars().take(max).collect();
        c.push_str("\n[… cut]");
        c
    }
}

/// Does the request seem to be about what is on screen? Then the window's text and the
/// selection go along; otherwise only the window list (screen content stays local).
pub fn refers_to_screen(request: &str) -> bool {
    const WORDS: &[&str] = &[
        "this",
        "these",
        "page",
        "selected",
        "selection",
        "highlighted",
        "document",
        "article",
        "email",
        "error",
        "above",
        "summarize",
        "summarise",
        "summary",
        "tldr",
        "translate",
        "explain",
        "reply",
        "rewrite",
        "proofread",
        "paragraph",
    ];
    // "window" and "screen" alone are how window requests talk ("fill the screen").
    const PHRASES: &[&str] = &[
        "on my screen",
        "on the screen",
        "on screen",
        "in this window",
        "这",
        "页面",
        "网页",
        "屏幕上",
        "窗口里",
        "选中",
        "上面",
        "总结",
        "概括",
        "翻译",
        "解释",
        "回复",
        "改写",
        "润色",
        "看看",
        "读一下",
        "文章",
        "邮件",
        "报错",
        "错误",
        "这段",
        "原文",
    ];
    let lower = request.to_lowercase();
    PHRASES.iter().any(|p| lower.contains(p))
        || lower
            .split(|c: char| !c.is_alphanumeric())
            .any(|w| WORDS.contains(&w))
}

fn format_block(glance: Option<&Value>, text: Option<&str>, selection: Option<&str>) -> String {
    let Some(g) = glance else {
        return String::new();
    };
    if g.get("locked").and_then(Value::as_bool).unwrap_or(false) {
        return String::new();
    }
    let looking = g
        .pointer("/looking_at/id")
        .and_then(Value::as_str)
        .unwrap_or("");
    let mut out = String::from(
        "Windows on the person's screen and on yours (ids work in the desktop tools as written):\n",
    );
    let windows = g.get("windows").and_then(Value::as_array);
    for w in windows.into_iter().flatten() {
        let s = |k: &str| w.get(k).and_then(Value::as_str).unwrap_or("");
        out.push_str(&format!("- {} {} {:?}", s("id"), s("app_id"), s("title")));
        if s("location") == "background" {
            out.push_str(" (your background screen)");
        }
        if !looking.is_empty() && s("id") == looking {
            out.push_str(" ← they were looking at this one");
        }
        out.push('\n');
    }
    if windows.map(|w| w.is_empty()).unwrap_or(true) {
        out.push_str("(none)\n");
    }
    if let Some(t) = text {
        // `slate-desktop read` starts with "<id> (<title>):".
        let body = t.split_once('\n').map(|(_, b)| b).unwrap_or(t).trim();
        if !body.is_empty() {
            out.push_str(
                "Text of the window they were looking at (its accessibility tree; may be cut):\n",
            );
            out.push_str(&cut(body, TEXT_MAX));
            out.push('\n');
        }
    }
    if let Some(sel) = selection {
        out.push_str("Text they have selected (in some window):\n");
        out.push_str(sel);
        out.push('\n');
    }
    format!("<screen>\n{out}</screen>\n\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn requests_about_the_screen_are_recognised() {
        assert!(refers_to_screen("Summarize the page I have open"));
        assert!(refers_to_screen("what does this error mean?"));
        assert!(refers_to_screen("翻译一下这段"));
        assert!(refers_to_screen("帮我回复这封邮件"));
        assert!(!refers_to_screen("set the volume to 30%"));
        assert!(!refers_to_screen("open WeChat"));
        assert!(!refers_to_screen("把音量调到 30"));
        assert!(!refers_to_screen(
            "Make the Firefox window fill the screen."
        ));
        assert!(!refers_to_screen("把窗口放大到全屏"));
        assert!(refers_to_screen("what's on my screen?"));
        assert!(refers_to_screen("屏幕上那个按钮是干什么的"));
        // Whole words only: "thistle" is not "this".
        assert!(!refers_to_screen("buy thistle seeds"));
    }

    #[test]
    fn the_block_lists_windows_and_marks_the_one_looked_at() {
        let g = json!({
            "looking_at": {"id": "46ea3bb6"},
            "windows": [
                {"id": "46ea3bb6", "app_id": "firefox", "title": "Slate", "location": "screen"},
                {"id": "94663fbf", "app_id": "libreoffice-writer", "title": "Untitled 1", "location": "background"}
            ]
        });
        let b = format_block(
            Some(&g),
            Some("46ea3bb6… (Slate):\nA little less human."),
            None,
        );
        assert!(b.starts_with("<screen>\n"));
        assert!(b.contains("- 46ea3bb6 firefox \"Slate\" ← they were looking at this one"));
        assert!(b.contains("\"Untitled 1\" (your background screen)"));
        assert!(b.contains("A little less human."));
        assert!(!b.contains("(Slate):"));
        assert_eq!(format_block(Some(&json!({"locked": true})), None, None), "");
        assert_eq!(format_block(None, None, None), "");
    }

    #[test]
    fn nothing_is_sent_when_the_prompt_did_not_look() {
        assert_eq!(Screen::default().block("summarize this"), "");
    }
}
