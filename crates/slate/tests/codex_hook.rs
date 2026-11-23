//! Codex's PreToolUse hook (`slate hook codex-pre-tool-use`), run as Codex runs it:
//! the call's JSON on stdin, the answer on stdout. A stand-in daemon plays slated.

use serde_json::{json, Value};
use slate_proto::{Decision, Envelope, Reply, ReplyEnvelope, Request, Tier};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;
use std::path::Path;
use std::process::{Command, Stdio};

/// What the hook printed for one call. `task` is the Slate task Codex works for.
fn hook(sock: &Path, task: Option<&str>) -> String {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_slate"));
    cmd.args(["hook", "codex-pre-tool-use"])
        .env(slate_proto::ENV_SOCK, sock)
        .env_remove(slate_proto::ENV_TASK)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped());
    if let Some(task) = task {
        cmd.env(slate_proto::ENV_TASK, task);
    }
    let mut child = cmd.spawn().unwrap();
    let call = json!({
        "session_id": "s1", "cwd": "/home/u", "hook_event_name": "PreToolUse",
        "permission_mode": "bypassPermissions", "tool_name": "Bash",
        "tool_input": {"command": "cat ~/.ssh/id_ed25519"}, "tool_use_id": "exec-1"
    });
    child
        .stdin
        .take()
        .unwrap()
        .write_all(call.to_string().as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success(), "the hook itself must not fail");
    String::from_utf8(out.stdout).unwrap()
}

/// A daemon that answers one connection: the check with `decision`, and the approval
/// request that follows an Ask with `allow`. Returns the requests it saw.
fn daemon(sock: &Path, decision: Decision, allow: bool) -> std::thread::JoinHandle<Vec<Request>> {
    let listener = UnixListener::bind(sock).unwrap();
    std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let mut writer = stream.try_clone().unwrap();
        let mut seen = vec![];
        for line in BufReader::new(stream).lines() {
            let Ok(line) = line else { break };
            let env: Envelope = serde_json::from_str(&line).unwrap();
            let reply = match &env.request {
                Request::ToolCheck { .. } => Reply::ToolChecked {
                    tier: Tier::Confirm,
                    decision,
                    reason: "reads a key".into(),
                    snapshot: None,
                },
                Request::ApprovalRequest { .. } => Reply::Approval {
                    allow,
                    message: "denied by the user".into(),
                },
                _ => Reply::Ok,
            };
            seen.push(env.request);
            let reply = ReplyEnvelope { id: env.id, reply };
            writeln!(writer, "{}", serde_json::to_string(&reply).unwrap()).unwrap();
        }
        seen
    })
}

fn decision(out: &str) -> Option<(String, String)> {
    let v: Value = serde_json::from_str(out.trim()).ok()?;
    let h = v.get("hookSpecificOutput")?;
    assert_eq!(h["hookEventName"], "PreToolUse");
    Some((
        h["permissionDecision"].as_str()?.to_string(),
        h["permissionDecisionReason"].as_str()?.to_string(),
    ))
}

#[test]
fn codex_by_itself_is_left_alone() {
    // No Slate task: someone runs Codex themselves. Nothing is asked, nothing is said,
    // although no daemon answers.
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(hook(&dir.path().join("none.sock"), None), "");
}

#[test]
fn without_the_daemon_a_task_gets_nothing_through() {
    let dir = tempfile::tempdir().unwrap();
    let (verdict, reason) = decision(&hook(&dir.path().join("none.sock"), Some("t1"))).unwrap();
    assert_eq!(verdict, "deny");
    assert!(reason.contains("daemon"), "{reason}");
}

#[test]
fn an_allowed_call_goes_through_in_silence() {
    let dir = tempfile::tempdir().unwrap();
    let sock = dir.path().join("d.sock");
    let d = daemon(&sock, Decision::Allow, false);
    assert_eq!(hook(&sock, Some("t1")), "");
    let seen = d.join().unwrap();
    assert!(
        matches!(&seen[..], [Request::ToolCheck { task_id: Some(t), tool_name, .. }] if t == "t1" && tool_name == "Bash")
    );
}

#[test]
fn a_call_that_needs_the_person_waits_for_their_answer() {
    // Yes: the hook asked (Codex does not act on "ask" itself) and lets the call pass.
    let dir = tempfile::tempdir().unwrap();
    let sock = dir.path().join("d.sock");
    let d = daemon(&sock, Decision::Ask, true);
    assert_eq!(hook(&sock, Some("t1")), "");
    let seen = d.join().unwrap();
    assert!(matches!(
        &seen[..],
        [Request::ToolCheck { .. }, Request::ApprovalRequest { task_id: Some(t), .. }] if t == "t1"
    ));

    // No: the call is refused, with the person's word as the reason.
    let sock = dir.path().join("e.sock");
    let d = daemon(&sock, Decision::Ask, false);
    let (verdict, reason) = decision(&hook(&sock, Some("t1"))).unwrap();
    assert_eq!(verdict, "deny");
    assert!(reason.contains("denied by the user"), "{reason}");
    d.join().unwrap();
}

#[test]
fn what_the_policy_refuses_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let sock = dir.path().join("d.sock");
    let d = daemon(&sock, Decision::Deny, true);
    let (verdict, reason) = decision(&hook(&sock, Some("t1"))).unwrap();
    assert_eq!(verdict, "deny");
    assert!(reason.contains("reads a key"), "{reason}");
    assert_eq!(
        d.join().unwrap().len(),
        1,
        "a refusal is not put to the person"
    );
}
