use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub type SummaryId = String;

fn generate_summary_id(session_id: &str, level: u8, seq: u32) -> SummaryId {
    format!("{}-L{}-{:03}", session_id, level, seq)
}

fn format_time(ms: u64) -> String {
    let total_seconds = ms / 1000;
    let hours = total_seconds / 3600;
    let minutes = (total_seconds % 3600) / 60;
    let seconds = total_seconds % 60;

    if hours > 0 {
        format!("{:02}:{:02}:{:02}", hours, minutes, seconds)
    } else {
        format!("{:02}:{:02}", minutes, seconds)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SummaryNode {
    pub id: SummaryId,
    pub level: u8,
    pub parent_id: Option<SummaryId>,
    pub children: Vec<SummaryId>,
    pub time_start_ms: u64,
    pub time_end_ms: u64,
    pub digest: String,
    pub body: String,
    pub event_count: u32,
    pub llm_generated: bool,
    pub generation: u16,
}

impl SummaryNode {
    pub fn new_leaf(
        id: SummaryId,
        time_start_ms: u64,
        time_end_ms: u64,
        digest: String,
        body: String,
        event_count: u32,
    ) -> Self {
        Self {
            id,
            level: 0,
            parent_id: None,
            children: Vec::new(),
            time_start_ms,
            time_end_ms,
            digest,
            body,
            event_count,
            llm_generated: false,
            generation: 0,
        }
    }

    pub fn format_time_range(&self) -> String {
        let start = format_time(self.time_start_ms);
        let end = format_time(self.time_end_ms);
        format!("{}–{}", start, end)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DisplayNode {
    pub id: SummaryId,
    pub level: u8,
    pub depth: usize,
    pub digest: String,
    pub body: String,
    pub time_start_ms: u64,
    pub time_end_ms: u64,
    pub has_children: bool,
    pub expanded: bool,
}

#[derive(Debug, Clone, Default)]
pub struct SummaryTree {
    nodes: HashMap<SummaryId, SummaryNode>,
    roots: Vec<SummaryId>,
    pending_leaves: Vec<SummaryId>,
    pending_sections: Vec<SummaryId>,
    session_id: String,
    sequence_counters: [u32; 5],
}

impl SummaryTree {
    pub fn new(session_id: String) -> Self {
        Self {
            nodes: HashMap::new(),
            roots: Vec::new(),
            pending_leaves: Vec::new(),
            pending_sections: Vec::new(),
            session_id,
            sequence_counters: [0; 5],
        }
    }

    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    pub fn get_node(&self, id: &SummaryId) -> Option<&SummaryNode> {
        self.nodes.get(id)
    }

    pub fn get_node_mut(&mut self, id: &SummaryId) -> Option<&mut SummaryNode> {
        self.nodes.get_mut(id)
    }

    pub fn pending_leaves(&self) -> &[SummaryId] {
        &self.pending_leaves
    }

    pub fn pending_sections(&self) -> &[SummaryId] {
        &self.pending_sections
    }

    pub fn roots(&self) -> &[SummaryId] {
        &self.roots
    }

    pub fn add_leaf(
        &mut self,
        time_start_ms: u64,
        time_end_ms: u64,
        digest: String,
        body: String,
        event_count: u32,
    ) -> SummaryId {
        let seq = self.sequence_counters[0];
        self.sequence_counters[0] += 1;

        let id = generate_summary_id(&self.session_id, 0, seq);
        let node = SummaryNode::new_leaf(
            id.clone(),
            time_start_ms,
            time_end_ms,
            digest,
            body,
            event_count,
        );

        self.nodes.insert(id.clone(), node);
        self.pending_leaves.push(id.clone());

        id
    }

    pub fn add_section(
        &mut self,
        time_start_ms: u64,
        time_end_ms: u64,
        digest: String,
        body: String,
        child_ids: Vec<SummaryId>,
        llm_generated: bool,
    ) -> SummaryId {
        let seq = self.sequence_counters[1];
        self.sequence_counters[1] += 1;

        let id = generate_summary_id(&self.session_id, 1, seq);
        let event_count: u32 = child_ids
            .iter()
            .filter_map(|cid| self.nodes.get(cid).map(|n| n.event_count))
            .sum();

        let node = SummaryNode {
            id: id.clone(),
            level: 1,
            parent_id: None,
            children: child_ids.clone(),
            time_start_ms,
            time_end_ms,
            digest,
            body,
            event_count,
            llm_generated,
            generation: 0,
        };

        for child_id in &child_ids {
            if let Some(child) = self.nodes.get_mut(child_id) {
                child.parent_id = Some(id.clone());
            }
        }

        self.nodes.insert(id.clone(), node);
        self.pending_sections.push(id.clone());

        id
    }

    pub fn add_session(
        &mut self,
        time_start_ms: u64,
        time_end_ms: u64,
        digest: String,
        body: String,
        child_ids: Vec<SummaryId>,
        llm_generated: bool,
    ) -> SummaryId {
        let seq = self.sequence_counters[2];
        self.sequence_counters[2] += 1;

        let id = generate_summary_id(&self.session_id, 2, seq);
        let event_count: u32 = child_ids
            .iter()
            .filter_map(|cid| self.nodes.get(cid).map(|n| n.event_count))
            .sum();

        let node = SummaryNode {
            id: id.clone(),
            level: 2,
            parent_id: None,
            children: child_ids.clone(),
            time_start_ms,
            time_end_ms,
            digest,
            body,
            event_count,
            llm_generated,
            generation: 0,
        };

        for child_id in &child_ids {
            if let Some(child) = self.nodes.get_mut(child_id) {
                child.parent_id = Some(id.clone());
            }
        }

        self.nodes.insert(id.clone(), node);
        self.roots.push(id.clone());

        id
    }

    pub fn should_rollup_leaves(&self, count_threshold: usize, _duration_ms: u64) -> bool {
        self.pending_leaves.len() >= count_threshold
    }

    pub fn should_rollup_sections(&self, count_threshold: usize, _duration_ms: u64) -> bool {
        self.pending_sections.len() >= count_threshold
    }

    pub fn rollup_leaves_noop(&mut self) -> Option<SummaryId> {
        if self.pending_leaves.is_empty() {
            return None;
        }

        let child_ids: Vec<SummaryId> = self.pending_leaves.drain(..).collect();

        let time_start_ms = child_ids
            .iter()
            .filter_map(|id| self.nodes.get(id).map(|n| n.time_start_ms))
            .min()
            .unwrap_or(0);

        let time_end_ms = child_ids
            .iter()
            .filter_map(|id| self.nodes.get(id).map(|n| n.time_end_ms))
            .max()
            .unwrap_or(0);

        let event_count: u32 = child_ids
            .iter()
            .filter_map(|id| self.nodes.get(id).map(|n| n.event_count))
            .sum();

        let digest = format!("Section ({} actions)", child_ids.len());
        let body = child_ids
            .iter()
            .enumerate()
            .filter_map(|(i, id)| {
                self.nodes
                    .get(id)
                    .map(|n| format!("{}. {}", i + 1, n.digest))
            })
            .collect::<Vec<_>>()
            .join("\n");

        let id = self.add_section(time_start_ms, time_end_ms, digest, body, child_ids, false);

        Some(id)
    }

    pub fn rollup_sections_noop(&mut self) -> Option<SummaryId> {
        if self.pending_sections.is_empty() {
            return None;
        }

        let child_ids: Vec<SummaryId> = self.pending_sections.drain(..).collect();

        let time_start_ms = child_ids
            .iter()
            .filter_map(|id| self.nodes.get(id).map(|n| n.time_start_ms))
            .min()
            .unwrap_or(0);

        let time_end_ms = child_ids
            .iter()
            .filter_map(|id| self.nodes.get(id).map(|n| n.time_end_ms))
            .max()
            .unwrap_or(0);

        let digest = format!("Session ({} sections)", child_ids.len());
        let body = child_ids
            .iter()
            .enumerate()
            .filter_map(|(i, id)| {
                self.nodes
                    .get(id)
                    .map(|n| format!("{}. {}", i + 1, n.digest))
            })
            .collect::<Vec<_>>()
            .join("\n");

        let id = self.add_session(time_start_ms, time_end_ms, digest, body, child_ids, false);

        Some(id)
    }

    pub fn finalize_session_noop(&mut self) -> Option<SummaryId> {
        if !self.pending_leaves.is_empty() {
            self.rollup_leaves_noop();
        }

        if !self.pending_sections.is_empty() {
            self.rollup_sections_noop();
        }

        self.roots.last().cloned()
    }

    pub fn get_children_digests(&self, parent_id: &SummaryId) -> Vec<String> {
        self.nodes
            .get(parent_id)
            .map(|parent| {
                parent
                    .children
                    .iter()
                    .filter_map(|cid| self.nodes.get(cid).map(|n| n.digest.clone()))
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn get_children_bodies(&self, parent_id: &SummaryId) -> Vec<String> {
        self.nodes
            .get(parent_id)
            .map(|parent| {
                parent
                    .children
                    .iter()
                    .filter_map(|cid| self.nodes.get(cid).map(|n| n.body.clone()))
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn flatten_for_display(&self, expanded: &HashMap<SummaryId, bool>) -> Vec<DisplayNode> {
        let mut result = Vec::new();

        fn visit(
            tree: &SummaryTree,
            node_id: &SummaryId,
            depth: usize,
            expanded: &HashMap<SummaryId, bool>,
            result: &mut Vec<DisplayNode>,
        ) {
            let Some(node) = tree.nodes.get(node_id) else {
                return;
            };

            let is_expanded = expanded
                .get(node_id)
                .copied()
                .unwrap_or_else(|| node.level >= 2);

            result.push(DisplayNode {
                id: node.id.clone(),
                level: node.level,
                depth,
                digest: node.digest.clone(),
                body: node.body.clone(),
                time_start_ms: node.time_start_ms,
                time_end_ms: node.time_end_ms,
                has_children: !node.children.is_empty(),
                expanded: is_expanded,
            });

            if is_expanded {
                for child_id in &node.children {
                    visit(tree, child_id, depth + 1, expanded, result);
                }
            }
        }

        for root_id in &self.roots {
            visit(self, root_id, 0, expanded, &mut result);
        }

        result
    }
}
