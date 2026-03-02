use crumbeez_lib::CrumbeezConfig;
use std::collections::BTreeMap;
use std::path::PathBuf;
use tracing::{debug, error, info};
use zellij_tile::prelude::run_command;

const CONFIG_FILE_NAME: &str = "config.json";
const CTX_ACTION: &str = "crumbeez_config_action";

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
enum ConfigAction {
    Load,
    Save,
}

pub struct ConfigIO {
    config_dir: Option<PathBuf>,
    config: CrumbeezConfig,
    load_requested: bool,
    save_requested: bool,
}

impl ConfigIO {
    pub fn new() -> Self {
        Self {
            config_dir: None,
            config: CrumbeezConfig::default(),
            load_requested: false,
            save_requested: false,
        }
    }

    pub fn set_config_dir(&mut self, dir: PathBuf) {
        self.config_dir = Some(dir);
    }

    pub fn config(&self) -> &CrumbeezConfig {
        &self.config
    }

    pub fn config_mut(&mut self) -> &mut CrumbeezConfig {
        &mut self.config
    }

    pub fn needs_onboarding(&self) -> bool {
        self.config.needs_onboarding()
    }

    pub fn needs_load(&self) -> bool {
        self.config_dir.is_some() && !self.load_requested
    }

    fn action_context(action: ConfigAction) -> BTreeMap<String, String> {
        let mut ctx = BTreeMap::new();
        ctx.insert(
            CTX_ACTION.to_string(),
            serde_json::to_string(&action).expect("ConfigAction serialization is infallible"),
        );
        ctx
    }

    pub fn request_load(&mut self) {
        if let Some(ref dir) = self.config_dir {
            let path = dir.join(CONFIG_FILE_NAME);
            debug!(path = %path.display(), "Requesting config load via command");

            let path_str = path.to_string_lossy().to_string();
            let args: Vec<&str> = vec!["cat", &path_str];

            run_command(&args, Self::action_context(ConfigAction::Load));
            self.load_requested = true;
        }
    }

    pub fn request_save(&mut self) {
        if let Some(ref dir) = self.config_dir {
            let path = dir.join(CONFIG_FILE_NAME);
            debug!(path = %path.display(), "Requesting config save via command");

            let json = match serde_json::to_string_pretty(&self.config) {
                Ok(j) => j,
                Err(e) => {
                    error!(error = %e, "Failed to serialize config");
                    return;
                }
            };

            let json_encoded = base64_encode(&json);
            let path_str = path.to_string_lossy().to_string();

            let args: Vec<String> = vec![
                "sh".to_string(),
                "-c".to_string(),
                format!("echo {} | base64 -d > {}", json_encoded, path_str),
            ];
            let args_ref: Vec<&str> = args.iter().map(|s| s.as_str()).collect();

            run_command(&args_ref, Self::action_context(ConfigAction::Save));
            self.save_requested = true;
        }
    }

    pub fn handle_command_result(
        &mut self,
        exit_code: Option<i32>,
        stdout: &[u8],
        context: &BTreeMap<String, String>,
    ) -> bool {
        let action: ConfigAction = match context.get(CTX_ACTION) {
            Some(s) => match serde_json::from_str(s) {
                Ok(a) => a,
                Err(_) => return false,
            },
            None => return false,
        };

        match action {
            ConfigAction::Load if self.load_requested => {
                if exit_code == Some(0) {
                    let output = String::from_utf8_lossy(stdout);
                    if !output.is_empty() {
                        match serde_json::from_str::<CrumbeezConfig>(&output) {
                            Ok(config) => {
                                info!("Loaded config successfully");
                                self.config = config;
                            }
                            Err(e) => {
                                error!(error = %e, "Failed to parse config, using defaults");
                            }
                        }
                    }
                } else {
                    info!("Config file not found, will create on first run");
                }
                self.load_requested = false;
                true
            }
            ConfigAction::Save if self.save_requested => {
                if exit_code == Some(0) {
                    info!("Saved config successfully");
                } else {
                    error!(?exit_code, "Failed to save config");
                }
                self.save_requested = false;
                true
            }
            _ => false,
        }
    }
}

impl Default for ConfigIO {
    fn default() -> Self {
        Self::new()
    }
}

fn base64_encode(data: &str) -> String {
    use base64::{engine::general_purpose, Engine as _};
    general_purpose::STANDARD.encode(data.as_bytes())
}
