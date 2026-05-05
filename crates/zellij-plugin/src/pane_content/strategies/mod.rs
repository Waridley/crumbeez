pub mod progress;
pub mod stdio;
pub mod tui;

use crumbeez_lib::OutputTrigger;

use crate::pane_content::detectors::PaneMode;

/// Shared state threaded through a strategy across multiple viewport updates.
#[derive(Default)]
pub struct StrategyState {
    /// Lines accumulated since the last emission.
    pub pending_lines: Vec<String>,
    /// Total raw lines seen (for transparency in the log).
    pub total_raw_lines: usize,
    /// For progress mode: the most recent in-place update line.
    pub last_progress_line: Option<String>,
    /// For TUI mode: the full previous viewport (for change detection).
    pub last_snapshot: Option<Vec<String>>,
}

/// A content-processing strategy for a particular pane output mode.
pub trait ContentStrategy {
    /// Process a new viewport and update `state`.
    fn process(&self, viewport: &[String], state: &mut StrategyState);

    /// Returns `true` if accumulated content should be emitted now (e.g.
    /// buffer full, prompt detected) independent of a viewport change.
    fn should_emit(&self, state: &StrategyState, trigger: OutputTrigger) -> bool;
}

/// Select the right strategy for the detected pane mode.
pub fn for_mode(mode: PaneMode) -> Box<dyn ContentStrategy> {
    match mode {
        PaneMode::Stdio => Box::new(stdio::StdioStrategy),
        PaneMode::Tui => Box::new(tui::TuiStrategy),
        PaneMode::Progress => Box::new(progress::ProgressStrategy),
    }
}
