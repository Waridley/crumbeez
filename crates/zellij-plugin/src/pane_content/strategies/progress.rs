/// Strategy for progress-bar / spinner output.
///
/// Progress indicators use `\r` to overwrite the current line in-place.
/// Zellij delivers the *rendered* viewport (the final visual state after all
/// `\r` rewrites), so what we see is the latest in-place update.
///
/// Strategy: track the last rendered progress line and emit only the final
/// state when a newline appears (indicating the progress run ended) or when
/// a forced trigger arrives.
use crumbeez_lib::{OutputTrigger, OutputType};

use super::{ContentStrategy, StrategyState};

pub struct ProgressStrategy;

impl ContentStrategy for ProgressStrategy {
    fn process(&self, viewport: &[String], state: &mut StrategyState) {
        state.total_raw_lines = viewport.len();

        // The last non-empty line is the current progress state.
        let progress_line = viewport
            .iter()
            .rev()
            .find(|l| !l.trim().is_empty())
            .cloned();

        if let Some(line) = progress_line {
            state.last_progress_line = Some(line.clone());
            state.pending_lines = vec![line];
        }

        state.last_snapshot = Some(viewport.to_vec());
    }

    fn should_emit(&self, state: &StrategyState, trigger: OutputTrigger) -> bool {
        if state.last_progress_line.is_none() {
            return false;
        }
        matches!(
            trigger,
            OutputTrigger::PaneSwitch | OutputTrigger::CommandExit
        )
    }
}

/// Build the final content string: just the last progress line.
pub fn flush(state: &mut StrategyState) -> (String, usize, OutputType) {
    let raw = state.total_raw_lines;
    let content = state.last_progress_line.take().unwrap_or_default();
    state.pending_lines.clear();
    state.total_raw_lines = 0;
    (content, raw, OutputType::ProgressFinal)
}
