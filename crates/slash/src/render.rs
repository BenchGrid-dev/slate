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

/// Inline Markdown for the terminal, applied as the answer streams in: `**bold**`,
/// `` `code` `` and `# headings` become ANSI styles and their markers disappear;
/// a fenced block's lines are shown as code without the fences. A marker split
/// across two deltas is held back until the next one. Everything else (lists,
/// single `*`, links) passes through as written.
#[derive(Default)]
pub struct Markdown {
    bold: bool,
    code: bool,
    fence: bool,
    heading: bool,
    line_start: bool,
    started: bool,
    /// Characters that may be the start of a marker, waiting for the next delta.
    held: String,
    /// Skipping the rest of a fence line (its language tag).
    skip_line: bool,
}

impl Markdown {
    pub fn new() -> Self {
        Self::default()
    }

    fn style(&self) -> String {
        if !color_enabled() {
            return String::new();
        }
        let mut s = String::from("\x1b[0m");
        if self.bold || self.heading {
            s.push_str("\x1b[1m");
        }
        if self.code || self.fence {
            s.push_str("\x1b[36m");
        }
        s
    }

    /// Render the next piece of text.
    pub fn push(&mut self, text: &str) -> String {
        if !self.started {
            self.started = true;
            self.line_start = true;
        }
        let input: Vec<char> = std::mem::take(&mut self.held)
            .chars()
            .chain(text.chars())
            .collect();
        let mut out = String::new();
        let mut i = 0;
        while i < input.len() {
            let c = input[i];
            let rest = input.len() - i;
            if self.skip_line {
                if c == '\n' {
                    self.skip_line = false;
                    self.line_start = true;
                }
                i += 1;
                continue;
            }
            // A run of '`' or '*' at the end may continue in the next delta.
            if (c == '`' || c == '*' || (c == '#' && self.line_start))
                && input[i..].iter().all(|&x| x == c)
                && rest < 3
            {
                self.held = input[i..].iter().collect();
                break;
            }
            if self.line_start && c == '`' && input[i..].starts_with(&['`', '`', '`']) {
                self.fence = !self.fence;
                self.skip_line = true;
                out.push_str(&self.style());
                i += 3;
                continue;
            }
            if self.fence {
                out.push(c);
                self.line_start = c == '\n';
                i += 1;
                continue;
            }
            if self.line_start && c == '#' {
                let hashes = input[i..].iter().take_while(|&&x| x == '#').count();
                match input.get(i + hashes) {
                    Some(' ') => {
                        self.heading = true;
                        out.push_str(&self.style());
                        i += hashes + 1;
                        self.line_start = false;
                        continue;
                    }
                    None => {
                        self.held = input[i..].iter().collect();
                        break;
                    }
                    _ => {}
                }
            }
            if c == '`' {
                self.code = !self.code;
                out.push_str(&self.style());
                i += 1;
                self.line_start = false;
                continue;
            }
            if !self.code && c == '*' && input.get(i + 1) == Some(&'*') {
                self.bold = !self.bold;
                out.push_str(&self.style());
                i += 2;
                self.line_start = false;
                continue;
            }
            if c == '\n' {
                if self.heading || self.code {
                    // Styles never run past the end of a line.
                    self.heading = false;
                    self.code = false;
                    out.push_str(&self.style());
                }
                self.line_start = true;
            } else if !(self.line_start && c == ' ') {
                self.line_start = false;
            }
            out.push(c);
            i += 1;
        }
        out
    }

    /// The end of the answer: flush what was held back and reset the style.
    pub fn finish(&mut self) -> String {
        let held = std::mem::take(&mut self.held);
        let had_style = self.bold || self.code || self.fence || self.heading;
        *self = Self::default();
        let mut out = held;
        if had_style && color_enabled() {
            out.push_str("\x1b[0m");
        }
        out
    }
}

#[cfg(test)]
mod markdown_tests {
    use super::Markdown;

    fn plain(pieces: &[&str]) -> String {
        std::env::set_var("NO_COLOR", "1");
        let mut m = Markdown::new();
        let mut out: String = pieces.iter().map(|p| m.push(p)).collect();
        out.push_str(&m.finish());
        out
    }

    #[test]
    fn markers_disappear() {
        assert_eq!(
            plain(&["1. **`src/slate/target`** — 4.4G — `cargo build`\n"]),
            "1. src/slate/target — 4.4G — cargo build\n"
        );
        assert_eq!(plain(&["## Disk usage\nok\n"]), "Disk usage\nok\n");
    }

    #[test]
    fn markers_split_across_deltas() {
        assert_eq!(plain(&["a *", "*b*", "* c `x", "` d"]), "a b c x d");
        assert_eq!(plain(&["#", "# Title\n"]), "Title\n");
        assert_eq!(plain(&["x\n``", "`sh\nls -la\n``", "`\ny"]), "x\nls -la\ny");
    }

    #[test]
    fn other_text_passes_through() {
        assert_eq!(
            plain(&["* item\n- two\n3 * 4 = 12\n"]),
            "* item\n- two\n3 * 4 = 12\n"
        );
        assert_eq!(plain(&["issue #42 and C#\n"]), "issue #42 and C#\n");
        assert_eq!(plain(&["trailing *"]), "trailing *");
    }
}
