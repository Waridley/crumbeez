use crumbeez_lib::{
    BackendError, GroupRange, LLMBackend, SummarizationRequest, SummarizationResponse,
    SummarizationType,
};
use std::collections::BTreeMap;
use tracing::{debug, error, info};
use zellij_tile::prelude::{web_request, HttpVerb};

const CTX_ACTION: &str = "crumbeez_llm_action";

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum LLMRequestAction {
    LeafSummary { event_count: u32 },
    SectionSummary,
    Grouping,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct LLMRequestContext {
    action: LLMRequestAction,
}

pub struct LLMRequestor {
    backend: LLMBackend,
    pending_request: bool,
}

impl LLMRequestor {
    pub fn new(backend: LLMBackend) -> Self {
        Self {
            backend,
            pending_request: false,
        }
    }

    pub fn backend(&self) -> &LLMBackend {
        &self.backend
    }

    pub fn set_backend(&mut self, backend: LLMBackend) {
        self.backend = backend;
    }

    pub fn is_pending(&self) -> bool {
        self.pending_request
    }

    pub fn request_leaf_summary(&mut self, events: Vec<String>, event_count: u32) -> bool {
        let (endpoint, model) = match &self.backend {
            LLMBackend::Ollama { endpoint, model } => (endpoint, model),
            _ => {
                debug!("LLM backend not configured or not supported for web requests");
                return false;
            }
        };

        if self.pending_request {
            debug!("LLM request already pending");
            return false;
        }

        let prompt = build_leaf_prompt(&events);
        let body = serde_json::json!({
            "model": model,
            "prompt": prompt,
            "stream": false,
            "options": {
                "temperature": 0.7,
                "num_predict": 2048
            }
        });

        let url = format!("{}/api/generate", endpoint);
        let mut headers = BTreeMap::new();
        headers.insert("Content-Type".to_string(), "application/json".to_string());

        let mut context = BTreeMap::new();
        context.insert(
            CTX_ACTION.to_string(),
            serde_json::to_string(&LLMRequestContext {
                action: LLMRequestAction::LeafSummary { event_count },
            })
            .expect("context serialization is infallible"),
        );

        info!(url = %url, "Requesting leaf summary from Ollama");
        web_request(
            url,
            HttpVerb::Post,
            headers,
            body.to_string().into_bytes(),
            context,
        );
        self.pending_request = true;
        true
    }

    pub fn request_section_summary(&mut self, child_digests: Vec<String>) -> bool {
        let (endpoint, model) = match &self.backend {
            LLMBackend::Ollama { endpoint, model } => (endpoint, model),
            _ => return false,
        };

        if self.pending_request {
            return false;
        }

        let prompt = build_section_prompt(&child_digests);
        let body = serde_json::json!({
            "model": model,
            "prompt": prompt,
            "stream": false,
            "options": {
                "temperature": 0.7,
                "num_predict": 2048
            }
        });

        let url = format!("{}/api/generate", endpoint);
        let mut headers = BTreeMap::new();
        headers.insert("Content-Type".to_string(), "application/json".to_string());

        let mut context = BTreeMap::new();
        context.insert(
            CTX_ACTION.to_string(),
            serde_json::to_string(&LLMRequestContext {
                action: LLMRequestAction::SectionSummary,
            })
            .expect("context serialization is infallible"),
        );

        info!(url = %url, "Requesting section summary from Ollama");
        web_request(
            url,
            HttpVerb::Post,
            headers,
            body.to_string().into_bytes(),
            context,
        );
        self.pending_request = true;
        true
    }

    pub fn request_grouping(&mut self, actions: Vec<String>) -> bool {
        let (endpoint, model) = match &self.backend {
            LLMBackend::Ollama { endpoint, model } => (endpoint, model),
            _ => return false,
        };

        if self.pending_request {
            return false;
        }

        let prompt = build_grouping_prompt(&actions);
        let body = serde_json::json!({
            "model": model,
            "prompt": prompt,
            "stream": false,
            "options": {
                "temperature": 0.7,
                "num_predict": 2048
            }
        });

        let url = format!("{}/api/generate", endpoint);
        let mut headers = BTreeMap::new();
        headers.insert("Content-Type".to_string(), "application/json".to_string());

        let mut context = BTreeMap::new();
        context.insert(
            CTX_ACTION.to_string(),
            serde_json::to_string(&LLMRequestContext {
                action: LLMRequestAction::Grouping,
            })
            .expect("context serialization is infallible"),
        );

        info!(url = %url, "Requesting grouping from Ollama");
        web_request(
            url,
            HttpVerb::Post,
            headers,
            body.to_string().into_bytes(),
            context,
        );
        self.pending_request = true;
        true
    }

    pub fn handle_web_request_result(
        &mut self,
        status_code: u16,
        _headers: &BTreeMap<String, String>,
        body: &[u8],
        context: &BTreeMap<String, String>,
    ) -> Option<LLMResult> {
        self.pending_request = false;

        let ctx: LLMRequestContext = match context.get(CTX_ACTION) {
            Some(s) => serde_json::from_str(s).ok()?,
            None => return None,
        };

        if status_code != 200 {
            error!(status_code, "LLM request failed");
            return Some(LLMResult::Error(format!("HTTP {}", status_code)));
        }

        let body_str = String::from_utf8_lossy(body);
        let json: serde_json::Value = match serde_json::from_str(&body_str) {
            Ok(j) => j,
            Err(e) => {
                error!(error = %e, "Failed to parse LLM response JSON");
                return Some(LLMResult::Error(format!("JSON parse error: {}", e)));
            }
        };

        let response_text = json["response"]
            .as_str()
            .ok_or_else(|| "No response field in Ollama output")
            .ok()?;

        match ctx.action {
            LLMRequestAction::LeafSummary { event_count: _ } | LLMRequestAction::SectionSummary => {
                match parse_response(response_text) {
                    Ok((digest, body)) => Some(LLMResult::Summary(SummarizationResponse {
                        digest,
                        body,
                        groups: None,
                        detail_requests: None,
                    })),
                    Err(e) => Some(LLMResult::Error(e.to_string())),
                }
            }
            LLMRequestAction::Grouping => match parse_grouping_response(response_text) {
                Ok(groups) => Some(LLMResult::Summary(SummarizationResponse {
                    digest: String::new(),
                    body: String::new(),
                    groups: Some(groups),
                    detail_requests: None,
                })),
                Err(e) => Some(LLMResult::Error(e.to_string())),
            },
        }
    }
}

#[derive(Debug, Clone)]
pub enum LLMResult {
    Summary(SummarizationResponse),
    Error(String),
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
<markdown>"#,
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

For each group, output one line:
GROUP <start>-<end>: <label>

Where start and end are 0-indexed, and label is a 2-5 word summary.

Example input:
0. Opened terminal, navigated to /home/user/project
1. Ran vim config.toml
2. Edited database settings
3. Saved and quit vim
4. Ran cargo build
5. Fixed compilation error in src/lib.rs
6. Re-ran cargo build, success

Example output:
GROUP 0-0: Project navigation
GROUP 1-3: Config editing
GROUP 4-6: Build and fix

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

fn parse_grouping_response(text: &str) -> Result<Vec<GroupRange>, BackendError> {
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
                        groups.push(GroupRange {
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

impl Default for LLMRequestor {
    fn default() -> Self {
        Self::new(LLMBackend::NoLLM)
    }
}
