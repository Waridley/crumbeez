mod keystroke;
mod root_discovery;

use std::collections::{BTreeMap, HashMap};
use zellij_tile::prelude::*;

use crumbeez_lib::{KeystrokeActivity, KeystrokeEvent, PaneFocusedEvent};
use keystroke::{classify, key_to_bytes};
use root_discovery::RootDiscovery;

#[derive(Default)]
struct State {
    /// Async root discovery state.
    discovery: RootDiscovery,
    /// Whether permissions have been granted yet.
    permissions_granted: bool,
    /// Semantic keystroke activity log (proof-of-concept).
    keystroke_activity: KeystrokeActivity,
    /// The pane that currently has keyboard focus, used to detect switches.
    focused_pane: Option<FocusedPane>,
    /// Tab names keyed by tab position (0-indexed), kept up to date from
    /// `TabUpdate` events so `PaneUpdate` handling can label focus events.
    tab_names: HashMap<usize, String>,
}

/// Minimal description of the currently focused pane, stored so we can detect
/// when focus moves and emit a [`KeystrokeEvent::PaneFocused`] entry.
#[derive(Debug, Clone, PartialEq)]
struct FocusedPane {
    /// Zellij tab position (0-indexed).
    tab_index: usize,
    /// Pane id within its tab.
    pane_id: u32,
    /// `true` for plugin panes, `false` for terminal panes.
    is_plugin: bool,
}

impl State {
    /// Inspect a `PaneUpdate` manifest to detect focus changes.
    ///
    /// Iterates all tabs and panes to find the one with `is_focused = true`
    /// that is also selectable (i.e. not a UI bar).  If the focused pane has
    /// changed since the last update, a [`KeystrokeEvent::PaneFocused`] entry
    /// is pushed into the activity log.
    fn handle_pane_update(&mut self, manifest: PaneManifest) {
        let my_plugin_id = get_plugin_ids().plugin_id;

        // Walk all tabs in index order to find the focused pane.
        let mut new_focus: Option<(usize, PaneInfo)> = None;
        let mut focused_tab_name: Option<String> = None;

        // PaneManifest.panes is HashMap<usize, Vec<PaneInfo>> keyed by tab index.
        // We need to also know the tab name; that requires iterating the panes
        // and relying on the tab index alone — TabInfo is not included in
        // PaneUpdate, only the tab position.  We store tab names separately
        // when TabUpdate arrives; for now use tab index as fallback label.
        for (tab_index, panes) in &manifest.panes {
            for pane in panes {
                if !pane.is_selectable || pane.is_suppressed {
                    continue;
                }
                // Skip our own plugin pane — focus landing on ourselves during
                // dev is not interesting as a "context switch" event.
                if pane.is_plugin {
                    if let Some(ref url) = pane.plugin_url {
                        if url.contains("crumbeez") {
                            continue;
                        }
                    }
                    // Also skip by plugin id if we can identify ourselves.
                    if pane.id == my_plugin_id {
                        continue;
                    }
                }
                if pane.is_focused {
                    new_focus = Some((*tab_index, pane.clone()));
                    // Build a simple tab label from the index (1-based for display).
                    focused_tab_name = self
                        .tab_names
                        .get(tab_index)
                        .cloned()
                        .or_else(|| Some(format!("tab {}", tab_index + 1)));
                    break;
                }
            }
            if new_focus.is_some() {
                break;
            }
        }

        let Some((tab_index, pane)) = new_focus else {
            return;
        };

        let new_fp = FocusedPane {
            tab_index,
            pane_id: pane.id,
            is_plugin: pane.is_plugin,
        };

        // Only emit an event when focus has actually moved.
        if self.focused_pane.as_ref() == Some(&new_fp) {
            return;
        }

        self.focused_pane = Some(new_fp);

        let event = KeystrokeEvent::PaneFocused(PaneFocusedEvent {
            tab_name: focused_tab_name,
            pane_title: pane.title.clone(),
            command: pane.terminal_command.clone(),
            is_plugin: pane.is_plugin,
        });
        eprintln!("[crumbeez] {}", event);
        self.keystroke_activity.push_event(event);
    }
}

impl ZellijPlugin for State {
    fn load(&mut self, _configuration: BTreeMap<String, String>) {
        request_permission(&[
            PermissionType::ReadApplicationState,
            PermissionType::RunCommands,
            // InterceptInput: receive every keystroke session-wide via
            // InterceptedKeyPress.  We immediately re-forward each key back to
            // the focused pane so the user's input is not swallowed.
            PermissionType::InterceptInput,
            // WriteToStdin: needed to forward the intercepted keys back.
            PermissionType::WriteToStdin,
        ]);

        subscribe(&[
            // Key fires only when the plugin pane itself has focus.
            EventType::Key,
            // InterceptedKeyPress fires for every keystroke in any pane once
            // the InterceptInput permission is granted.
            EventType::InterceptedKeyPress,
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
                // Kick off root discovery now that we have RunCommands permission.
                let cwd = get_plugin_ids().initial_cwd;
                eprintln!("[crumbeez] Permissions granted. initial_cwd: {:?}", cwd);
                self.discovery.start(cwd);
                // Begin intercepting all keystrokes session-wide.  Each
                // InterceptedKeyPress will be logged and immediately forwarded
                // back to the focused pane so the user's input is not swallowed.
                intercept_key_presses();
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
            // InterceptedKeyPress: session-wide keystroke received because we
            // called intercept_key_presses().  Log it, then immediately write
            // the raw bytes back to the focused pane so the keystroke is not
            // swallowed.
            Event::InterceptedKeyPress(key) => {
                let bytes = key_to_bytes(&key);
                write(bytes);
                let event = classify(&key);
                eprintln!("[crumbeez] key event: {}", event);
                self.keystroke_activity.push_event(event);
                true
            }
            // Key fires only when this plugin pane itself has focus (i.e. the
            // user is interacting with the crumbeez pane directly).  No
            // forwarding needed — just log it.
            Event::Key(key) => {
                let event = classify(&key);
                eprintln!("[crumbeez] key event (plugin focused): {}", event);
                self.keystroke_activity.push_event(event);
                true
            }
            Event::TabUpdate(tabs) => {
                // Keep our tab name cache up to date.
                self.tab_names = tabs
                    .into_iter()
                    .filter(|t| !t.name.is_empty())
                    .map(|t| (t.position, t.name))
                    .collect();
                false // no re-render needed for this alone
            }
            Event::PaneUpdate(manifest) => {
                self.handle_pane_update(manifest);
                true
            }
            // TODO: handle other events for event collection
            _ => false,
        }
    }

    fn render(&mut self, rows: usize, cols: usize) {
        println!("crumbeez — breadcrumb logger");
        println!();
        println!("Root discovery: {}", self.discovery.phase);

        if let Some(ref git_root) = self.discovery.git_root {
            println!("  git root: {}", git_root.display());
        }
        if let Some(ref parent) = self.discovery.parent_git_root {
            println!("  parent repo: {}", parent.display());
        }

        println!();
        println!("─── Keystroke Activity ───────────────────────────────");

        let events = self.keystroke_activity.events();
        if events.is_empty() {
            println!("  (no keystrokes yet)");
        } else {
            // Show as many recent events as fit, newest last.  Reserve ~4
            // lines for the header above and leave one blank line at bottom.
            let available_lines = rows.saturating_sub(10).max(1);
            let skip = events.len().saturating_sub(available_lines);
            for event in events.iter().skip(skip) {
                let line = format!("  {}", event);
                // Truncate to terminal width so lines never wrap.
                let truncated = if cols > 4 && line.chars().count() > cols {
                    let mut s: String = line.chars().take(cols - 1).collect();
                    s.push('…');
                    s
                } else {
                    line
                };
                println!("{}", truncated);
            }
        }
    }
}

register_plugin!(State);
