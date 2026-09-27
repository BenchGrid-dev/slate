//! Tier classification for tool calls. See docs/architecture.md, "Approval tiers".
//!
//! Unknown things default to Confirm. This is deliberately conservative and
//! will grow a policy file; for now the rules live here so they are testable.

use serde::Deserialize;
use serde_json::Value;
use slate_proto::Tier;
use std::sync::OnceLock;

/// User overrides from `~/.config/slate/policy.toml`. Loaded once at startup.
#[derive(Debug, Default, Clone, Deserialize)]
#[serde(default)]
pub struct PolicyFile {
    /// Exact tool name -> tier. Wins over everything else.
    pub tools: std::collections::BTreeMap<String, Tier>,
    pub shell: ShellPolicy,
    pub paths: PathPolicy,
}

#[derive(Debug, Default, Clone, Deserialize)]
#[serde(default)]
pub struct ShellPolicy {
    /// Extra command names treated as read-only.
    pub read_only: Vec<String>,
    /// Extra substrings that force Confirm.
    pub confirm_patterns: Vec<String>,
    /// Substrings that are trusted: a command containing one is Reversible even if
    /// it matches a confirm pattern. Use sparingly, e.g. "git push origin feature/".
    pub trusted_patterns: Vec<String>,
}

#[derive(Debug, Default, Clone, Deserialize)]
#[serde(default)]
pub struct PathPolicy {
    /// Extra path substrings whose writes need Confirm.
    pub sensitive: Vec<String>,
}

static POLICY: OnceLock<PolicyFile> = OnceLock::new();

pub fn policy() -> &'static PolicyFile {
    POLICY.get_or_init(|| {
        let Some(path) = dirs::config_dir().map(|d| d.join("slate").join("policy.toml")) else {
            return PolicyFile::default();
        };
        match std::fs::read_to_string(&path) {
            Ok(text) => match toml::from_str(&text) {
                Ok(p) => {
                    eprintln!("slated: loaded policy from {}", path.display());
                    p
                }
                Err(e) => {
                    eprintln!("slated: ignoring {}: {e}", path.display());
                    PolicyFile::default()
                }
            },
            Err(_) => PolicyFile::default(),
        }
    })
}

/// For tests and embedding: install a policy instead of reading the file.
#[cfg(test)]
pub fn set_policy_for_tests(p: PolicyFile) {
    let _ = POLICY.set(p);
}

pub struct Verdict {
    pub tier: Tier,
    pub reason: String,
}

fn v(tier: Tier, reason: impl Into<String>) -> Verdict {
    Verdict {
        tier,
        reason: reason.into(),
    }
}

pub fn classify(tool_name: &str, input: &Value) -> Verdict {
    if let Some(t) = policy().tools.get(tool_name) {
        return v(*t, "policy.toml [tools]");
    }
    match tool_name {
        "Read"
        | "Glob"
        | "Grep"
        | "LS"
        | "TodoRead"
        | "TodoWrite"
        | "AskUserQuestion"
        | "WebSearch"
        | "WebFetch"
        | "Task"
        | "Agent"
        | "ListMcpResourcesTool"
        | "ReadMcpResourceTool"
        | "NotebookRead"
        | "BashOutput"
        | "KillShell"
        | "ToolSearch"
        | "Skill"
        | "SlashCommand"
        | "EnterPlanMode"
        | "ExitPlanMode"
        | "TaskOutput"
        | "TaskStop"
        | "Monitor"
        | "ScheduleWakeup" => v(Tier::Observe, "agent-internal or read-only tool"),
        "Edit" | "Write" | "MultiEdit" | "NotebookEdit" => {
            let path = input.get("file_path").and_then(Value::as_str).unwrap_or("");
            if is_sensitive_path(path) {
                v(Tier::Confirm, "writes a sensitive path")
            } else {
                v(Tier::Reversible, "file edit (snapshotted)")
            }
        }
        "Bash" => {
            let cmd = input.get("command").and_then(Value::as_str).unwrap_or("");
            classify_shell(cmd)
        }
        name if name.starts_with("mcp__slate__") => v(Tier::Observe, "slate's own tool"),
        "mcp__desktop__desktop_windows"
        | "mcp__desktop__desktop_screenshot"
        | "mcp__desktop__desktop_move" => v(Tier::Observe, "looks at the desktop"),
        "mcp__desktop__desktop_click"
        | "mcp__desktop__desktop_scroll"
        | "mcp__desktop__desktop_key"
        | "mcp__desktop__desktop_type"
            if input.get("seat").and_then(Value::as_str) == Some("user") =>
        {
            v(Tier::Confirm, "takes over your mouse and keyboard briefly")
        }
        "mcp__desktop__desktop_click"
        | "mcp__desktop__desktop_scroll"
        | "mcp__desktop__desktop_key" => {
            v(Tier::Reversible, "drives the desktop on the agent seat")
        }
        "mcp__desktop__desktop_type" => {
            let t = input.get("text").and_then(Value::as_str).unwrap_or("");
            if t.len() > 2000 {
                v(Tier::Confirm, "types a very large text")
            } else {
                v(Tier::Reversible, "types on the agent seat")
            }
        }
        "mcp__desktop__desktop_launch" => {
            let c = input.get("command").and_then(Value::as_str).unwrap_or("");
            if DANGEROUS_PATTERNS.iter().any(|p| c.contains(p.trim())) {
                v(Tier::Confirm, "launches a sensitive program")
            } else {
                v(Tier::Reversible, "launches a desktop program")
            }
        }
        name if name.starts_with("mcp__") => v(Tier::Confirm, "unknown MCP tool"),
        _ => v(Tier::Confirm, "unknown tool"),
    }
}

const READ_ONLY_CMDS: &[&str] = &[
    "ls",
    "cat",
    "head",
    "tail",
    "less",
    "more",
    "wc",
    "grep",
    "rg",
    "egrep",
    "fgrep",
    "find",
    "fd",
    "fdfind",
    "which",
    "whereis",
    "type",
    "file",
    "stat",
    "du",
    "df",
    "pwd",
    "echo",
    "printf",
    "true",
    "false",
    "test",
    "date",
    "cal",
    "uptime",
    "whoami",
    "id",
    "hostname",
    "uname",
    "env",
    "printenv",
    "ps",
    "top",
    "htop",
    "pgrep",
    "lsof",
    "free",
    "nproc",
    "lscpu",
    "lsblk",
    "mount",
    "tree",
    "diff",
    "cmp",
    "comm",
    "sort",
    "uniq",
    "cut",
    "tr",
    "awk",
    "sed",
    "jq",
    "yq",
    "xargs",
    "basename",
    "dirname",
    "realpath",
    "readlink",
    "md5sum",
    "sha256sum",
    "shasum",
    "base64",
    "strings",
    "hexdump",
    "xxd",
    "od",
    "man",
    "help",
    "tldr",
    "ip",
    "ss",
    "netstat",
    "ping",
    "dig",
    "nslookup",
    "host",
    "cargo",
    "rustc",
    "python",
    "python3",
    "node",
    "go",
    "make",
    "ninja",
    "cmake",
    "gcc",
    "clang",
    "npm",
    "pnpm",
    "yarn",
    "bun",
    "pip",
    "pip3",
    "uv",
    "systemctl",
    "journalctl",
    "wpctl",
    "pactl",
    "brightnessctl",
    "xdg-open",
    "open",
    "sleep",
    "seq",
    "yes",
    "column",
    "paste",
    "join",
    "nl",
    "tac",
    "rev",
    "fold",
    "fmt",
    "expand",
    "unexpand",
    "btrfs",
    "git",
];

/// Subcommands of tools that are read-only even though the tool itself can write.
const READ_ONLY_SUBCMDS: &[(&str, &[&str])] = &[
    (
        "git",
        &[
            "status",
            "log",
            "diff",
            "show",
            "branch",
            "remote",
            "rev-parse",
            "ls-files",
            "blame",
            "describe",
            "tag",
            "stash list",
            "config --get",
            "grep",
            "shortlog",
            "cat-file",
            "ls-remote",
        ],
    ),
    (
        "cargo",
        &[
            "build",
            "check",
            "test",
            "clippy",
            "fmt --check",
            "doc",
            "tree",
            "metadata",
            "search",
            "run",
            "bench",
            "--version",
        ],
    ),
    (
        "npm",
        &[
            "ls",
            "list",
            "view",
            "info",
            "outdated",
            "test",
            "run",
            "--version",
        ],
    ),
    ("pnpm", &["ls", "list", "test", "run", "--version"]),
    ("yarn", &["list", "test", "run", "--version"]),
    ("pip", &["list", "show", "freeze", "--version"]),
    ("pip3", &["list", "show", "freeze", "--version"]),
    ("uv", &["pip list", "tree", "--version"]),
    (
        "systemctl",
        &[
            "status",
            "list-units",
            "list-unit-files",
            "is-active",
            "is-enabled",
            "show",
            "cat",
            "--user status",
            "--user list-units",
        ],
    ),
    ("journalctl", &[""]),
    (
        "btrfs",
        &[
            "subvolume list",
            "subvolume show",
            "filesystem",
            "device stats",
            "qgroup show",
        ],
    ),
    ("wpctl", &["status", "get-volume", "inspect"]),
    ("pactl", &["list", "info", "get-"]),
    (
        "ip",
        &[
            "addr",
            "link show",
            "route show",
            "-4",
            "-6",
            "a",
            "r",
            "-br",
        ],
    ),
    ("python", &["--version", "-c print", "-m json.tool"]),
    ("python3", &["--version", "-c print", "-m json.tool"]),
    ("node", &["--version", "-e", "-p"]),
    ("go", &["build", "test", "vet", "version", "env", "list"]),
    ("make", &["-n", "--dry-run"]),
];

/// Anything matching these is Confirm regardless of context.
const DANGEROUS_PATTERNS: &[&str] = &[
    "rm -rf /",
    "rm -rf ~",
    "rm -rf $HOME",
    "rm -rf *",
    "rm -r /",
    "rm -fr /",
    "mkfs",
    "dd if=",
    "dd of=",
    "> /dev/sd",
    "shred",
    "wipefs",
    "fdisk",
    "parted",
    "sgdisk",
    ":(){",
    "chmod -R 777",
    "chown -R",
    "git push",
    "git push --force",
    "git reset --hard",
    "git clean -f",
    "git branch -D",
    "git checkout --",
    "git restore",
    "curl -X POST",
    "curl -X PUT",
    "curl -X DELETE",
    "curl -d",
    "curl --data",
    "curl -F",
    "wget --post",
    "http POST",
    "ssh ",
    "scp ",
    "rsync ",
    "sftp ",
    "mail ",
    "sendmail",
    "mutt ",
    "msmtp",
    "aws ",
    "gcloud ",
    "az ",
    "kubectl delete",
    "kubectl apply",
    "terraform apply",
    "terraform destroy",
    "docker rm",
    "docker rmi",
    "docker system prune",
    "npm publish",
    "cargo publish",
    "pip upload",
    "twine upload",
    "gh pr create",
    "gh release",
    "gh repo delete",
    "git remote set-url",
    "passwd",
    "useradd",
    "userdel",
    "usermod",
    "visudo",
    "crontab",
    "systemctl disable",
    "systemctl mask",
    "systemctl stop",
    "systemctl restart",
    "systemctl enable",
    "nixos-rebuild",
    "reboot",
    "shutdown",
    "poweroff",
    "halt",
    "kill -9 -1",
    "killall",
    "pkill",
    "iptables",
    "nft ",
    "ufw ",
    "firewall-cmd",
    "openssl req",
    "gpg --",
    "ssh-keygen",
    "keychain",
    "security ",
    "1password",
    "op ",
    "vault ",
    "history -c",
    "export AWS_SECRET",
    "export GITHUB_TOKEN",
    "export ANTHROPIC_API_KEY",
    "export OPENAI_API_KEY",
    ".ssh/",
    ".gnupg/",
    ".aws/",
    ".config/gh/",
    "credentials",
    "id_rsa",
    "id_ed25519",
    "sudo ",
    "doas ",
    "pkexec ",
    "su ",
];

fn is_sensitive_path(path: &str) -> bool {
    let p = path.to_ascii_lowercase();
    if policy()
        .paths
        .sensitive
        .iter()
        .any(|s| p.contains(&s.to_ascii_lowercase()))
    {
        return true;
    }
    [
        "/.ssh/",
        "/.gnupg/",
        "/.aws/",
        "/.config/gh/",
        "credentials",
        "/etc/",
        "/.claude/",
        "/.codex/",
        "id_rsa",
        "id_ed25519",
        "authorized_keys",
        "/.password",
        "secret",
    ]
    .iter()
    .any(|s| p.contains(s))
        || p.starts_with("/boot")
        || p.starts_with("/usr")
        || p.starts_with("/nix/store")
}

pub fn classify_shell(cmd: &str) -> Verdict {
    let trimmed = cmd.trim();
    if trimmed.is_empty() {
        return v(Tier::Observe, "empty command");
    }
    let lower = trimmed.to_ascii_lowercase();
    let pol = &policy().shell;
    let trusted = pol
        .trusted_patterns
        .iter()
        .any(|t| lower.contains(&t.to_ascii_lowercase()));
    if !trusted {
        for pat in pol
            .confirm_patterns
            .iter()
            .map(String::as_str)
            .chain(DANGEROUS_PATTERNS.iter().copied())
        {
            let pat_l = pat.to_ascii_lowercase();
            if lower.contains(&pat_l) {
                return v(Tier::Confirm, format!("matches {:?}", pat.trim()));
            }
        }
    }
    // Writes via redirection or in-place edits are reversible, not observe.
    let writes = lower.contains('>')
        || lower.contains("tee ")
        || lower.contains("sed -i")
        || lower.contains("perl -i");
    let segments = split_segments(trimmed);
    let mut all_read_only = !segments.is_empty();
    for seg in &segments {
        if !segment_is_read_only(seg) {
            all_read_only = false;
            break;
        }
    }
    if all_read_only && !writes {
        v(Tier::Observe, "read-only command")
    } else {
        v(Tier::Reversible, "shell command (snapshotted)")
    }
}

/// Split on `|`, `&&`, `||`, `;` and newlines, ignoring quoting subtleties.
fn split_segments(cmd: &str) -> Vec<String> {
    let mut out = vec![];
    let mut cur = String::new();
    let mut chars = cmd.chars().peekable();
    let mut in_single = false;
    let mut in_double = false;
    while let Some(c) = chars.next() {
        match c {
            '\'' if !in_double => {
                in_single = !in_single;
                cur.push(c);
            }
            '"' if !in_single => {
                in_double = !in_double;
                cur.push(c);
            }
            '|' | ';' | '\n' if !in_single && !in_double => {
                if c == '|' && chars.peek() == Some(&'|') {
                    chars.next();
                }
                out.push(std::mem::take(&mut cur));
            }
            '&' if !in_single && !in_double && chars.peek() == Some(&'&') => {
                chars.next();
                out.push(std::mem::take(&mut cur));
            }
            _ => cur.push(c),
        }
    }
    out.push(cur);
    out.into_iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

fn segment_is_read_only(seg: &str) -> bool {
    // Strip leading env assignments and common wrappers.
    let mut words: Vec<&str> = seg.split_whitespace().collect();
    while let Some(w) = words.first() {
        let is_assignment = w.contains('=') && !w.starts_with('-');
        let is_wrapper = matches!(*w, "env" | "time" | "nice" | "command" | "builtin" | "exec");
        if is_assignment || is_wrapper {
            words.remove(0);
        } else {
            break;
        }
    }
    let Some(cmd) = words.first() else {
        return true;
    };
    let base = cmd.rsplit('/').next().unwrap_or(cmd);
    if policy().shell.read_only.iter().any(|c| c == base) {
        return true;
    }
    if !READ_ONLY_CMDS.contains(&base) {
        return false;
    }
    if let Some((_, subs)) = READ_ONLY_SUBCMDS.iter().find(|(c, _)| *c == base) {
        let rest = words[1..].join(" ");
        return subs.iter().any(|s| rest.starts_with(s));
    }
    // Interpreters and build tools with arbitrary args are not read-only.
    if matches!(
        base,
        "python"
            | "python3"
            | "node"
            | "xargs"
            | "awk"
            | "sed"
            | "make"
            | "cargo"
            | "npm"
            | "pnpm"
            | "yarn"
            | "bun"
            | "pip"
            | "pip3"
            | "uv"
            | "go"
            | "gcc"
            | "clang"
            | "cmake"
            | "ninja"
            | "systemctl"
            | "btrfs"
            | "git"
            | "wpctl"
            | "pactl"
            | "brightnessctl"
            | "ip"
            | "xdg-open"
            | "open"
    ) {
        return false;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn tier(cmd: &str) -> Tier {
        classify_shell(cmd).tier
    }

    fn install_test_policy() {
        let p: PolicyFile = toml::from_str(
            r#"
[tools]
"mcp__foo__safe" = "observe"

[shell]
read_only = ["mytool"]
confirm_patterns = ["deploy "]
trusted_patterns = ["git push origin feature/"]

[paths]
sensitive = ["/srv/vault"]
"#,
        )
        .unwrap();
        set_policy_for_tests(p);
    }

    #[test]
    fn policy_file_overrides() {
        install_test_policy();
        assert_eq!(classify("mcp__foo__safe", &json!({})).tier, Tier::Observe);
        assert_eq!(tier("mytool --list"), Tier::Observe);
        assert_eq!(tier("./deploy prod"), Tier::Confirm);
        assert_eq!(tier("git push origin feature/x"), Tier::Reversible);
        assert_eq!(tier("git push origin main"), Tier::Confirm);
        assert_eq!(
            classify("Write", &json!({"file_path": "/srv/vault/key"})).tier,
            Tier::Confirm
        );
    }

    #[test]
    fn read_only_shell() {
        install_test_policy();
        assert_eq!(tier("ls -la"), Tier::Observe);
        assert_eq!(tier("git status && git diff"), Tier::Observe);
        assert_eq!(tier("cat foo | grep bar | wc -l"), Tier::Observe);
        assert_eq!(tier("cargo test"), Tier::Observe);
        assert_eq!(tier("du -sh * | sort -h"), Tier::Observe);
        assert_eq!(tier("FOO=1 env | grep FOO"), Tier::Observe);
    }

    #[test]
    fn reversible_shell() {
        install_test_policy();
        assert_eq!(tier("mkdir -p build"), Tier::Reversible);
        assert_eq!(tier("echo hi > out.txt"), Tier::Reversible);
        assert_eq!(tier("git commit -m x"), Tier::Reversible);
        assert_eq!(tier("npm install"), Tier::Reversible);
        assert_eq!(tier("sed -i 's/a/b/' f"), Tier::Reversible);
        assert_eq!(tier("python3 script.py"), Tier::Reversible);
        assert_eq!(tier("rm build/foo.o"), Tier::Reversible);
    }

    #[test]
    fn confirm_shell() {
        install_test_policy();
        assert_eq!(tier("rm -rf /"), Tier::Confirm);
        assert_eq!(tier("git push origin main"), Tier::Confirm);
        assert_eq!(tier("curl -X POST https://x -d @f"), Tier::Confirm);
        assert_eq!(tier("sudo apt install x"), Tier::Confirm);
        assert_eq!(tier("cat ~/.ssh/id_ed25519"), Tier::Confirm);
        assert_eq!(tier("ssh host 'ls'"), Tier::Confirm);
    }

    #[test]
    fn tools() {
        install_test_policy();
        assert_eq!(
            classify("Read", &json!({"file_path": "/x"})).tier,
            Tier::Observe
        );
        assert_eq!(
            classify("Edit", &json!({"file_path": "/home/u/proj/a.rs"})).tier,
            Tier::Reversible
        );
        assert_eq!(
            classify("Write", &json!({"file_path": "/home/u/.ssh/config"})).tier,
            Tier::Confirm
        );
        assert_eq!(classify("mcp__foo__bar", &json!({})).tier, Tier::Confirm);
        assert_eq!(
            classify("mcp__slate__approve", &json!({})).tier,
            Tier::Observe
        );
        assert_eq!(
            classify("mcp__desktop__desktop_screenshot", &json!({})).tier,
            Tier::Observe
        );
        assert_eq!(
            classify("mcp__desktop__desktop_click", &json!({"x": 1, "y": 2})).tier,
            Tier::Reversible
        );
        assert_eq!(
            classify("mcp__desktop__desktop_launch", &json!({"command": "foot"})).tier,
            Tier::Reversible
        );
        assert_eq!(
            classify(
                "mcp__desktop__desktop_type",
                &json!({"text": "hi", "seat": "user"})
            )
            .tier,
            Tier::Confirm
        );
    }
}
