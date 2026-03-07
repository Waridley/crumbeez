/// Default strategy for line-oriented stdio output.
///
/// Computes the diff between the current viewport and the previous snapshot,
/// deduplicates repeated lines (run-length encoding), and accumulates new
/// lines into a buffer.  Emits when the buffer exceeds the max-lines
/// threshold or when a forced trigger arrives (pane switch, command exit).
use crumbeez_lib::{OutputTrigger, OutputType};

use super::{ContentStrategy, StrategyState};

/// Emit when this many new lines have accumulated.
const MAX_PENDING_LINES: usize = 200;

pub struct StdioStrategy;

impl ContentStrategy for StdioStrategy {
    fn process(&self, viewport: &[String], state: &mut StrategyState) {
        // Compute new lines by finding the diff against last snapshot.
        let new_lines: Vec<String> = if let Some(ref last) = state.last_snapshot {
            // Build a set of hashes from the previous snapshot for O(n) diff.
            // We treat the viewport as an append-only stream: lines present in
            // the old snapshot that are also in the new one are considered
            // "already seen"; lines only in the new snapshot are new.
            //
            // This is a simplification: it handles the common case of a
            // scrolling terminal correctly but will over-report on cleared
            // screens.  The TUI detector should catch full-screen clears.
            let old_set: std::collections::HashSet<&str> =
                last.iter().map(String::as_str).collect();
            viewport
                .iter()
                .filter(|l| !old_set.contains(l.as_str()))
                .cloned()
                .collect()
        } else {
            // First capture: take the whole viewport.
            viewport.to_vec()
        };

        state.last_snapshot = Some(viewport.to_vec());
        state.total_raw_lines += new_lines.len();

        if new_lines.is_empty() {
            return;
        }

        // Dedup consecutive identical lines into "[N×] line" entries.
        let deduped = run_length_encode(new_lines);
        state.pending_lines.extend(deduped);
    }

    fn should_emit(&self, state: &StrategyState, trigger: OutputTrigger) -> bool {
        if state.pending_lines.is_empty() {
            return false;
        }
        match trigger {
            OutputTrigger::PaneSwitch | OutputTrigger::CommandExit => true,
            OutputTrigger::MaxAccumulated => state.pending_lines.len() >= MAX_PENDING_LINES,
        }
    }
}

/// Collapse runs of identical lines into `[N×] line` notation.
fn run_length_encode(lines: Vec<String>) -> Vec<String> {
    if lines.is_empty() {
        return lines;
    }
    let mut out = Vec::with_capacity(lines.len());
    let mut current = lines[0].clone();
    let mut count: usize = 1;

    for line in lines.into_iter().skip(1) {
        if line == current {
            count += 1;
        } else {
            out.push(format_rle(&current, count));
            current = line;
            count = 1;
        }
    }
    out.push(format_rle(&current, count));
    out
}

fn format_rle(line: &str, count: usize) -> String {
    if count == 1 {
        line.to_string()
    } else {
        format!("[{}×] {}", count, line)
    }
}

/// Build the final content string from pending lines.
/// Called by `PaneTracker` when it decides to emit.
pub fn flush(state: &mut StrategyState) -> (String, usize, OutputType) {
    let raw = state.total_raw_lines;
    let content = state.pending_lines.join("\n");
    state.pending_lines.clear();
    state.total_raw_lines = 0;
    (content, raw, OutputType::Diff)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_length_encode_single() {
        let result = run_length_encode(vec!["hello".into()]);
        assert_eq!(result, vec!["hello"]);
    }

    #[test]
    fn run_length_encode_run() {
        let lines = vec!["Building...".into(); 5];
        let result = run_length_encode(lines);
        assert_eq!(result, vec!["[5×] Building..."]);
    }

    #[test]
    fn run_length_encode_mixed() {
        let lines = vec![
            "a".into(),
            "a".into(),
            "b".into(),
            "c".into(),
            "c".into(),
            "c".into(),
        ];
        let result = run_length_encode(lines);
        assert_eq!(result, vec!["[2×] a", "b", "[3×] c"]);
    }
}
