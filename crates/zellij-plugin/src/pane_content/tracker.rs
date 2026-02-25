/// Per-pane tracking state.
///
/// `PaneTracker` wraps a [`Processor`] and attaches the pane-level metadata
/// (id, title, command) needed to build a [`PaneOutputEvent`].  It also owns
/// a hash of the last viewport to skip processing when nothing has changed.
/// Note: DefaultHasher is used for in-memory deduplication only; it does not
/// guarantee stable hashing across runs or platforms and must not be used
/// for persistence.
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use crumbeez_lib::{OutputTrigger, PaneOutputEvent};

use crate::pane_content::processor::Processor;

pub struct PaneTracker {
    pub pane_id: u32,
    pub pane_title: String,
    pub command: Option<String>,
    processor: Processor,
    /// Hash of the last viewport we processed; used to skip unchanged frames.
    last_viewport_hash: u64,
}

impl PaneTracker {
    pub fn new(pane_id: u32, pane_title: String, command: Option<String>) -> Self {
        Self {
            pane_id,
            pane_title,
            command,
            processor: Processor::new(),
            last_viewport_hash: 0,
        }
    }

    /// Update the pane's title and command (from `PaneUpdate` events).
    pub fn update_meta(&mut self, pane_title: String, command: Option<String>) {
        self.pane_title = pane_title;
        self.command = command;
    }

    /// Feed a new viewport snapshot.  Returns a [`PaneOutputEvent`] if the
    /// accumulated content is ready to emit given the provided trigger, or
    /// `None` if we should keep accumulating.
    pub fn ingest(
        &mut self,
        viewport: &[String],
        trigger: OutputTrigger,
    ) -> Option<PaneOutputEvent> {
        let h = hash_viewport(viewport);
        let is_pane_switch = matches!(trigger, OutputTrigger::PaneSwitch);
        let is_unchanged = h == self.last_viewport_hash && !is_pane_switch;

        if !is_unchanged && !viewport.is_empty() {
            self.last_viewport_hash = h;
            self.processor.ingest(viewport);
        }
        if is_pane_switch && !is_unchanged {
            self.last_viewport_hash = h;
        }

        if let Some((content, raw_lines, output_type)) = self.processor.flush(trigger) {
            Some(PaneOutputEvent {
                pane_id: self.pane_id,
                pane_title: self.pane_title.clone(),
                command: self.command.clone(),
                output_type,
                content,
                raw_lines,
                trigger,
            })
        } else {
            None
        }
    }

    /// Flush accumulated content without ingesting new data.
    ///
    /// Used when focus changes to emit any pending content from the old pane.
    pub fn flush_only(&mut self, trigger: OutputTrigger) -> Option<PaneOutputEvent> {
        if let Some((content, raw_lines, output_type)) = self.processor.flush(trigger) {
            Some(PaneOutputEvent {
                pane_id: self.pane_id,
                pane_title: self.pane_title.clone(),
                command: self.command.clone(),
                output_type,
                content,
                raw_lines,
                trigger,
            })
        } else {
            None
        }
    }
}

fn hash_viewport(viewport: &[String]) -> u64 {
    let mut h = DefaultHasher::new();
    viewport.hash(&mut h);
    h.finish()
}
