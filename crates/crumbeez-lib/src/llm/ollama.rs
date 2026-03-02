use crate::llm::{
    BackendError, LLMBackend, SummarizationBackend, SummarizationRequest, SummarizationResponse,
    SummarizationType,
};

pub struct OllamaBackend {
    endpoint: String,
    model: String,
}

impl OllamaBackend {
    pub fn new(endpoint: String, model: String) -> Self {
        Self { endpoint, model }
    }

    pub fn from_config(config: &LLMBackend) -> Option<Self> {
        match config {
            LLMBackend::Ollama { endpoint, model } => {
                Some(Self::new(endpoint.clone(), model.clone()))
            }
            _ => None,
        }
    }

    fn build_leaf_prompt(events: &[String]) -> String {
        format!(
            r#"You are summarizing a user's recent terminal activity.

Actions:
{}

Produce:
1. DIGEST (max 80 chars): the essence of what happened.
2. BODY (2-5 sentences, Markdown): files touched, commands run, outcomes.

Format your response as:
DIGEST: <text>
BODY:
<markdown>"#,
            events
                .iter()
                .enumerate()
                .map(|(i, e)| format!("{}. {}", i + 1, e))
                .collect::<Vec<_>>()
                .join("\n")
        )
    }

    fn build_section_prompt(child_digests: &[String]) -> String {
        format!(
            r#"You are creating a higher-level summary of a work segment. Below are digests of logically distinct tasks:

{}

Produce DIGEST (max 100 chars) and BODY (3-8 sentences).

Format your response as:
DIGEST: <text>
BODY:
<markdown>

If any digest is too vague to summarize confidently, respond with: NEED_DETAIL: <number>"#,
            child_digests
                .iter()
                .enumerate()
                .map(|(i, d)| format!("{}. {}", i + 1, d))
                .collect::<Vec<_>>()
                .join("\n")
        )
    }

    fn build_grouping_prompt(actions: &[String]) -> String {
        format!(
            r#"You are grouping terminal activity into logically distinct tasks.
A "logically distinct" task is where a human would say "that was one thing, now I'm on another."

Group these actions. For each group, output:
GROUP <start_idx>-<end_idx>: <2-5 word task label>

Example:
GROUP 0-12: Configure user authentication
GROUP 13-18: Write unit tests
GROUP 19-25: Update documentation

Now process these actions:
{}"#,
            actions
                .iter()
                .enumerate()
                .map(|(i, a)| format!("{}. {}", i, a))
                .collect::<Vec<_>>()
                .join("\n")
        )
    }

    fn parse_response(text: &str) -> Result<(String, String), BackendError> {
        let lines: Vec<&str> = text.lines().collect();
        let mut digest = String::new();
        let mut body_lines = Vec::new();
        let mut in_body = false;

        for line in lines {
            if line.starts_with("DIGEST:") {
                digest = line
                    .strip_prefix("DIGEST:")
                    .unwrap_or("")
                    .trim()
                    .to_string();
            } else if line.starts_with("BODY:") {
                in_body = true;
            } else if in_body {
                body_lines.push(line);
            }
        }

        let body = body_lines.join("\n").trim().to_string();

        if digest.is_empty() {
            return Err(BackendError::Parse(
                "No DIGEST found in response".to_string(),
            ));
        }

        Ok((digest, body))
    }

    fn parse_grouping_response(text: &str) -> Result<Vec<crate::llm::GroupRange>, BackendError> {
        let mut groups = Vec::new();

        for line in text.lines() {
            let line = line.trim();
            if line.starts_with("GROUP ") {
                let rest = line.strip_prefix("GROUP ").unwrap_or("");
                if let Some((range, label)) = rest.split_once(':') {
                    let label = label.trim().to_string();
                    if let Some((start_str, end_str)) = range.split_once('-') {
                        if let (Ok(start), Ok(end)) = (
                            start_str.trim().parse::<usize>(),
                            end_str.trim().parse::<usize>(),
                        ) {
                            groups.push(crate::llm::GroupRange {
                                start_idx: start,
                                end_idx: end,
                                label,
                            });
                        }
                    }
                }
            }
        }

        Ok(groups)
    }

    fn parse_detail_requests(text: &str) -> Vec<usize> {
        let mut requests = Vec::new();
        for line in text.lines() {
            if line.starts_with("NEED_DETAIL:") {
                if let Ok(idx) = line
                    .strip_prefix("NEED_DETAIL:")
                    .unwrap_or("")
                    .trim()
                    .parse::<usize>()
                {
                    requests.push(idx.saturating_sub(1));
                }
            }
        }
        requests
    }
}

#[cfg(all(not(target_arch = "wasm32"), feature = "llm-http"))]
impl SummarizationBackend for OllamaBackend {
    fn summarize(
        &self,
        request: SummarizationRequest,
    ) -> Result<SummarizationResponse, BackendError> {
        let prompt = match &request.request_type {
            SummarizationType::Leaf => Self::build_leaf_prompt(&request.events),
            SummarizationType::Section { child_digests } => {
                Self::build_section_prompt(child_digests)
            }
            SummarizationType::Session { child_digests } => {
                Self::build_section_prompt(child_digests)
            }
            SummarizationType::Grouping { actions } => Self::build_grouping_prompt(actions),
        };

        let url = format!("{}/api/generate", self.endpoint);

        let body = serde_json::json!({
            "model": self.model,
            "prompt": prompt,
            "stream": false,
            "options": {
                "temperature": 0.7,
                "num_predict": 2048
            }
        });

        let response = reqwest::blocking::Client::new()
            .post(&url)
            .json(&body)
            .send()
            .map_err(|e| BackendError::Network(e.to_string()))?;

        if !response.status().is_success() {
            return Err(BackendError::Api(format!("HTTP {}", response.status())));
        }

        let json: serde_json::Value = response
            .json()
            .map_err(|e| BackendError::Parse(e.to_string()))?;

        let text = json["response"]
            .as_str()
            .ok_or_else(|| BackendError::Parse("No response field in Ollama output".to_string()))?;

        match &request.request_type {
            SummarizationType::Grouping { .. } => {
                let groups = Self::parse_grouping_response(text)?;
                Ok(SummarizationResponse {
                    digest: String::new(),
                    body: String::new(),
                    groups: Some(groups),
                    detail_requests: None,
                })
            }
            _ => {
                let (digest, body) = Self::parse_response(text)?;
                let detail_requests = Self::parse_detail_requests(text);
                Ok(SummarizationResponse {
                    digest,
                    body,
                    groups: None,
                    detail_requests: if detail_requests.is_empty() {
                        None
                    } else {
                        Some(detail_requests)
                    },
                })
            }
        }
    }

    fn is_available(&self) -> bool {
        true
    }

    fn backend_name(&self) -> &'static str {
        "Ollama"
    }
}

#[cfg(any(target_arch = "wasm32", not(feature = "llm-http")))]
impl SummarizationBackend for OllamaBackend {
    fn summarize(
        &self,
        _request: SummarizationRequest,
    ) -> Result<SummarizationResponse, BackendError> {
        Err(BackendError::Config(
            "Ollama backend requires llm-http feature".to_string(),
        ))
    }

    fn is_available(&self) -> bool {
        false
    }

    fn backend_name(&self) -> &'static str {
        "Ollama"
    }
}
