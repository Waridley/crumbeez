use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use zellij_tile::prelude::*;

use crumbeez_lib::{EventLog, Summary};

const CTX_PURPOSE: &str = "crumbeez_event_log_purpose";

#[derive(Debug, Serialize, Deserialize)]
enum EventLogCommand {
    ReadEventLog,
    WriteEventLog,
}

fn purpose_context(purpose: EventLogCommand) -> BTreeMap<String, String> {
    let mut ctx = BTreeMap::new();
    ctx.insert(
        CTX_PURPOSE.to_string(),
        serde_json::to_string(&purpose).expect("EventLogCommand serialization is infallible"),
    );
    ctx
}

pub struct EventLogIO {
    log_path: Option<PathBuf>,
    pending_write: Option<Vec<u8>>,
}

impl Default for EventLogIO {
    fn default() -> Self {
        Self::new()
    }
}

impl EventLogIO {
    pub fn new() -> Self {
        Self {
            log_path: None,
            pending_write: None,
        }
    }

    pub fn set_log_path(&mut self, path: PathBuf) {
        eprintln!("[crumbeez] Event log path set to: {:?}", path);
        self.log_path = Some(path);
    }

    pub fn load(&mut self, cwd: PathBuf) {
        let Some(log_path) = &self.log_path else {
            eprintln!("[crumbeez] No log path set for load");
            return;
        };
        let path_str = log_path.to_string_lossy().into_owned();
        eprintln!("[crumbeez] Loading event log from: {}", path_str);
        let base64_cmd = format!("if [ -f '{}' ]; then base64 '{}'; fi", path_str, path_str);
        run_command_with_env_variables_and_cwd(
            &["sh", "-c", &base64_cmd],
            BTreeMap::new(),
            cwd,
            purpose_context(EventLogCommand::ReadEventLog),
        );
    }

    pub fn save(&mut self, cwd: PathBuf, data: Vec<u8>) {
        let Some(log_path) = &self.log_path else {
            eprintln!("[crumbeez] No log path set for save");
            return;
        };
        let path_str = log_path.to_string_lossy().into_owned();
        let b64 = base64_encode(&data);
        eprintln!(
            "[crumbeez] Saving {} bytes to {} (b64 len: {})",
            data.len(),
            path_str,
            b64.len()
        );
        let cmd = format!("printf '%s' '{}' | base64 -d > '{}'", b64, path_str);
        self.pending_write = Some(data);
        run_command_with_env_variables_and_cwd(
            &["sh", "-c", &cmd],
            BTreeMap::new(),
            cwd,
            purpose_context(EventLogCommand::WriteEventLog),
        );
    }

    pub fn handle_result(
        &mut self,
        context: &BTreeMap<String, String>,
        stdout: &[u8],
        exit_code: Option<i32>,
        event_log: &mut EventLog,
    ) -> bool {
        let purpose: EventLogCommand = match context.get(CTX_PURPOSE) {
            Some(s) => match serde_json::from_str(s) {
                Ok(p) => p,
                Err(_) => return false,
            },
            None => return false,
        };

        match purpose {
            EventLogCommand::ReadEventLog => {
                eprintln!("[crumbeez] ReadEventLog result: exit_code={:?}", exit_code);
                if exit_code == Some(0) && !stdout.is_empty() {
                    let b64_str = String::from_utf8_lossy(stdout);
                    if let Some(decoded) = base64_decode(&b64_str) {
                        if let Ok(loaded_log) = EventLog::deserialize(&decoded) {
                            eprintln!(
                                "[crumbeez] Loaded {} events from disk",
                                loaded_log.total_count()
                            );
                            *event_log = loaded_log;
                        } else {
                            eprintln!("[crumbeez] Failed to deserialize event log");
                        }
                    } else {
                        eprintln!("[crumbeez] Failed to decode base64");
                    }
                }
                true
            }
            EventLogCommand::WriteEventLog => {
                eprintln!("[crumbeez] WriteEventLog result: exit_code={:?}", exit_code);
                self.pending_write = None;
                true
            }
        }
    }
}

fn base64_encode(data: &[u8]) -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut result = String::new();
    let mut padding = 0;

    for chunk in data.chunks(3) {
        let mut n = 0u32;
        for (i, &byte) in chunk.iter().enumerate() {
            n |= (byte as u32) << (16 - i * 8);
        }
        padding = 3 - chunk.len();
        for i in 0..(4 - padding) {
            let idx = ((n >> (18 - i * 6)) & 0x3F) as usize;
            result.push(ALPHABET[idx] as char);
        }
    }

    for _ in 0..padding {
        result.push('=');
    }

    result
}

fn base64_decode(s: &str) -> Option<Vec<u8>> {
    const DECODE_TABLE: [i8; 128] = [
        -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1,
        -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, 62, -1, -1,
        -1, 63, 52, 53, 54, 55, 56, 57, 58, 59, 60, 61, -1, -1, -1, -1, -1, -1, -1, 0, 1, 2, 3, 4,
        5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, -1, -1, -1,
        -1, -1, -1, 26, 27, 28, 29, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40, 41, 42, 43, 44, 45,
        46, 47, 48, 49, 50, 51, -1, -1, -1, -1, -1,
    ];

    let s = s.trim();
    let s = s.trim_end_matches('=');

    let mut result = Vec::with_capacity(s.len() * 3 / 4);
    let mut buffer = 0u32;
    let mut bits = 0;

    for c in s.chars() {
        let val = if (c as usize) < 128 {
            DECODE_TABLE[c as usize]
        } else {
            -1
        };
        if val < 0 {
            continue;
        }
        buffer = (buffer << 6) | (val as u32);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            result.push((buffer >> bits) as u8);
        }
    }

    Some(result)
}

pub fn generate_summary(event_log: &mut EventLog) -> Option<String> {
    let unconsumed: Vec<_> = event_log.unconsumed().cloned().collect();
    if unconsumed.is_empty() {
        return None;
    }

    let summary = Summary::from_events(unconsumed.into_iter());
    let count = summary.events_consumed;
    event_log.consume(count);

    let mut lines = vec![format!("📊 Summary: {} events processed", count)];
    for (event_type, cnt) in &summary.event_types {
        lines.push(format!("  {}: {}", event_type, cnt));
    }

    Some(lines.join("\n"))
}
