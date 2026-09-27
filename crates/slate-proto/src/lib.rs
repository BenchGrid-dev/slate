//! Shared types for the Slate agent runtime.
//!
//! Everything that crosses a process boundary (slash <-> slated <-> slate-desktop)
//! is defined here so that the components can evolve independently.
//! Wire format and IPC transport are still open questions; see docs/architecture.md.

/// How much human involvement an action needs before it runs.
///
/// See docs/architecture.md, "Approval tiers".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalTier {
    /// Read-only. Runs silently.
    Observe,
    /// Reversible. Runs silently after a snapshot; user can say "undo".
    Reversible,
    /// Destructive, sends data off-machine, or touches credentials. Needs explicit approval.
    Confirm,
}

/// Which agent backend drives a session. Slate never talks to a model directly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Backend {
    ClaudeCode,
    Codex,
    Other(String),
}

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
