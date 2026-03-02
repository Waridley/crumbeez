use serde::{Deserialize, Serialize};

pub type SummaryId = String;

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
