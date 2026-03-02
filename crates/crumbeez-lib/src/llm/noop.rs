use crate::llm::{BackendError, SummarizationBackend, SummarizationRequest, SummarizationResponse};

pub struct NoOpSummarizer;

impl NoOpSummarizer {
    pub fn new() -> Self {
        Self
    }

    pub fn generate_leaf_digest_and_body(
        event_count: u32,
        event_types: &std::collections::HashMap<String, usize>,
    ) -> (String, String) {
        let mut type_parts: Vec<String> = event_types
            .iter()
            .map(|(k, v)| format!("{}×{}", k, v))
            .collect();
        type_parts.sort();

        let digest = if type_parts.is_empty() {
            format!("{} events", event_count)
        } else {
            format!("{} events: {}", event_count, type_parts.join(", "))
        };

        let body = format!(
            "Activity summary:\n{}\n\nTotal events: {}",
            type_parts.join("\n"),
            event_count
        );

        (digest, body)
    }

    fn generate_section_digest_and_body(child_digests: &[String]) -> (String, String) {
        let count = child_digests.len();
        let digest = format!("Section ({} items)", count);

        let body = child_digests
            .iter()
            .enumerate()
            .map(|(i, d)| format!("{}. {}", i + 1, d))
            .collect::<Vec<_>>()
            .join("\n");

        (digest, body)
    }
}

impl Default for NoOpSummarizer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl SummarizationBackend for NoOpSummarizer {
    fn summarize(
        &self,
        request: SummarizationRequest,
    ) -> Result<SummarizationResponse, BackendError> {
        use crate::llm::SummarizationType;

        let (digest, body) = match request.request_type {
            SummarizationType::Leaf => {
                let event_count = request.events.len() as u32;
                let mut event_types = std::collections::HashMap::new();
                for event in &request.events {
                    let event_type = event.split(':').next().unwrap_or("Unknown");
                    *event_types.entry(event_type.to_string()).or_insert(0) += 1;
                }
                Self::generate_leaf_digest_and_body(event_count, &event_types)
            }
            SummarizationType::Section { ref child_digests }
            | SummarizationType::Session { ref child_digests } => {
                Self::generate_section_digest_and_body(child_digests)
            }
            SummarizationType::Grouping { ref actions } => {
                let count = actions.len();
                let digest = format!("{} actions", count);

                let groups = actions
                    .iter()
                    .enumerate()
                    .map(|(i, _)| crate::llm::GroupRange {
                        start_idx: i,
                        end_idx: i,
                        label: format!("Action {}", i + 1),
                    })
                    .collect();

                return Ok(SummarizationResponse {
                    digest,
                    body: String::new(),
                    groups: Some(groups),
                    detail_requests: None,
                });
            }
        };

        Ok(SummarizationResponse {
            digest,
            body,
            groups: None,
            detail_requests: None,
        })
    }

    fn is_available(&self) -> bool {
        true
    }

    fn backend_name(&self) -> &'static str {
        "NoOp"
    }
}

#[cfg(target_arch = "wasm32")]
impl SummarizationBackend for NoOpSummarizer {
    fn summarize(
        &self,
        request: SummarizationRequest,
    ) -> Result<SummarizationResponse, BackendError> {
        use crate::llm::SummarizationType;

        let (digest, body) = match request.request_type {
            SummarizationType::Leaf => {
                let event_count = request.events.len() as u32;
                let mut event_types = std::collections::HashMap::new();
                for event in &request.events {
                    let event_type = event.split(':').next().unwrap_or("Unknown");
                    *event_types.entry(event_type.to_string()).or_insert(0) += 1;
                }
                Self::generate_leaf_digest_and_body(event_count, &event_types)
            }
            SummarizationType::Section { ref child_digests }
            | SummarizationType::Session { ref child_digests } => {
                Self::generate_section_digest_and_body(child_digests)
            }
            SummarizationType::Grouping { ref actions } => {
                let count = actions.len();
                let digest = format!("{} actions", count);

                let groups = actions
                    .iter()
                    .enumerate()
                    .map(|(i, _)| crate::llm::GroupRange {
                        start_idx: i,
                        end_idx: i,
                        label: format!("Action {}", i + 1),
                    })
                    .collect();

                return Ok(SummarizationResponse {
                    digest,
                    body: String::new(),
                    groups: Some(groups),
                    detail_requests: None,
                });
            }
        };

        Ok(SummarizationResponse {
            digest,
            body,
            groups: None,
            detail_requests: None,
        })
    }

    fn is_available(&self) -> bool {
        true
    }

    fn backend_name(&self) -> &'static str {
        "NoOp"
    }
}
