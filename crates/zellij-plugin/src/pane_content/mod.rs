/// Pane content tracking pipeline.
///
/// This module is compiled only when the `pane-content-tracking` feature is
/// enabled, as it depends on the `PaneRenderReport` event type that was merged
/// into zellij after the 0.43.1 release.
///
/// # Pipeline overview
///
/// ```text
/// PaneRenderReport (HashMap<PaneId, PaneContents>)
///        ↓
///   PaneRegistry  (per-pane PaneTracker, keyed by pane id)
///        ↓ per-pane
///   PaneTracker   (hash check → skip if unchanged)
///        ↓
///   Processor     (detect mode → select strategy)
///        ↓
///   Strategy      (diff/compress: stdio | tui | progress)
///        ↓
///   flush()       (on boundary: pane-switch, command-exit, buffer-full)
///        ↓
///   PaneOutputEvent  (emitted into the EventLog)
/// ```
pub mod detectors;
pub mod processor;
pub mod strategies;
pub mod tracker;

use std::collections::HashMap;

use crumbeez_lib::{OutputTrigger, PaneOutputEvent};
use zellij_tile::prelude::{PaneContents, PaneId};

use tracker::PaneTracker;

/// Central registry of all tracked panes.
///
/// Holds one [`PaneTracker`] per terminal pane.  Call [`PaneRegistry::ingest`]
/// for every `PaneRenderReport` event and collect the returned events into the
/// `EventLog`.
#[derive(Default)]
pub struct PaneRegistry {
    trackers: HashMap<u32, PaneTracker>,
    /// The pane id that currently has keyboard focus (used to decide trigger
    /// type: a viewport update for the focused pane uses `MaxAccumulated`;
    /// if the focused pane just changed we use `PaneSwitch` for the old one).
    focused_pane_id: Option<u32>,
}

impl PaneRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Notify the registry that focus has moved to `new_pane_id`.
    ///
    /// Immediately flushes the previously-focused pane with a `PaneSwitch`
    /// trigger and returns any resulting event.
    pub fn on_focus_changed(&mut self, new_pane_id: u32) -> Option<PaneOutputEvent> {
        let old = self.focused_pane_id.replace(new_pane_id);
        if let Some(old_id) = old {
            if old_id != new_pane_id {
                if let Some(tracker) = self.trackers.get_mut(&old_id) {
                    return tracker.flush_only(OutputTrigger::PaneSwitch);
                }
            }
        }
        None
    }

    /// Process a full `PaneRenderReport` payload.
    ///
    /// Returns all `PaneOutputEvent`s that are ready to emit (may be empty).
    pub fn ingest_report(&mut self, report: HashMap<PaneId, PaneContents>) -> Vec<PaneOutputEvent> {
        let mut events = Vec::new();

        for (pane_id, contents) in report {
            let id = match pane_id {
                PaneId::Terminal(n) => n,
                PaneId::Plugin(_) => continue, // don't track plugin panes
            };

            let tracker = self
                .trackers
                .entry(id)
                .or_insert_with(|| PaneTracker::new(id, String::new(), None));

            // Use MaxAccumulated trigger for background updates; PaneSwitch is
            // handled separately via on_focus_changed().
            let trigger = OutputTrigger::MaxAccumulated;
            if let Some(event) = tracker.ingest(&contents.viewport, trigger) {
                events.push(event);
            }
        }

        events
    }

    /// Update pane metadata (title, command) from a `PaneUpdate` event.
    pub fn update_pane_meta(&mut self, pane_id: u32, title: String, command: Option<String>) {
        if let Some(tracker) = self.trackers.get_mut(&pane_id) {
            tracker.update_meta(title, command);
        }
        // If the pane isn't tracked yet we'll pick up the metadata on first
        // PaneRenderReport.
    }

    /// Remove a pane from the registry (called when the pane closes).
    pub fn remove_pane(&mut self, pane_id: u32) {
        self.trackers.remove(&pane_id);
    }
}
