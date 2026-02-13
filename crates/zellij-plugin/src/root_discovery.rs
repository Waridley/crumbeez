use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
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
        eprintln!(
            "[crumbeez] Not a git repo, using initial_cwd: {:?}",
            self.initial_cwd
        );
        self.create_crumbeez_dirs(vec![self.initial_cwd.clone()]);
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
                eprintln!(
                    "[crumbeez] Submodule detected. Parent repo: {:?}",
                    parent_path
                );
                self.parent_git_root = Some(parent_path.clone());
                roots.push(parent_path);
            }
        }

        self.create_crumbeez_dirs(roots);
        true
    }

    fn handle_mkdir_result(&mut self, exit_code: Option<i32>, stderr: &[u8]) -> bool {
        if let DiscoveryPhase::CreatingDirs {
            ref mut pending,
            ref dirs,
        } = self.phase
        {
            if exit_code != Some(0) {
                let err = String::from_utf8_lossy(stderr);
                eprintln!("[crumbeez] mkdir failed: {}", err);
            }

            *pending = pending.saturating_sub(1);
            if *pending == 0 {
                eprintln!("[crumbeez] Root discovery complete. Dirs: {:?}", dirs);
                // Move dirs out of CreatingDirs into Ready
                let dirs = dirs.clone();
                self.phase = DiscoveryPhase::Ready { dirs };
            }
        }
        true
    }

    fn create_crumbeez_dirs(&mut self, roots: Vec<PathBuf>) {
        let count = roots.len();
        let dirs: Vec<PathBuf> = roots
            .iter()
            .map(|r| crumbeez_lib::crumbeez_dir(r))
            .collect();

        for root in &roots {
            let mkdir_args: Vec<String> = crumbeez_lib::required_dirs(root)
                .into_iter()
                .map(|d| d.to_string_lossy().into_owned())
                .collect();
            let mkdir_strs: Vec<&str> = mkdir_args.iter().map(|s| s.as_str()).collect();

            let mut cmd: Vec<&str> = vec!["mkdir", "-p"];
            cmd.extend_from_slice(&mkdir_strs);

            run_command_with_env_variables_and_cwd(
                &cmd,
                BTreeMap::new(),
                self.initial_cwd.clone(),
                purpose_context(CommandPurpose::MkdirCrumbeez),
            );

            eprintln!(
                "[crumbeez] Creating .crumbeez dir at: {:?}",
                crumbeez_lib::crumbeez_dir(root)
            );
        }

        self.phase = DiscoveryPhase::CreatingDirs {
            pending: count,
            dirs,
        };
    }
}
