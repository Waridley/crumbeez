use std::time::{SystemTime, UNIX_EPOCH};

use crumbeez_lib::{EventLog, LogEntry, NoOpSummarizer, Summary, SummaryId, SummaryTree};
use tracing::{debug, info};

pub const DEFAULT_LEAF_ROLLUP_COUNT: usize = 5;
pub const DEFAULT_LEAF_ROLLUP_DURATION_MS: u64 = 30 * 60 * 1000;
pub const DEFAULT_SECTION_ROLLUP_COUNT: usize = 3;
pub const DEFAULT_SECTION_ROLLUP_DURATION_MS: u64 = 60 * 60 * 1000;

#[derive(Debug, Clone, PartialEq)]
pub enum RollupPhase {
    Idle,
    RollingUpLeaves,
    RollingUpSections,
    Saving,
}

pub struct SummarizationOrchestrator {
    tree: SummaryTree,
    rollup_phase: RollupPhase,
    leaf_rollup_count_threshold: usize,
    leaf_rollup_duration_ms: u64,
    section_rollup_count_threshold: usize,
    section_rollup_duration_ms: u64,
    session_start_ms: u64,
}

impl SummarizationOrchestrator {
    pub fn new(session_id: String, session_start_ms: u64) -> Self {
        Self {
            tree: SummaryTree::new(session_id),
            rollup_phase: RollupPhase::Idle,
            leaf_rollup_count_threshold: DEFAULT_LEAF_ROLLUP_COUNT,
            leaf_rollup_duration_ms: DEFAULT_LEAF_ROLLUP_DURATION_MS,
            section_rollup_count_threshold: DEFAULT_SECTION_ROLLUP_COUNT,
            section_rollup_duration_ms: DEFAULT_SECTION_ROLLUP_DURATION_MS,
            session_start_ms,
        }
    }

    pub fn tree(&self) -> &SummaryTree {
        &self.tree
    }

    pub fn tree_mut(&mut self) -> &mut SummaryTree {
        &mut self.tree
    }

    pub fn phase(&self) -> &RollupPhase {
        &self.rollup_phase
    }

    pub fn session_id(&self) -> &str {
        self.tree.session_id()
    }

    pub fn add_leaf_from_events(
        &mut self,
        events: Vec<LogEntry>,
        current_time_ms: u64,
    ) -> Option<SummaryId> {
        if events.is_empty() {
            return None;
        }

        let time_start_ms = events
            .first()
            .map(|e| e.timestamp_ms)
            .unwrap_or(current_time_ms);
        let time_end_ms = current_time_ms;

        let summary = Summary::from_events(events.into_iter());
        let event_count = summary.events_consumed as u32;

        let (digest, body) =
            NoOpSummarizer::generate_leaf_digest_and_body(event_count, &summary.event_types);

        let leaf_id = self
            .tree
            .add_leaf(time_start_ms, time_end_ms, digest, body, event_count);

        debug!(leaf_id = %leaf_id, "Added leaf summary");

        self.check_and_rollup_leaves();

        Some(leaf_id)
    }

    pub fn check_and_rollup_leaves(&mut self) {
        if self.tree.should_rollup_leaves(
            self.leaf_rollup_count_threshold,
            self.leaf_rollup_duration_ms,
        ) {
            info!("Rolling up leaves into section");
            self.rollup_phase = RollupPhase::RollingUpLeaves;

            if let Some(section_id) = self.tree.rollup_leaves_noop() {
                debug!(section_id = %section_id, "Leaves rolled up into section");

                if self.tree.should_rollup_sections(
                    self.section_rollup_count_threshold,
                    self.section_rollup_duration_ms,
                ) {
                    info!("Rolling up sections into session");
                    self.rollup_phase = RollupPhase::RollingUpSections;

                    if let Some(session_id) = self.tree.rollup_sections_noop() {
                        debug!(session_id = %session_id, "Sections rolled up into session");
                    }
                }
            }

            self.rollup_phase = RollupPhase::Idle;
        }
    }

    pub fn finalize_session(&mut self) -> Option<SummaryId> {
        info!("Finalizing session");
        self.rollup_phase = RollupPhase::Saving;

        let result = self.tree.finalize_session_noop();

        self.rollup_phase = RollupPhase::Idle;

        result
    }

    pub fn pending_leaves_count(&self) -> usize {
        self.tree.pending_leaves().len()
    }

    pub fn pending_sections_count(&self) -> usize {
        self.tree.pending_sections().len()
    }

    pub fn get_pending_leaf_digests(&self) -> Vec<String> {
        self.tree
            .pending_leaves()
            .iter()
            .filter_map(|id| self.tree.get_node(id).map(|n| n.digest.clone()))
            .collect()
    }
}

pub fn generate_leaf_summary(
    event_log: &mut EventLog,
    _current_time_ms: u64,
) -> Option<(Vec<LogEntry>, u32)> {
    let unconsumed: Vec<_> = event_log.unconsumed().cloned().collect();

    if unconsumed.is_empty() {
        return None;
    }

    let count = unconsumed.len() as u32;
    event_log.consume(count as usize);

    Some((unconsumed, count))
}

pub fn get_current_time_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}
