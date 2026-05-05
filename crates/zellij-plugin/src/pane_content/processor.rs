/// Content diffing, mode detection, and flush coordination.
///
/// `Processor` owns the per-pane [`StrategyState`] and the current
/// [`ContentStrategy`].  It receives raw viewport snapshots, delegates
/// processing to the active strategy, and exposes a `flush` method that
/// produces the final `(content, raw_lines, OutputType)` tuple when a
/// trigger fires.
///
/// ANSI codes are preserved in the content for LLM context; mode detection
/// uses control codes to classify programs (TUI vs stdio vs progress).
use crumbeez_lib::{OutputTrigger, OutputType};

use crate::pane_content::{
    detectors::{detect_mode, PaneMode},
    strategies::{self, ContentStrategy, StrategyState},
};

pub struct Processor {
    strategy: Box<dyn ContentStrategy>,
    state: StrategyState,
    current_mode: PaneMode,
}

impl Processor {
    pub fn new() -> Self {
        // Start in Stdio mode; re-detect on first real content.
        Self {
            strategy: strategies::for_mode(PaneMode::Stdio),
            state: StrategyState::default(),
            current_mode: PaneMode::Stdio,
        }
    }

    /// Feed a new viewport snapshot into the processor.
    ///
    /// Detects the pane mode from the raw content (joining lines for
    /// analysis), potentially switching strategies, then delegates to the
    /// active strategy.
    pub fn ingest(&mut self, viewport: &[String]) {
        if viewport.is_empty() {
            return;
        }

        // Detect mode from the full viewport content (ANSI codes can appear anywhere).
        let sample = viewport.join("\n");
        let mode = detect_mode(&sample);

        if mode != self.current_mode {
            // Strategy changed: reset state so the new strategy starts fresh.
            self.current_mode = mode;
            self.strategy = strategies::for_mode(mode);
            self.state = StrategyState::default();
        }

        // Pass viewport lines to strategy (ANSI preserved for LLM context).
        self.strategy.process(viewport, &mut self.state);
    }

    /// Returns `true` if the strategy thinks content should be emitted now.
    pub fn should_emit(&self, trigger: OutputTrigger) -> bool {
        self.strategy.should_emit(&self.state, trigger)
    }

    /// Consume accumulated content and return `(content, raw_lines, OutputType)`.
    /// Returns `None` if there is nothing to emit.
    pub fn flush(&mut self, trigger: OutputTrigger) -> Option<(String, usize, OutputType)> {
        if !self.should_emit(trigger) {
            return None;
        }
        let result = match self.current_mode {
            PaneMode::Stdio => {
                let (content, raw, ot) = strategies::stdio::flush(&mut self.state);
                Some((content, raw, ot))
            }
            PaneMode::Tui => {
                let (content, raw, ot) = strategies::tui::flush(&mut self.state);
                Some((content, raw, ot))
            }
            PaneMode::Progress => {
                let (content, raw, ot) = strategies::progress::flush(&mut self.state);
                Some((content, raw, ot))
            }
        };
        result.filter(|(content, _, _)| !content.is_empty())
    }
}
