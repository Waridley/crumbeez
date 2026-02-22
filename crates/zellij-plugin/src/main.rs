mod event_log_io;
mod keystroke;
mod root_discovery;

use std::collections::{BTreeMap, HashMap};
use zellij_tile::prelude::*;

use crumbeez_lib::{
    EditControlEvent, EventLog, KeystrokeActivity, KeystrokeEvent, NavDirection, PaneFocusedEvent,
};
use event_log_io::EventLogIO;
use keystroke::{classify, key_to_bytes};
use root_discovery::RootDiscovery;

#[derive(Default)]
struct State {
    discovery: RootDiscovery,
    permissions_granted: bool,
    keystroke_activity: KeystrokeActivity,
    focused_pane: Option<FocusedPane>,
    tab_names: HashMap<usize, String>,
    event_log: EventLog,
    event_log_io: EventLogIO,
    pending_summaries: Vec<String>,
    live_text: Option<String>,
    live_cursor: usize,
}

#[derive(Debug, Clone, PartialEq)]
struct FocusedPane {
    tab_index: usize,
    pane_id: u32,
    is_plugin: bool,
}

const SUMMARY_TIMER_SECS: f64 = 60.0;

impl State {
    fn log_event(&mut self, event: KeystrokeEvent) {
        self.keystroke_activity.push_event(event.clone());
        self.process_for_event_log(event);
    }

    fn process_for_event_log(&mut self, event: KeystrokeEvent) {
        match &event {
            KeystrokeEvent::TextTyped(s) => {
                if let Some(ref mut text) = self.live_text {
                    text.insert_str(self.live_cursor, s);
                    self.live_cursor += s.len();
                } else {
                    self.live_text = Some(s.clone());
                    self.live_cursor = s.len();
                }
            }
            KeystrokeEvent::EditControl(EditControlEvent::Backspace { .. }) => {
                if let Some(ref mut text) = self.live_text {
                    if self.live_cursor > 0 {
                        let prev = prev_char_boundary(text, self.live_cursor);
                        text.drain(prev..self.live_cursor);
                        self.live_cursor = prev;
                        if text.is_empty() {
                            self.live_text = None;
                        }
                    }
                }
            }
            KeystrokeEvent::EditControl(EditControlEvent::Delete { .. }) => {
                if let Some(ref mut text) = self.live_text {
                    if self.live_cursor < text.len() {
                        let next = next_char_boundary(text, self.live_cursor);
                        text.drain(self.live_cursor..next);
                        if text.is_empty() {
                            self.live_text = None;
                        }
                    }
                }
            }
            KeystrokeEvent::Navigation(nav) => match nav.direction {
                NavDirection::Left => {
                    if let Some(ref text) = self.live_text {
                        let new_pos = if nav.with_ctrl {
                            word_left(text, self.live_cursor)
                        } else {
                            prev_char_boundary(text, self.live_cursor)
                        };
                        self.live_cursor = new_pos;
                    }
                }
                NavDirection::Right => {
                    if let Some(ref text) = self.live_text {
                        let new_pos = if nav.with_ctrl {
                            word_right(text, self.live_cursor)
                        } else {
                            next_char_boundary(text, self.live_cursor)
                        };
                        self.live_cursor = new_pos;
                    }
                }
                NavDirection::Home => {
                    self.live_cursor = 0;
                }
                NavDirection::End => {
                    if let Some(ref text) = self.live_text {
                        self.live_cursor = text.len();
                    }
                }
                NavDirection::Up
                | NavDirection::Down
                | NavDirection::PageUp
                | NavDirection::PageDown => {
                    self.seal_and_log(event);
                }
            },
            _ => {
                self.seal_and_log(event);
            }
        }
    }

    fn seal_and_log(&mut self, event: KeystrokeEvent) {
        if let Some(text) = self.live_text.take() {
            if !text.is_empty() {
                self.event_log
                    .append(KeystrokeEvent::TextTyped(text), Self::current_time_ms());
            }
        }
        self.live_cursor = 0;
        self.event_log.append(event, Self::current_time_ms());
    }

    fn seal_pending_text(&mut self) {
        if let Some(text) = self.live_text.take() {
            if !text.is_empty() {
                self.event_log
                    .append(KeystrokeEvent::TextTyped(text), Self::current_time_ms());
            }
        }
        self.live_cursor = 0;
    }

    fn current_time_ms() -> u64 {
        use std::time::{SystemTime, UNIX_EPOCH};
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64
    }

    fn handle_discovery_ready(&mut self) {
        eprintln!(
            "[crumbeez] handle_discovery_ready called, phase: {:?}",
            self.discovery.phase
        );
        if let crumbeez_lib::DiscoveryPhase::Ready { ref dirs } = self.discovery.phase {
            if let Some(dir) = dirs.first() {
                let log_path = crumbeez_lib::event_log_path_from_crumbeez_dir(dir);
                eprintln!("[crumbeez] Log path: {:?}", log_path);
                self.event_log_io.set_log_path(log_path.clone());
                self.event_log_io.load(self.discovery.initial_cwd.clone());
                set_timeout(SUMMARY_TIMER_SECS);
            }
        }
    }

    fn handle_pane_update(&mut self, manifest: PaneManifest) {
        let my_plugin_id = get_plugin_ids().plugin_id;
        let mut new_focus: Option<(usize, PaneInfo)> = None;
        let mut focused_tab_name: Option<String> = None;

        for (tab_index, panes) in &manifest.panes {
            for pane in panes {
                if !pane.is_selectable || pane.is_suppressed {
                    continue;
                }
                if pane.is_plugin {
                    if let Some(ref url) = pane.plugin_url {
                        if url.contains("crumbeez") {
                            continue;
                        }
                    }
                    if pane.id == my_plugin_id {
                        continue;
                    }
                }
                if pane.is_focused {
                    new_focus = Some((*tab_index, pane.clone()));
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
        self.log_event(event);
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
                let cwd = get_plugin_ids().initial_cwd;
                eprintln!("[crumbeez] Permissions granted. initial_cwd: {:?}", cwd);
                self.discovery.start(cwd);
                intercept_key_presses();
                true
            }
            Event::PermissionRequestResult(PermissionStatus::Denied) => {
                eprintln!("[crumbeez] Permissions denied!");
                self.discovery.phase =
                    root_discovery::DiscoveryPhase::Failed("Permissions denied".to_string());
                true
            }
            Event::RunCommandResult(exit_code, stdout, stderr, context) => {
                if self.event_log_io.handle_result(
                    &context,
                    &stdout,
                    exit_code,
                    &mut self.event_log,
                ) {
                    return true;
                }
                let was_creating = matches!(
                    self.discovery.phase,
                    crumbeez_lib::DiscoveryPhase::CreatingDirs { .. }
                );
                let handled = self
                    .discovery
                    .handle_command_result(exit_code, &stdout, &stderr, &context);
                if was_creating
                    && matches!(
                        self.discovery.phase,
                        crumbeez_lib::DiscoveryPhase::Ready { .. }
                    )
                {
                    self.handle_discovery_ready();
                }
                handled
            }
            Event::InterceptedKeyPress(key) => {
                let bytes = key_to_bytes(&key);
                write(bytes);
                let event = classify(&key);
                eprintln!("[crumbeez] key event: {}", event);
                self.log_event(event);
                true
            }
            Event::Key(key) => {
                let event = classify(&key);
                eprintln!("[crumbeez] key event (plugin focused): {}", event);
                self.log_event(event);
                true
            }
            Event::TabUpdate(tabs) => {
                self.tab_names = tabs
                    .into_iter()
                    .filter(|t| !t.name.is_empty())
                    .map(|t| (t.position, t.name))
                    .collect();
                false
            }
            Event::PaneUpdate(manifest) => {
                self.handle_pane_update(manifest);
                true
            }
            Event::Timer(_) => {
                eprintln!(
                    "[crumbeez] Timer fired, unconsumed: {}",
                    self.event_log.unconsumed_count()
                );
                self.seal_pending_text();
                if self.event_log.unconsumed_count() > 0 {
                    if let Some(summary) = event_log_io::generate_summary(&mut self.event_log) {
                        self.pending_summaries.push(summary);
                        if self.pending_summaries.len() > 10 {
                            self.pending_summaries.remove(0);
                        }
                    }
                    if let Ok(data) = self.event_log.serialize() {
                        self.event_log_io
                            .save(self.discovery.initial_cwd.clone(), data);
                    } else {
                        eprintln!("[crumbeez] Failed to serialize event log");
                    }
                }
                set_timeout(SUMMARY_TIMER_SECS);
                true
            }
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
        println!("─── Event Log ─────────────────────────────────────────");
        println!(
            "  Total: {} events, {} unconsumed",
            self.event_log.total_count(),
            self.event_log.unconsumed_count()
        );

        if !self.pending_summaries.is_empty() {
            println!();
            println!("─── Summaries ─────────────────────────────────────────");
            for summary in &self.pending_summaries {
                for line in summary.lines() {
                    let truncated = if cols > 4 && line.chars().count() > cols {
                        let mut s: String = line.chars().take(cols - 1).collect();
                        s.push('…');
                        s
                    } else {
                        line.to_string()
                    };
                    println!("{}", truncated);
                }
            }
        }

        println!();
        println!("─── Keystroke Activity ───────────────────────────────");

        let events = self.keystroke_activity.events();
        if events.is_empty() {
            println!("  (no keystrokes yet)");
        } else {
            let available_lines = rows.saturating_sub(15).max(1);
            let skip = events.len().saturating_sub(available_lines);
            for event in events.iter().skip(skip) {
                let line = format!("  {}", event);
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

fn prev_char_boundary(s: &str, pos: usize) -> usize {
    if pos == 0 {
        return 0;
    }
    let mut p = pos - 1;
    while p > 0 && !s.is_char_boundary(p) {
        p -= 1;
    }
    p
}

fn next_char_boundary(s: &str, pos: usize) -> usize {
    if pos >= s.len() {
        return s.len();
    }
    let mut p = pos + 1;
    while p < s.len() && !s.is_char_boundary(p) {
        p += 1;
    }
    p
}

fn word_left(s: &str, pos: usize) -> usize {
    let chars_before: Vec<(usize, char)> = s[..pos].char_indices().collect();
    if chars_before.is_empty() {
        return 0;
    }
    let mut iter = chars_before.iter().rev();
    while let Some(&(_, c)) = iter.next() {
        if c.is_alphanumeric() || c == '_' {
            break;
        }
    }
    for &(i, c) in iter {
        if !c.is_alphanumeric() && c != '_' {
            return next_char_boundary(s, i);
        }
    }
    0
}

fn word_right(s: &str, pos: usize) -> usize {
    let chars_after: Vec<(usize, char)> =
        s[pos..].char_indices().map(|(i, c)| (pos + i, c)).collect();
    if chars_after.is_empty() {
        return s.len();
    }
    let mut iter = chars_after.iter();
    let mut found_word = false;
    for &(_i, c) in iter.by_ref() {
        if c.is_alphanumeric() || c == '_' {
            found_word = true;
            break;
        }
    }
    if !found_word {
        return s.len();
    }
    for &(byte_i, c) in iter.by_ref() {
        if !c.is_alphanumeric() && c != '_' {
            return byte_i;
        }
    }
    s.len()
}

register_plugin!(State);
