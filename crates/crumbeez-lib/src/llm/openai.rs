use crate::llm::{
    BackendError, LLMBackend, SummarizationBackend, SummarizationRequest, SummarizationResponse,
    SummarizationType,
};

pub struct OpenAIBackend {
    api_key_env: String,
    model: String,
}

impl OpenAIBackend {
    pub fn new(api_key_env: String, model: String) -> Self {
        Self { api_key_env, model }
    }

    pub fn from_config(config: &LLMBackend) -> Option<Self> {
        match config {
            LLMBackend::OpenAI { api_key_env, model } => {
                Some(Self::new(api_key_env.clone(), model.clone()))
            }
            _ => None,
        }
    }

    fn get_api_key(&self) -> Result<String, BackendError> {
        std::env::var(&self.api_key_env).map_err(|_| {
            BackendError::Config(format!("Environment variable {} not set", self.api_key_env))
        })
    }

    fn build_system_prompt(request_type: &SummarizationType) -> &'static str {
        match request_type {
            SummarizationType::Leaf => {
                "You are summarizing terminal activity. Write in the style of meeting minutes or a log: short imperative phrases, no prose, no filler. Each BODY entry on its own line, prefixed with the timestamp of the first relevant event. Respond with:\n\
                 DIGEST: <max 80 chars summary>\n\
                 BODY:\n\
                 <timestamped log>"
            }
            SummarizationType::Section { .. } | SummarizationType::Session { .. } => {
                "You are creating higher-level summaries. Write in the style of meeting minutes: terse bullet points, short imperative phrases, no prose, no filler. Respond with:\n\
                 DIGEST: <max 100 chars summary>\n\
                 BODY:\n\
                 <bullet-point log>\n\
                 If a child digest is vague, respond with: NEED_DETAIL: <number>"
            }
            SummarizationType::Grouping { .. } => {
                "Group activities into logical tasks. Respond with:\n\
                 GROUP <start>-<end>: <2-5 word label>\n\
                 for each group."
            }
        }
    }

    fn build_user_prompt(request: &SummarizationRequest) -> String {
        match &request.request_type {
            SummarizationType::Leaf => {
                format!(
                    "Summarize these actions:\n{}",
                    request
                        .events
                        .iter()
                        .enumerate()
                        .map(|(i, e)| format!("{}. {}", i + 1, e))
                        .collect::<Vec<_>>()
                        .join("\n")
                )
            }
            SummarizationType::Section { child_digests }
            | SummarizationType::Session { child_digests } => {
                format!(
                    "Summarize these task digests:\n{}",
                    child_digests
                        .iter()
                        .enumerate()
                        .map(|(i, d)| format!("{}. {}", i + 1, d))
                        .collect::<Vec<_>>()
                        .join("\n")
                )
            }
            SummarizationType::Grouping { actions } => {
                format!(
                    "Group these actions:\n{}",
                    actions
                        .iter()
                        .enumerate()
                        .map(|(i, a)| format!("{}. {}", i, a))
                        .collect::<Vec<_>>()
                        .join("\n")
                )
            }
        }
    }

    fn parse_response(
        text: &str,
        is_grouping: bool,
    ) -> Result<SummarizationResponse, BackendError> {
        if is_grouping {
            let groups = Self::parse_grouping_response(text)?;
            return Ok(SummarizationResponse {
                digest: String::new(),
                body: String::new(),
                groups: Some(groups),
                detail_requests: None,
            });
        }

        let lines: Vec<&str> = text.lines().collect();
        let mut digest = String::new();
        let mut body_lines = Vec::new();
        let mut in_body = false;
        let mut detail_requests = Vec::new();

        for line in lines {
            if line.starts_with("DIGEST:") {
                digest = line
                    .strip_prefix("DIGEST:")
                    .unwrap_or("")
                    .trim()
                    .to_string();
            } else if line.starts_with("BODY:") {
                in_body = true;
            } else if line.starts_with("NEED_DETAIL:") {
                if let Ok(idx) = line
                    .strip_prefix("NEED_DETAIL:")
                    .unwrap_or("")
                    .trim()
                    .parse::<usize>()
                {
                    detail_requests.push(idx.saturating_sub(1));
                }
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
}

#[cfg(all(not(target_arch = "wasm32"), feature = "llm-http"))]
impl SummarizationBackend for OpenAIBackend {
    fn summarize(
        &self,
        request: SummarizationRequest,
    ) -> Result<SummarizationResponse, BackendError> {
        let api_key = self.get_api_key()?;
        let is_grouping = matches!(request.request_type, SummarizationType::Grouping { .. });

        let system_prompt = Self::build_system_prompt(&request.request_type);
        let user_prompt = Self::build_user_prompt(&request);

        let body = serde_json::json!({
            "model": self.model,
            "messages": [
                {"role": "system", "content": system_prompt},
                {"role": "user", "content": user_prompt}
            ],
            "max_tokens": 2048,
            "temperature": 0.7
        });

        let response = reqwest::blocking::Client::new()
            .post("https://api.openai.com/v1/chat/completions")
            .header("Authorization", format!("Bearer {}", api_key))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .map_err(|e| BackendError::Network(e.to_string()))?;

        if !response.status().is_success() {
            return Err(BackendError::Api(format!("HTTP {}", response.status())));
        }

        let json: serde_json::Value = response
            .json()
            .map_err(|e| BackendError::Parse(e.to_string()))?;

        let text = json["choices"][0]["message"]["content"]
            .as_str()
            .ok_or_else(|| BackendError::Parse("No content in OpenAI response".to_string()))?;

        Self::parse_response(text, is_grouping)
    }

    fn is_available(&self) -> bool {
        std::env::var(&self.api_key_env).is_ok()
    }

    fn backend_name(&self) -> &'static str {
        "OpenAI"
    }
}

#[cfg(any(target_arch = "wasm32", not(feature = "llm-http")))]
impl SummarizationBackend for OpenAIBackend {
    fn summarize(
        &self,
        _request: SummarizationRequest,
    ) -> Result<SummarizationResponse, BackendError> {
        Err(BackendError::Config(
            "OpenAI backend requires llm-http feature".to_string(),
        ))
    }

    fn is_available(&self) -> bool {
        false
    }

    fn backend_name(&self) -> &'static str {
        "OpenAI"
    }
}
