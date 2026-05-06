mod config;
mod noop;
mod ollama;
mod openai;

pub use config::*;
pub use noop::NoOpSummarizer;
pub use ollama::OllamaBackend;
pub use openai::OpenAIBackend;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SummarizationRequest {
    pub events: Vec<String>,
    pub context: Option<String>,
    pub request_type: SummarizationType,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SummarizationType {
    Leaf,
    Section { child_digests: Vec<String> },
    Session { child_digests: Vec<String> },
    Grouping { actions: Vec<String> },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SummarizationResponse {
    pub digest: String,
    pub body: String,
    pub groups: Option<Vec<GroupRange>>,
    pub detail_requests: Option<Vec<usize>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupRange {
    pub start_idx: usize,
    pub end_idx: usize,
    pub label: String,
}

#[derive(Debug, Clone)]
pub enum BackendError {
    Network(String),
    Api(String),
    Parse(String),
    Config(String),
    NotConfigured,
}

impl std::fmt::Display for BackendError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Network(msg) => write!(f, "Network error: {}", msg),
            Self::Api(msg) => write!(f, "API error: {}", msg),
            Self::Parse(msg) => write!(f, "Parse error: {}", msg),
            Self::Config(msg) => write!(f, "Configuration error: {}", msg),
            Self::NotConfigured => write!(f, "LLM backend not configured"),
        }
    }
}

impl std::error::Error for BackendError {}

#[cfg(not(target_arch = "wasm32"))]
pub trait SummarizationBackend: Send + Sync {
    fn summarize(
        &self,
        request: SummarizationRequest,
    ) -> Result<SummarizationResponse, BackendError>;

    fn is_available(&self) -> bool;

    fn backend_name(&self) -> &'static str;
}

#[cfg(target_arch = "wasm32")]
pub trait SummarizationBackend {
    fn summarize(
        &self,
        request: SummarizationRequest,
    ) -> Result<SummarizationResponse, BackendError>;

    fn is_available(&self) -> bool;

    fn backend_name(&self) -> &'static str;
}

#[cfg(all(not(target_arch = "wasm32"), feature = "llm-http"))]
pub fn create_backend(config: &LLMBackend) -> Box<dyn SummarizationBackend> {
    match config {
        LLMBackend::NoLLM => Box::new(NoOpSummarizer::new()),
        LLMBackend::Ollama { endpoint, model } => {
            Box::new(OllamaBackend::new(endpoint.clone(), model.clone()))
        }
        LLMBackend::OpenAI { api_key_env, model } => {
            Box::new(OpenAIBackend::new(api_key_env.clone(), model.clone()))
        }
        LLMBackend::Anthropic { .. } => Box::new(NoOpSummarizer::new()),
    }
}

#[cfg(any(target_arch = "wasm32", not(feature = "llm-http")))]
pub fn create_backend(config: &LLMBackend) -> Box<dyn SummarizationBackend> {
    match config {
        LLMBackend::NoLLM => Box::new(NoOpSummarizer::new()),
        _ => Box::new(NoOpSummarizer::new()),
    }
}
