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
    /// The title of the focused pane (e.g. "vim", "cargo run", "bash")
    focused_pane_title: Option<String>,
    /// The terminal command running in the focused pane
    focused_pane_command: Option<String>,
}

impl PaneRegistry {
    /// Notify the registry that focus has moved to `new_pane_id`.
    ///
    /// Immediately flushes the previously focused pane with a `PaneSwitch`
    /// trigger and returns any resulting event.
    pub fn on_focus_changed(
        &mut self,
        new_pane_id: u32,
        pane_title: Option<String>,
        pane_command: Option<String>,
    ) -> Option<PaneOutputEvent> {
        let old = self.focused_pane_id.replace(new_pane_id);
        self.focused_pane_title = pane_title.clone();
        self.focused_pane_command = pane_command.clone();

        // Update the new tracker's pane info
        if let Some(tracker) = self.trackers.get_mut(&new_pane_id) {
            tracker.set_pane_info(pane_title, pane_command);
        }

        if let Some(old_id) = old {
            if old_id != new_pane_id {
                if let Some(tracker) = self.trackers.get_mut(&old_id) {
                    return tracker.flush_only(OutputTrigger::PaneSwitch);
                }
            }
        }
        None
    }

    /// Returns the current pane title and command for the focused pane.
    pub fn focused_pane_info(&self) -> (Option<&str>, Option<&str>) {
        (
            self.focused_pane_title.as_deref(),
            self.focused_pane_command.as_deref(),
        )
    }

    /// Process a full `PaneRenderReport` payload.
    ///
    /// Only processes the currently focused pane to avoid flooding the event
    /// log with updates from background panes. Returns any `PaneOutputEvent`s
    /// that are ready to emit (may be empty).
    pub fn ingest_report(&mut self, report: HashMap<PaneId, PaneContents>) -> Vec<PaneOutputEvent> {
        let mut events = Vec::new();

        // Only process the focused pane; background panes are ignored until
        // they gain focus (at which point on_focus_changed() flushes the old).
        let focused = match self.focused_pane_id {
            Some(id) => id,
            None => return events,
        };

        for (pane_id, contents) in report {
            let id = match pane_id {
                PaneId::Terminal(n) => n,
                PaneId::Plugin(_) => continue,
            };

            if id != focused {
                continue;
            }

            let tracker = self
                .trackers
                .entry(id)
                .or_insert_with(|| PaneTracker::new(id));

            // Ensure tracker has pane info if this is a newly created tracker
            if !tracker.has_pane_info() && self.focused_pane_title.is_some() {
                tracker.set_pane_info(
                    self.focused_pane_title.clone(),
                    self.focused_pane_command.clone(),
                );
            }

            if let Some(event) = tracker.ingest(&contents.viewport, OutputTrigger::MaxAccumulated) {
                events.push(event);
            }
        }

        events
    }
}
