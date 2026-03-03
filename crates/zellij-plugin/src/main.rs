mod config_io;
mod event_log_io;
mod keystroke;
mod llm_io;
#[cfg(feature = "pane-content-tracking")]
mod pane_content;
mod root_discovery;
mod summary_io;

use console_error_panic_hook;
use std::collections::{BTreeMap, HashMap};
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::{debug, error, info};
use zellij_tile::prelude::*;
use zellij_tile::prelude::{BareKey, KeyWithModifier};

use config_io::ConfigIO;
use crumbeez_lib::{
    EditControlEvent, EventLog, KeystrokeActivity, KeystrokeEvent, LLMBackend, NavDirection,
    PaneFocusedEvent,
};
use event_log_io::EventLogIO;
use keystroke::{classify, key_to_bytes};
use llm_io::{LLMRequestor, LLMResult};
use root_discovery::{to_wasi_host_path, RootDiscovery};
use summary_io::SummaryIO;

#[derive(Default)]
struct State {
    discovery: RootDiscovery,
    permissions_granted: bool,
    keystroke_activity: KeystrokeActivity,
    focused_pane: Option<FocusedPane>,
    current_pane_has_activity: bool,
    tab_names: HashMap<usize, String>,
    event_log: EventLog,
    event_log_io: EventLogIO,
    summary_io: SummaryIO,
    pending_summaries: Vec<String>,
    live_text: Option<String>,
    live_cursor: usize,
    last_activity_time: Option<SystemTime>,
    last_summary_time: Option<SystemTime>,
    config_io: ConfigIO,
    onboarding_active: bool,
    onboarding_selection: usize,
    llm_requestor: LLMRequestor,
    #[cfg(feature = "pane-content-tracking")]
    pane_registry: pane_content::PaneRegistry,
}

#[derive(Debug, Clone, PartialEq)]
struct FocusedPane {
    tab_index: usize,
    pane_id: u32,
    is_plugin: bool,
}

const INACTIVITY_TIMER_SECS: f64 = 10.0;

impl State {
    fn log_event(&mut self, event: KeystrokeEvent) {
        self.keystroke_activity.push_event(event.clone());
        self.process_for_event_log(event);
        // Mark that this pane has had activity (for summary triggering on pane switch)
        self.current_pane_has_activity = true;
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

        self.last_activity_time = Some(SystemTime::now());
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
        use std::time::SystemTime;
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64
    }

    fn handle_discovery_ready(&mut self) {
        debug!(
            phase = ?self.discovery.phase,
            "handle_discovery_ready called"
        );
        if let crumbeez_lib::DiscoveryPhase::Ready { ref project_dirs } = self.discovery.phase {
            self.event_log_io.load_into(&mut self.event_log);

            if let Some(project_dir) = project_dirs.first() {
                // Translate the host-absolute project dir to its WASI /host path.
                let wasi_summaries = to_wasi_host_path(
                    &self.discovery.initial_cwd,
                    &project_dir.join(crumbeez_lib::SUMMARIES_SUBDIR),
                );
                info!(path = ?wasi_summaries, "Summary IO initialized");
                self.summary_io.set_summaries_dir(wasi_summaries);

                let wasi_config_dir = to_wasi_host_path(
                    &self.discovery.initial_cwd,
                    &project_dir.join(crumbeez_lib::CRUMBEEZ_DIR_NAME),
                );
                self.config_io.set_config_dir(wasi_config_dir);
                self.config_io.request_load();
            }

            self.reset_inactivity_timer();
        }
    }

    fn reset_inactivity_timer(&mut self) {
        debug!(secs = INACTIVITY_TIMER_SECS, "Resetting inactivity timer");
        set_timeout(INACTIVITY_TIMER_SECS);
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

        debug!(
            from = ?self.focused_pane,
            to = ?new_fp,
            "Pane focus changed"
        );

        // Trigger summary when switching away from a pane that had activity
        if self.current_pane_has_activity {
            self.trigger_summary_for_pane_switch();
        }

        // Notify the pane registry that focus has moved (flushes old pane).
        #[cfg(feature = "pane-content-tracking")]
        if let Some(pane_event) = self.pane_registry.on_focus_changed(pane.id) {
            self.event_log.append(
                KeystrokeEvent::PaneOutput(pane_event),
                Self::current_time_ms(),
            );
        }

        // Switch to new pane and reset activity flag
        self.focused_pane = Some(new_fp);
        self.current_pane_has_activity = false;

        let event = KeystrokeEvent::PaneFocused(PaneFocusedEvent {
            tab_name: focused_tab_name,
            pane_title: pane.title.clone(),
            command: pane.terminal_command.clone(),
            is_plugin: pane.is_plugin,
        });
        info!(%event);
        self.log_event(event);
    }

    fn trigger_summary_for_pane_switch(&mut self) {
        debug!("trigger_summary_for_pane_switch called");
        self.seal_pending_text();
        let unconsumed = self.event_log.unconsumed_count();
        if unconsumed > 0 {
            info!(
                count = unconsumed,
                "Pane switch trigger, summarizing events"
            );

            if matches!(self.llm_requestor.backend(), LLMBackend::Ollama { .. })
                && !self.llm_requestor.is_pending()
            {
                if let Some((events, event_count)) =
                    event_log_io::extract_events_for_llm(&mut self.event_log)
                {
                    info!(event_count, "Requesting LLM summary");
                    self.llm_requestor.request_leaf_summary(events, event_count);
                }
            } else if let Some(summary) = event_log_io::generate_summary(&mut self.event_log) {
                self.pending_summaries.push(summary.clone());
                if self.pending_summaries.len() > 10 {
                    self.pending_summaries.remove(0);
                }
                self.summary_io.save_summary_text(summary);
            }

            self.event_log.compact();
            if let Ok(data) = self.event_log.serialize() {
                self.event_log_io.save(data);
            } else {
                error!("Failed to serialize event log");
            }
        }
    }

    fn handle_onboarding_key(&mut self, key: &KeyWithModifier) -> bool {
        match key.bare_key {
            BareKey::Up | BareKey::Char('k') => {
                if self.onboarding_selection > 0 {
                    self.onboarding_selection -= 1;
                }
                true
            }
            BareKey::Down | BareKey::Char('j') => {
                if self.onboarding_selection < 3 {
                    self.onboarding_selection += 1;
                }
                true
            }
            BareKey::Enter => {
                let backend = match self.onboarding_selection {
                    0 => LLMBackend::NoLLM,
                    1 => LLMBackend::default_ollama(),
                    2 => LLMBackend::default_openai(),
                    3 => LLMBackend::default_anthropic(),
                    _ => LLMBackend::NoLLM,
                };
                self.config_io.config_mut().llm.backend = Some(backend.clone());
                self.llm_requestor.set_backend(backend);
                self.config_io.request_save();
                self.onboarding_active = false;
                info!(backend = ?self.config_io.config().llm.backend, "Onboarding complete");
                true
            }
            _ => false,
        }
    }

    fn render_onboarding(&self, _rows: usize, cols: usize) {
        println!("╔══════════════════════════════════════════════════════╗");
        println!("║       crumbeez — Choose Your LLM Backend              ║");
        println!("╠══════════════════════════════════════════════════════╣");
        println!("║                                                      ║");
        println!("║  Select how you want crumbeez to generate summaries: ║");
        println!("║                                                      ║");
        println!("╟──────────────────────────────────────────────────────╢");

        let options = [
            ("No LLM", "Record events only, no summaries"),
            ("Ollama (Local)", "Run LLM locally via Ollama"),
            ("OpenAI", "Cloud API (requires API key)"),
            ("Anthropic", "Cloud API (requires API key)"),
        ];

        for (i, (name, desc)) in options.iter().enumerate() {
            let marker = if i == self.onboarding_selection {
                "►"
            } else {
                " "
            };
            let line = format!("║  {} {} - {}", marker, name, desc);
            let padded = if cols > 4 && line.chars().count() > cols - 1 {
                let mut s: String = line.chars().take(cols - 2).collect();
                s.push_str("║");
                s
            } else {
                format!("{:width$}║", line, width = cols.saturating_sub(1))
            };
            println!("{}", padded);
        }

        println!("║                                                      ║");
        println!("║  ↑/↓ or j/k to select, Enter to confirm              ║");
        println!("╚══════════════════════════════════════════════════════╝");
    }
}

impl ZellijPlugin for State {
    fn load(&mut self, _configuration: BTreeMap<String, String>) {
        let _ = tracing_subscriber::fmt()
            .with_writer(std::io::stderr)
            .with_target(false)
            .try_init();

        console_error_panic_hook::set_once();

        #[cfg(feature = "pane-content-tracking")]
        let permissions = vec![
            PermissionType::ReadApplicationState,
            PermissionType::RunCommands,
            PermissionType::InterceptInput,
            PermissionType::FullHdAccess,
            PermissionType::WriteToStdin,
            PermissionType::ReadPaneContents,
            PermissionType::ReadSessionEnvironmentVariables,
            PermissionType::WebAccess,
        ];
        #[cfg(not(feature = "pane-content-tracking"))]
        let permissions = vec![
            PermissionType::ReadApplicationState,
            PermissionType::RunCommands,
            PermissionType::InterceptInput,
            PermissionType::FullHdAccess,
            PermissionType::WriteToStdin,
            PermissionType::ReadSessionEnvironmentVariables,
            PermissionType::WebAccess,
        ];
        request_permission(&permissions);

        #[cfg(feature = "pane-content-tracking")]
        let event_types = vec![
            EventType::Key,
            EventType::InterceptedKeyPress,
            EventType::PaneUpdate,
            EventType::TabUpdate,
            EventType::FileSystemUpdate,
            EventType::Timer,
            EventType::RunCommandResult,
            EventType::PermissionRequestResult,
            EventType::PaneRenderReport,
            EventType::WebRequestResult,
        ];
        #[cfg(not(feature = "pane-content-tracking"))]
        let event_types = vec![
            EventType::Key,
            EventType::InterceptedKeyPress,
            EventType::PaneUpdate,
            EventType::TabUpdate,
            EventType::FileSystemUpdate,
            EventType::Timer,
            EventType::RunCommandResult,
            EventType::PermissionRequestResult,
            EventType::WebRequestResult,
        ];
        subscribe(&event_types);
    }

    fn update(&mut self, event: Event) -> bool {
        let result = match event {
            Event::PermissionRequestResult(PermissionStatus::Granted) => {
                self.permissions_granted = true;
                let cwd = get_plugin_ids().initial_cwd;
                info!(?cwd, "Permissions granted");
                self.discovery.start(cwd);
                intercept_key_presses();
                true
            }
            Event::PermissionRequestResult(PermissionStatus::Denied) => {
                error!("Permissions denied");
                self.discovery.phase =
                    root_discovery::DiscoveryPhase::Failed("Permissions denied".to_string());
                true
            }
            Event::RunCommandResult(exit_code, stdout, stderr, context) => {
                let was_ready_before = matches!(
                    self.discovery.phase,
                    crumbeez_lib::DiscoveryPhase::Ready { .. }
                );

                let handled = self
                    .discovery
                    .handle_command_result(exit_code, &stdout, &stderr, &context)
                    || self
                        .config_io
                        .handle_command_result(exit_code, &stdout, &context);

                let is_ready_now = matches!(
                    self.discovery.phase,
                    crumbeez_lib::DiscoveryPhase::Ready { .. }
                );

                if !was_ready_before && is_ready_now {
                    self.handle_discovery_ready();
                    if let Some(ref backend) = self.config_io.config().llm.backend {
                        self.llm_requestor.set_backend(backend.clone());
                        info!(backend = %backend.display_name(), "LLM backend loaded from config");
                    }
                    if self.config_io.needs_onboarding() {
                        self.onboarding_active = true;
                    }
                }
                handled
            }
            Event::InterceptedKeyPress(key) => {
                let bytes = key_to_bytes(&key);
                write(bytes);
                let event = classify(&key);
                debug!(%event, "key event");
                self.log_event(event);
                true
            }
            Event::Key(key) => {
                if self.onboarding_active {
                    self.handle_onboarding_key(&key)
                } else {
                    let event = classify(&key);
                    debug!(%event, "key event (plugin focused)");
                    self.log_event(event);
                    true
                }
            }
            Event::TabUpdate(tabs) => {
                self.tab_names = tabs
                    .into_iter()
                    .filter(|t| !t.name.is_empty())
                    .map(|t| (t.position, t.name))
                    .collect();
                true
            }
            Event::PaneUpdate(manifest) => {
                self.handle_pane_update(manifest);
                true
            }
            #[cfg(feature = "pane-content-tracking")]
            Event::PaneRenderReport(report) => {
                let events = self.pane_registry.ingest_report(report);
                let had_events = !events.is_empty();
                for pane_event in events {
                    use crumbeez_lib::KeystrokeEvent;
                    debug!(pane_id = pane_event.pane_id, "pane output event");
                    self.event_log.append(
                        KeystrokeEvent::PaneOutput(pane_event),
                        Self::current_time_ms(),
                    );
                }
                had_events
            }
            Event::Timer(elapsed) => {
                debug!(elapsed_secs = ?elapsed, "Timer fired");

                // Check if we've been inactive for the threshold AND there's new activity since last summary
                let should_summarize = self.last_activity_time.is_some_and(|last| {
                    let inactive_duration = SystemTime::now().duration_since(last);
                    inactive_duration
                        .map(|d| d.as_secs_f64() >= INACTIVITY_TIMER_SECS)
                        .unwrap_or(false)
                }) && self.last_summary_time.is_none_or(|last_summary| {
                    self.last_activity_time
                        .is_some_and(|last_activity| last_activity > last_summary)
                });

                if should_summarize {
                    self.seal_pending_text();
                    let unconsumed = self.event_log.unconsumed_count();
                    if unconsumed > 0 {
                        if matches!(self.llm_requestor.backend(), LLMBackend::Ollama { .. })
                            && !self.llm_requestor.is_pending()
                        {
                            if let Some((events, event_count)) =
                                event_log_io::extract_events_for_llm(&mut self.event_log)
                            {
                                info!(event_count, "Requesting LLM summary on inactivity");
                                self.llm_requestor.request_leaf_summary(events, event_count);
                            }
                        } else if let Some(summary) =
                            event_log_io::generate_summary(&mut self.event_log)
                        {
                            self.pending_summaries.push(summary.clone());
                            if self.pending_summaries.len() > 10 {
                                self.pending_summaries.remove(0);
                            }
                            self.summary_io.save_summary_text(summary);
                        }
                        self.event_log.compact();
                        if let Ok(data) = self.event_log.serialize() {
                            self.event_log_io.save(data);
                        } else {
                            error!("Failed to serialize event log");
                        }
                        self.last_summary_time = Some(SystemTime::now());
                    }
                } else {
                    debug!("Skipping summary - no new activity since last summary");
                }
                self.reset_inactivity_timer();
                true
            }
            Event::FileSystemUpdate(_) => true,
            Event::WebRequestResult(status, headers, body, context) => {
                if let Some(result) = self
                    .llm_requestor
                    .handle_web_request_result(status, &headers, &body, &context)
                {
                    match result {
                        LLMResult::Summary(response) => {
                            if !response.digest.is_empty() {
                                let summary_text = if response.body.is_empty() {
                                    response.digest.clone()
                                } else {
                                    format!("{}\n\n{}", response.digest, response.body)
                                };
                                self.pending_summaries.push(summary_text.clone());
                                if self.pending_summaries.len() > 10 {
                                    self.pending_summaries.remove(0);
                                }
                                self.summary_io.save_summary_text(summary_text);
                                info!(digest = %response.digest, "LLM summary received");
                            }
                        }
                        LLMResult::Error(e) => {
                            error!(error = %e, "LLM request failed");
                        }
                    }
                }
                true
            }
            _ => false,
        };

        result
    }

    fn render(&mut self, rows: usize, cols: usize) {
        if self.onboarding_active {
            self.render_onboarding(rows, cols);
            return;
        }

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
    for &(_, c) in iter.by_ref() {
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
