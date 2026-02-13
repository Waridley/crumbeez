mod root_discovery;

pub(crate) use std::collections::BTreeMap;
use zellij_tile::prelude::*;

use root_discovery::RootDiscovery;

#[derive(Default)]
struct State {
    /// Async root discovery state machine.
    discovery: RootDiscovery,
    /// Whether permissions have been granted yet.
    permissions_granted: bool,
}

impl ZellijPlugin for State {
    fn load(&mut self, _configuration: BTreeMap<String, String>) {
        request_permission(&[
            PermissionType::ReadApplicationState,
            PermissionType::RunCommands,
        ]);

        subscribe(&[
            EventType::PaneUpdate,
            EventType::TabUpdate,
            EventType::FileSystemUpdate,
            EventType::Timer,
            EventType::RunCommandResult,
            EventType::PermissionRequestResult,
        ]);
    }

    fn update(&mut self, event: Event) -> bool {
        match event {
            Event::PermissionRequestResult(PermissionStatus::Granted) => {
                self.permissions_granted = true;
                // Kick off root discovery now that we have RunCommands permission
                let cwd = get_plugin_ids().initial_cwd;
                eprintln!("[crumbeez] Permissions granted. initial_cwd: {:?}", cwd);
                self.discovery.start(cwd);
                true
            }
            Event::PermissionRequestResult(PermissionStatus::Denied) => {
                eprintln!("[crumbeez] Permissions denied!");
                self.discovery.phase =
                    root_discovery::DiscoveryPhase::Failed("Permissions denied".to_string());
                true
            }
            Event::RunCommandResult(exit_code, stdout, stderr, context) => self
                .discovery
                .handle_command_result(exit_code, &stdout, &stderr, &context),
            // TODO: handle other events for event collection
            _ => false,
        }
    }

    fn render(&mut self, _rows: usize, _cols: usize) {
        println!("crumbeez — breadcrumb logger");
        println!();
        println!("Root discovery: {}", self.discovery.phase);

        if let Some(ref git_root) = self.discovery.git_root {
            println!("  git root: {}", git_root.display());
        }
        if let Some(ref parent) = self.discovery.parent_git_root {
            println!("  parent repo: {}", parent.display());
        }
    }
}

register_plugin!(State);
