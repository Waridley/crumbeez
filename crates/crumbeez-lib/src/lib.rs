use std::fmt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

// ── Directory layout constants ───────────────────────────────────

/// Name of the crumbeez data directory created at each project root.
pub const CRUMBEEZ_DIR_NAME: &str = ".crumbeez";

/// Subdirectory for event logs (JSONL).
pub const EVENTS_SUBDIR: &str = "events";

/// Subdirectory for human-readable summary logs (Markdown).
pub const SUMMARIES_SUBDIR: &str = "summaries";

// ── Directory layout helpers ─────────────────────────────────────

/// Returns the `.crumbeez` directory path for a given project root.
pub fn crumbeez_dir(root: &Path) -> PathBuf {
    root.join(CRUMBEEZ_DIR_NAME)
}

/// Returns the events subdirectory path for a given project root.
pub fn events_dir(root: &Path) -> PathBuf {
    crumbeez_dir(root).join(EVENTS_SUBDIR)
}

/// Returns the summaries subdirectory path for a given project root.
pub fn summaries_dir(root: &Path) -> PathBuf {
    crumbeez_dir(root).join(SUMMARIES_SUBDIR)
}

/// Returns all directories that must exist for a given project root.
pub fn required_dirs(root: &Path) -> Vec<PathBuf> {
    vec![events_dir(root), summaries_dir(root)]
}

// ── Discovery phase ──────────────────────────────────────────────

/// Async state machine phases for root discovery.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub enum DiscoveryPhase {
    /// Waiting for RunCommands permission to be granted.
    #[default]
    AwaitingPermissions,
    /// Fired `git rev-parse --show-toplevel`, waiting for result.
    FindingGitRoot,
    /// Fired `git rev-parse --show-superproject-working-tree`, waiting for result.
    FindingSuperproject,
    /// Fired `mkdir -p` commands, waiting for them to complete.
    CreatingDirs { pending: usize, dirs: Vec<PathBuf> },
    /// All .crumbeez directories have been created and are ready.
    Ready { dirs: Vec<PathBuf> },
    /// Discovery failed with an error message.
    Failed(String),
}

impl fmt::Display for DiscoveryPhase {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AwaitingPermissions => write!(f, "⏳ Awaiting permissions..."),
            Self::FindingGitRoot => write!(f, "🔍 Finding git root..."),
            Self::FindingSuperproject => write!(f, "🔍 Checking for parent repo..."),
            Self::CreatingDirs { pending, .. } => {
                write!(f, "📁 Creating .crumbeez dirs ({pending} remaining)...")
            }
            Self::Ready { dirs } => {
                let dirs: Vec<_> = dirs.iter().map(|d| d.to_string_lossy()).collect();
                write!(f, "✅ Ready — {}", dirs.join(", "))
            }
            Self::Failed(msg) => write!(f, "❌ Failed: {msg}"),
        }
    }
}

