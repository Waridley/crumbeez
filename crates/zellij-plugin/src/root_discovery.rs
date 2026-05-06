use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tracing::{debug, info};
use zellij_tile::prelude::*;

pub use crumbeez_lib::DiscoveryPhase;

/// The WASI guest path under which the plugin's host CWD is mounted.
pub const WASI_HOST_MOUNT: &str = "/host";

/// Context key used to tag run_command requests for root discovery.
const CTX_PURPOSE: &str = "crumbeez_purpose";

/// Identifies which async command produced a given `RunCommandResult`.
#[derive(Debug, Serialize, Deserialize)]
enum CommandPurpose {
    GitToplevel,
    GitSuperproject,
}

/// Build a context map tagged with the given purpose.
fn purpose_context(purpose: CommandPurpose) -> BTreeMap<String, String> {
    let mut ctx = BTreeMap::new();
    ctx.insert(
        CTX_PURPOSE.to_string(),
        serde_json::to_string(&purpose).expect("CommandPurpose serialization is infallible"),
    );
    ctx
}

/// Translate a host-absolute path rooted under `host_cwd` into a WASI guest
/// path rooted at `/host`.
///
/// Example: if `host_cwd` is `/home/user/proj` and `host_path` is
/// `/home/user/proj/.crumbeez/summaries`, the result is
/// `/host/.crumbeez/summaries`.
///
/// Falls back to the original path if it doesn't start with `host_cwd`.
pub fn to_wasi_host_path(host_cwd: &PathBuf, host_path: &PathBuf) -> PathBuf {
    if let Ok(rel) = host_path.strip_prefix(host_cwd) {
        PathBuf::from(WASI_HOST_MOUNT).join(rel)
    } else {
        host_path.clone()
    }
}

/// State for the root discovery process.
#[derive(Default, Debug)]
pub struct RootDiscovery {
    /// The cwd where the Zellij session was started.
    pub initial_cwd: PathBuf,
    /// The git root for the repo containing initial_cwd (if any).
    pub git_root: Option<PathBuf>,
    /// The parent git repo root (if initial_cwd is inside a submodule).
    pub parent_git_root: Option<PathBuf>,
    /// Current phase of the discovery state machine.
    pub phase: DiscoveryPhase,
}

impl RootDiscovery {
    /// Initialize with the plugin's initial_cwd and kick off discovery.
    /// Call this once permissions have been granted.
    pub fn start(&mut self, initial_cwd: PathBuf) {
        self.initial_cwd = initial_cwd.clone();
        self.phase = DiscoveryPhase::FindingGitRoot;

        run_command_with_env_variables_and_cwd(
            &["sh", "-c", "git rev-parse --show-toplevel 2>/dev/null"],
            BTreeMap::new(),
            initial_cwd,
            purpose_context(CommandPurpose::GitToplevel),
        );
    }

    /// Handle a RunCommandResult event. Returns true if this event was consumed
    /// by the discovery process (i.e. it was tagged with our context key).
    pub fn handle_command_result(
        &mut self,
        exit_code: Option<i32>,
        stdout: &[u8],
        stderr: &[u8],
        context: &BTreeMap<String, String>,
    ) -> bool {
        let purpose: CommandPurpose = match context.get(CTX_PURPOSE) {
            Some(s) => match serde_json::from_str(s) {
                Ok(p) => p,
                Err(_) => return false,
            },
            None => return false,
        };

        match purpose {
            CommandPurpose::GitToplevel => self.handle_git_toplevel(exit_code, stdout, stderr),
            CommandPurpose::GitSuperproject => {
                self.handle_git_superproject(exit_code, stdout, stderr)
            }
        }
    }

    fn handle_git_toplevel(
        &mut self,
        exit_code: Option<i32>,
        stdout: &[u8],
        _stderr: &[u8],
    ) -> bool {
        if exit_code == Some(0) {
            let root = String::from_utf8_lossy(stdout).trim().to_string();
            if !root.is_empty() {
                let root_path = PathBuf::from(&root);
                self.git_root = Some(root_path.clone());
                self.phase = DiscoveryPhase::FindingSuperproject;

                run_command_with_env_variables_and_cwd(
                    &[
                        "sh",
                        "-c",
                        "git rev-parse --show-superproject-working-tree 2>/dev/null",
                    ],
                    BTreeMap::new(),
                    root_path,
                    purpose_context(CommandPurpose::GitSuperproject),
                );
                return true;
            }
        }

        debug!(
            path = ?self.initial_cwd,
            "Not a git repo, using initial_cwd"
        );
        self.create_dirs(vec![self.initial_cwd.clone()]);
        true
    }

    fn handle_git_superproject(
        &mut self,
        exit_code: Option<i32>,
        stdout: &[u8],
        _stderr: &[u8],
    ) -> bool {
        let mut roots = vec![];

        if let Some(ref git_root) = self.git_root {
            roots.push(git_root.clone());
        }

        if exit_code == Some(0) {
            let superproject = String::from_utf8_lossy(stdout).trim().to_string();
            if !superproject.is_empty() {
                let parent_path = PathBuf::from(&superproject);
                info!(
                    parent = ?parent_path,
                    "Submodule detected"
                );
                self.parent_git_root = Some(parent_path.clone());
                roots.push(parent_path);
            }
        }

        self.create_dirs(roots);
        true
    }

    /// Create all required in-repo directories via WASI `/host` paths, then
    /// transition to `Ready`.
    fn create_dirs(&mut self, roots: Vec<PathBuf>) {
        // Collect the host-absolute project dirs (.crumbeez/...) for each root.
        // These are the canonical paths stored in the phase for use by callers
        // that need to know the host layout (e.g. for display purposes).
        let project_dirs: Vec<PathBuf> = roots
            .iter()
            .map(|r| crumbeez_lib::crumbeez_dir(r))
            .collect();

        // For std::fs calls inside WASI we must use the /host-prefixed paths.
        // The plugin CWD (initial_cwd) is mounted at /host, so we translate
        // each directory by stripping the initial_cwd prefix and prepending /host.
        let mut created_count = 0;
        for root in &roots {
            for host_dir in crumbeez_lib::required_project_dirs(root) {
                let wasi_dir = to_wasi_host_path(&self.initial_cwd, &host_dir);
                match fs::create_dir_all(&wasi_dir) {
                    Ok(_) => {
                        info!(path = ?wasi_dir, "Created directory");
                        created_count += 1;
                    }
                    Err(e) => {
                        debug!(?e, path = ?wasi_dir, "Failed to create directory");
                    }
                }
            }
        }

        info!(
            ?project_dirs,
            created = created_count,
            "Root discovery complete"
        );

        self.phase = DiscoveryPhase::Ready { project_dirs };
    }
}
