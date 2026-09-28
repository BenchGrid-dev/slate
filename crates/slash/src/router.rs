//! Input routing. See docs/decisions/0003-slash-prefix-grammar.md.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Input {
    /// Nothing but whitespace.
    Empty,
    /// Bare text: goes to the agent backend.
    Agent(String),
    /// `/name args`: a control command. Unknown names are passed through to the backend.
    Control { name: String, args: String },
    /// `!cmd`: run in the user's real shell.
    Shell(String),
}

pub fn route(line: &str) -> Input {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return Input::Empty;
    }
    if let Some(rest) = trimmed.strip_prefix('!') {
        let cmd = rest.trim();
        return if cmd.is_empty() {
            Input::Empty
        } else {
            Input::Shell(cmd.to_string())
        };
    }
    if let Some(rest) = trimmed.strip_prefix('/') {
        // "//" escapes a leading slash so you can still send "/foo" to the agent.
        if let Some(escaped) = rest.strip_prefix('/') {
            return Input::Agent(format!("/{}", escaped.trim_start()));
        }
        let mut parts = rest.splitn(2, char::is_whitespace);
        let name = parts.next().unwrap_or("").to_string();
        let args = parts.next().unwrap_or("").trim().to_string();
        if name.is_empty() {
            return Input::Empty;
        }
        return Input::Control { name, args };
    }
    // Plain "exit" and friends leave slash itself; nobody means them for the agent.
    if matches!(
        trimmed.to_ascii_lowercase().as_str(),
        "exit" | "quit" | "bye" | "退出" | "再见" | "logout"
    ) {
        return Input::Control {
            name: "quit".into(),
            args: String::new(),
        };
    }
    Input::Agent(trimmed.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bare_text_goes_to_agent() {
        assert_eq!(
            route("find the pdf Alice sent"),
            Input::Agent("find the pdf Alice sent".into())
        );
    }

    #[test]
    fn bang_goes_to_shell() {
        assert_eq!(route("!git status"), Input::Shell("git status".into()));
        assert_eq!(route("!  ls -la "), Input::Shell("ls -la".into()));
        assert_eq!(route("!"), Input::Empty);
    }

    #[test]
    fn slash_is_control() {
        assert_eq!(
            route("/agent codex"),
            Input::Control {
                name: "agent".into(),
                args: "codex".into()
            }
        );
        assert_eq!(
            route("/undo"),
            Input::Control {
                name: "undo".into(),
                args: String::new()
            }
        );
        assert_eq!(route("/"), Input::Empty);
    }

    #[test]
    fn double_slash_escapes_to_agent() {
        assert_eq!(
            route("//usr/bin/what is this"),
            Input::Agent("/usr/bin/what is this".into())
        );
    }

    #[test]
    fn exit_words_quit_slash() {
        for w in ["exit", "Quit", "退出"] {
            assert_eq!(
                route(w),
                Input::Control {
                    name: "quit".into(),
                    args: String::new()
                }
            );
        }
        assert!(matches!(route("exit the app for me"), Input::Agent(_)));
    }

    #[test]
    fn whitespace_is_empty() {
        assert_eq!(route("   "), Input::Empty);
    }
}
