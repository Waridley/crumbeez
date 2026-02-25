/// Strategy for full-screen TUI applications (vim, htop, lazygit, etc.).
///
/// In TUI mode the viewport is essentially a 2D framebuffer — diffs of
/// individual lines are noisy and unhelpful.  Instead we store the full
/// viewport as a snapshot and emit it whenever the content has changed
/// meaningfully since the last snapshot, or when a forced trigger arrives.
use crumbeez_lib::{OutputTrigger, OutputType};

use super::{ContentStrategy, StrategyState};

pub struct TuiStrategy;

impl ContentStrategy for TuiStrategy {
    fn process(&self, viewport: &[String], state: &mut StrategyState) {
        state.total_raw_lines = viewport.len();

        let changed = match &state.last_snapshot {
            None => true,
            Some(prev) => prev != viewport,
        };

        state.last_snapshot = Some(viewport.to_vec());

        if changed {
            // Store the new snapshot as the pending content.
            state.pending_lines = viewport.to_vec();
        }
    }

    fn should_emit(&self, state: &StrategyState, trigger: OutputTrigger) -> bool {
        if state.pending_lines.is_empty() {
            return false;
        }
        // For TUI we only emit on explicit triggers — the viewport changes
        // constantly (cursor blink, status bars) and we don't want noise.
        matches!(
            trigger,
            OutputTrigger::PaneSwitch | OutputTrigger::CommandExit
        )
    }
}

/// Build the final content string for a TUI snapshot.
pub fn flush(state: &mut StrategyState) -> (String, usize, OutputType) {
    let raw = state.total_raw_lines;
    let content = state.pending_lines.join("\n");
    state.pending_lines.clear();
    state.total_raw_lines = 0;
    (content, raw, OutputType::Full)
}
