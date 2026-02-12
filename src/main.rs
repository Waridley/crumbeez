use std::collections::BTreeMap;
use zellij_tile::prelude::*;

#[derive(Default)]
struct State {
    // TODO: add fields for event collection, logs, etc.
}

impl ZellijPlugin for State {
    fn load(&mut self, _configuration: BTreeMap<String, String>) {
        // Called once on plugin load
        request_permission(&[PermissionType::ReadApplicationState]);

        subscribe(&[
            EventType::PaneUpdate,
            EventType::TabUpdate,
            EventType::FileSystemUpdate,
            EventType::Timer,
        ]);
    }

    fn update(&mut self, _event: Event) -> bool {
        // TODO: implement event handling and summarization orchestration
        false
    }

    fn render(&mut self, _rows: usize, _cols: usize) {
        // Minimal placeholder UI
	        println!("crumbeez plugin loaded (skeleton)");
    }
}

register_plugin!(State);
