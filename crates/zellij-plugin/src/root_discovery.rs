use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tracing::{debug, error, info};
use zellij_tile::prelude::*;

pub use crumbeez_lib::DiscoveryPhase;

/// Context key used to tag run_command requests for root discovery.
const CTX_PURPOSE: &str = "crumbeez_purpose";

/// Identifies which async command produced a given `RunCommandResult`.
#[derive(Debug, Serialize, Deserialize)]
enum CommandPurpose {
    GitToplevel,
    GitSuperproject,
    MkdirCrumbeez,
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
            &["git", "rev-parse", "--show-toplevel"],
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
            None => return false, // Not our command
        };

        match purpose {
            CommandPurpose::GitToplevel => self.handle_git_toplevel(exit_code, stdout, stderr),
            CommandPurpose::GitSuperproject => {
                self.handle_git_superproject(exit_code, stdout, stderr)
            }
            CommandPurpose::MkdirCrumbeez => self.handle_mkdir_result(exit_code, stderr),
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

                // Check if this is a submodule
                run_command_with_env_variables_and_cwd(
                    &["git", "rev-parse", "--show-superproject-working-tree"],
                    BTreeMap::new(),
                    root_path,
                    purpose_context(CommandPurpose::GitSuperproject),
                );
                return true;
            }
        }

        // Not a git repo — use initial_cwd as root
        debug!(
            path = ?self.initial_cwd,
            "Not a git repo, using initial_cwd"
        );
        self.resolve_data_dir(vec![self.initial_cwd.clone()]);
        true
    }

    fn handle_git_superproject(
        &mut self,
        exit_code: Option<i32>,
        stdout: &[u8],
        _stderr: &[u8],
    ) -> bool {
        let mut roots = vec![];

        // Always include the git root itself
        if let Some(ref git_root) = self.git_root {
            roots.push(git_root.clone());
        }

        // If superproject found, also include it
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

        self.resolve_data_dir(roots);
        true
    }

    fn resolve_data_dir(&mut self, roots: Vec<PathBuf>) {
        let env = get_session_environment_variables();
        let data_home = env
            .get("XDG_DATA_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                env.get("HOME")
                    .map(|h| PathBuf::from(h).join(".local/share"))
                    .unwrap_or_else(|| PathBuf::from("/tmp/crumbeez"))
            });

        if !data_home.is_absolute() {
            self.phase = DiscoveryPhase::Failed(
                "Could not resolve data home directory: not an absolute path".to_string(),
            );
            return;
        }

        info!(data_home = ?data_home, "Resolved data home");
        self.create_dirs(roots, data_home);
    }

    fn handle_mkdir_result(&mut self, exit_code: Option<i32>, stderr: &[u8]) -> bool {
        if let DiscoveryPhase::CreatingDirs {
            ref mut pending,
            ref project_dirs,
            ref scratch_dir,
        } = self.phase
        {
            if exit_code != Some(0) {
                let err = String::from_utf8_lossy(stderr);
                error!(%err, "mkdir failed");
            }

            *pending = pending.saturating_sub(1);
            if *pending == 0 {
                info!(?project_dirs, ?scratch_dir, "Root discovery complete");
                let project_dirs = project_dirs.clone();
                let scratch_dir = scratch_dir.clone();
                self.phase = DiscoveryPhase::Ready {
                    project_dirs,
                    scratch_dir,
                };
            }
        }
        true
    }

    fn create_dirs(&mut self, roots: Vec<PathBuf>, data_home: PathBuf) {
        let project_dirs: Vec<PathBuf> = roots
            .iter()
            .map(|r| crumbeez_lib::crumbeez_dir(r))
            .collect();

        // The primary root is the first one (git root or initial_cwd).
        let primary_root = &roots[0];
        let scratch_dir = crumbeez_lib::global_scratch_dir(&data_home, primary_root);

        // Collect all directories that need to be created:
        // - In-repo summaries dirs for each project root
        // - Global scratchpad dir for the primary root
        let mut all_dirs: Vec<String> = Vec::new();
        for root in &roots {
            for dir in crumbeez_lib::required_project_dirs(root) {
                all_dirs.push(dir.to_string_lossy().into_owned());
            }
        }
        all_dirs.push(scratch_dir.to_string_lossy().into_owned());

        let dir_strs: Vec<&str> = all_dirs.iter().map(|s| s.as_str()).collect();
        let mut cmd: Vec<&str> = vec!["mkdir", "-p"];
        cmd.extend_from_slice(&dir_strs);

        run_command_with_env_variables_and_cwd(
            &cmd,
            BTreeMap::new(),
            self.initial_cwd.clone(),
            purpose_context(CommandPurpose::MkdirCrumbeez),
        );

        debug!(
            ?project_dirs,
            ?scratch_dir,
            "Creating project and scratchpad dirs"
        );

        self.phase = DiscoveryPhase::CreatingDirs {
            pending: 1, // Single mkdir -p command for all dirs
            project_dirs,
            scratch_dir,
        };
    }
}
