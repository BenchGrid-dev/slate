//! Minimal ANSI rendering. Respects NO_COLOR.

use std::io::{self, Write};

fn color_enabled() -> bool {
    std::env::var_os("NO_COLOR").is_none()
}

fn wrap(code: &str, s: &str) -> String {
    if color_enabled() {
        format!("\x1b[{code}m{s}\x1b[0m")
    } else {
        s.to_string()
    }
}

pub fn dim(s: &str) -> String {
    wrap("2", s)
}
pub fn bold(s: &str) -> String {
    wrap("1", s)
}
pub fn cyan(s: &str) -> String {
    wrap("36", s)
}
pub fn yellow(s: &str) -> String {
    wrap("33", s)
}
pub fn red(s: &str) -> String {
    wrap("31", s)
}
pub fn green(s: &str) -> String {
    wrap("32", s)
}

/// One-line summary of a tool call: `▸ Bash: git status`.
pub fn tool_line(name: &str, detail: &str) -> String {
    let detail = truncate(detail.lines().next().unwrap_or(""), 100);
    format!(
        "  {} {}{}",
        dim("▸"),
        cyan(name),
        dim(&format!(": {detail}"))
    )
}

pub fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let cut: String = s.chars().take(max.saturating_sub(1)).collect();
        format!("{cut}…")
    }
}

pub fn flush() {
    let _ = io::stdout().flush();
}
