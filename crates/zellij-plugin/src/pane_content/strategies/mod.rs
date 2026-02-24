pub mod progress;
pub mod stdio;
pub mod tui;

use crumbeez_lib::{OutputTrigger, OutputType};

use crate::pane_content::detectors::PaneMode;

/// The result of processing a new viewport snapshot through a strategy.
pub struct ProcessResult {
    /// Content to emit (if any).  `None` means "accumulate more, not ready".
    pub content: Option<String>,
    /// How many raw viewport lines contributed to this content.
    pub raw_lines: usize,
    /// Classification of the content (meaningful only when `content` is `Some`).
    pub output_type: OutputType,
}

impl ProcessResult {
    pub fn pending() -> Self {
        Self {
            content: None,
            raw_lines: 0,
            output_type: OutputType::Diff,
        }
    }
}

/// Shared state threaded through a strategy across multiple viewport updates.
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

impl Default for StrategyState {
    fn default() -> Self {
        Self {
            pending_lines: Vec::new(),
            total_raw_lines: 0,
            last_progress_line: None,
            last_snapshot: None,
        }
    }
}

/// A content-processing strategy for a particular pane output mode.
pub trait ContentStrategy {
    /// Process a new viewport and update `state`.  Returns a `ProcessResult`
    /// indicating whether content is ready to emit.
    fn process(&self, viewport: &[String], state: &mut StrategyState) -> ProcessResult;

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
