use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum LLMBackend {
    NoLLM,
    Ollama { endpoint: String, model: String },
    OpenAI { api_key_env: String, model: String },
    Anthropic { api_key_env: String, model: String },
}

impl Default for LLMBackend {
    fn default() -> Self {
        Self::NoLLM
    }
}

impl LLMBackend {
    pub fn is_configured(&self) -> bool {
        !matches!(self, Self::NoLLM)
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            Self::NoLLM => "No LLM (event logging only)",
            Self::Ollama { .. } => "Ollama (local)",
            Self::OpenAI { .. } => "OpenAI (cloud)",
            Self::Anthropic { .. } => "Anthropic (cloud)",
        }
    }

    pub fn default_ollama() -> Self {
        Self::Ollama {
            endpoint: "http://localhost:11434".to_string(),
            model: "qwen2.5-coder:3b".to_string(),
        }
    }

    pub fn default_openai() -> Self {
        Self::OpenAI {
            api_key_env: "OPENAI_API_KEY".to_string(),
            model: "gpt-4o-mini".to_string(),
        }
    }

    pub fn default_anthropic() -> Self {
        Self::Anthropic {
            api_key_env: "ANTHROPIC_API_KEY".to_string(),
            model: "claude-3-5-haiku-latest".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LLMConfig {
    pub backend: Option<LLMBackend>,
    pub max_tokens_per_request: Option<u32>,
    pub temperature: Option<f32>,
}

impl Default for LLMConfig {
    fn default() -> Self {
        Self {
            backend: None,
            max_tokens_per_request: Some(2048),
            temperature: Some(0.7),
        }
    }
}

impl LLMConfig {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_configured(&self) -> bool {
        self.backend.as_ref().map_or(false, |b| b.is_configured())
    }

    pub fn needs_onboarding(&self) -> bool {
        self.backend.is_none()
    }

    pub fn with_backend(mut self, backend: LLMBackend) -> Self {
        self.backend = Some(backend);
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CrumbeezConfig {
    pub llm: LLMConfig,
}

impl CrumbeezConfig {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn needs_onboarding(&self) -> bool {
        self.llm.needs_onboarding()
    }
}
