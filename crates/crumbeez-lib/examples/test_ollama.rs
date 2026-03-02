use crumbeez_lib::{create_backend, LLMBackend, SummarizationRequest, SummarizationType};
use reqwest::blocking::Client;
use serde::Deserialize;
use std::io::{self, BufRead, Write};

#[derive(Deserialize)]
struct OllamaTagsResponse {
    models: Vec<OllamaModel>,
}

#[derive(Deserialize)]
struct OllamaModel {
    name: String,
}

fn get_available_models(endpoint: &str) -> Result<Vec<String>, String> {
    let client = Client::new();
    let url = format!("{}/api/tags", endpoint);
    let response = client
        .get(&url)
        .send()
        .map_err(|e| format!("Failed to connect to Ollama: {}", e))?;

    if !response.status().is_success() {
        return Err(format!("Ollama returned status {}", response.status()));
    }

    let tags: OllamaTagsResponse = response
        .json()
        .map_err(|e| format!("Failed to parse response: {}", e))?;

    Ok(tags.models.into_iter().map(|m| m.name).collect())
}

fn ensure_model_loaded(client: &Client, endpoint: &str, model: &str) -> Result<(), String> {
    println!("Loading model '{}'...", model);
    let url = format!("{}/api/generate", endpoint);
    let body = serde_json::json!({
        "model": model,
        "prompt": "",
        "keep_alive": "5m",
    });

    let response = client
        .post(&url)
        .json(&body)
        .send()
        .map_err(|e| format!("Failed to load model: {}", e))?;

    if !response.status().is_success() {
        return Err(format!("Failed to load model: HTTP {}", response.status()));
    }

    println!("Model loaded.");
    Ok(())
}

fn select_model_interactively(models: &[String]) -> Result<String, String> {
    if models.is_empty() {
        return Err("No models found. Pull a model with 'ollama pull <model>'".to_string());
    }

    println!("\nAvailable models:");
    for (i, model) in models.iter().enumerate() {
        println!("  {}. {}", i + 1, model);
    }
    print!("\nSelect model (1-{}): ", models.len());
    io::stdout()
        .flush()
        .map_err(|e| format!("IO error: {}", e))?;

    let stdin = io::stdin();
    let mut line = String::new();
    stdin
        .lock()
        .read_line(&mut line)
        .map_err(|e| format!("Failed to read input: {}", e))?;

    let selection: usize = line
        .trim()
        .parse()
        .map_err(|_| "Invalid selection".to_string())?;

    if selection < 1 || selection > models.len() {
        return Err(format!("Selection must be 1-{}", models.len()));
    }

    Ok(models[selection - 1].clone())
}

fn parse_args() -> (String, Option<String>) {
    let args: Vec<String> = std::env::args().collect();
    let mut endpoint = None;
    let mut model = None;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "-e" | "--endpoint" => {
                endpoint = args.get(i + 1).cloned();
                i += 2;
            }
            "-m" | "--model" => {
                model = args.get(i + 1).cloned();
                i += 2;
            }
            arg if !arg.starts_with('-') && endpoint.is_none() => {
                endpoint = Some(arg.to_string());
                i += 1;
            }
            arg if !arg.starts_with('-') && model.is_none() => {
                model = Some(arg.to_string());
                i += 1;
            }
            _ => {
                eprintln!("Unknown argument: {}", args[i]);
                i += 1;
            }
        }
    }

    (
        endpoint.unwrap_or_else(|| "http://localhost:11434".to_string()),
        model,
    )
}

fn main() {
    let (mut endpoint, arg_model) = parse_args();

    if let Ok(val) = std::env::var("OLLAMA_ENDPOINT") {
        endpoint = val;
    }

    let model = arg_model
        .or_else(|| std::env::var("OLLAMA_MODEL").ok())
        .or_else(|| std::env::var("CRUMBEEZ_MODEL").ok())
        .or_else(|| {
            println!("Querying Ollama for available models...");
            match get_available_models(&endpoint) {
                Ok(models) => select_model_interactively(&models).ok(),
                Err(e) => {
                    eprintln!("Could not list models: {}", e);
                    None
                }
            }
        });

    let model = match model {
        Some(m) => m,
        None => {
            eprintln!("No model specified. Use --model, OLLAMA_MODEL, or pull a model to Ollama.");
            std::process::exit(1);
        }
    };

    println!("Testing Ollama at {} with model {}", endpoint, model);

    let client = Client::new();
    if let Err(e) = ensure_model_loaded(&client, &endpoint, &model) {
        eprintln!("{}", e);
        std::process::exit(1);
    }

    let config = LLMBackend::Ollama {
        endpoint: endpoint.clone(),
        model,
    };
    let backend = create_backend(&config);

    println!("Backend: {}", backend.backend_name());
    println!("Available: {}", backend.is_available());

    let events = vec![
        "TextTyped:cargo build".to_string(),
        "PaneFocused:[main] cargo build".to_string(),
        "TextTyped:Compiling crumbeez v0.1.0".to_string(),
        "EditControl:Enter".to_string(),
        "TextTyped:cargo test".to_string(),
        "EditControl:Enter".to_string(),
    ];

    println!("\n--- Testing Leaf Summary ---");
    let request = SummarizationRequest {
        events: events.clone(),
        context: Some("Rust development session".to_string()),
        request_type: SummarizationType::Leaf,
    };

    match backend.summarize(request) {
        Ok(response) => {
            println!("Digest: {}", response.digest);
            println!("Body:\n{}", response.body);
        }
        Err(e) => {
            println!("Error: {}", e);
        }
    }

    println!("\n--- Testing Section Summary ---");
    let child_digests = vec![
        "Built project with cargo".to_string(),
        "Ran tests - all passed".to_string(),
        "Edited src/main.rs".to_string(),
    ];

    let request = SummarizationRequest {
        events: vec![],
        context: None,
        request_type: SummarizationType::Section { child_digests },
    };

    match backend.summarize(request) {
        Ok(response) => {
            println!("Digest: {}", response.digest);
            println!("Body:\n{}", response.body);
        }
        Err(e) => {
            println!("Error: {}", e);
        }
    }

    println!("\n--- Testing Grouping ---");
    let actions = vec![
        "1. Opened terminal, navigated to project".to_string(),
        "2. Ran cargo build".to_string(),
        "3. Fixed compilation error in lib.rs".to_string(),
        "4. Switched to vim to edit config file".to_string(),
        "5. Updated Cargo.toml dependencies".to_string(),
        "6. Ran cargo test".to_string(),
    ];

    let request = SummarizationRequest {
        events: vec![],
        context: None,
        request_type: SummarizationType::Grouping { actions },
    };

    match backend.summarize(request) {
        Ok(response) => {
            println!("Groups:");
            if let Some(groups) = response.groups {
                for g in &groups {
                    println!("  {}-{}: {}", g.start_idx, g.end_idx, g.label);
                }
            }
        }
        Err(e) => {
            println!("Error: {}", e);
        }
    }
}
