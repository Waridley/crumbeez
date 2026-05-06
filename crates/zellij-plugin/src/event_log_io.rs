use std::fs;
use std::path::PathBuf;

use tracing::{debug, error, info};

use crumbeez_lib::{EventLog, LogEntry, Summary};

const WASI_EVENT_LOG_PATH: &str = "/data/events.bin";

pub struct EventLogIO;

impl Default for EventLogIO {
    fn default() -> Self {
        Self::new()
    }
}

impl EventLogIO {
    pub fn new() -> Self {
        Self
    }

    pub fn load_into(&mut self, event_log: &mut EventLog) {
        let path = PathBuf::from(WASI_EVENT_LOG_PATH);
        info!(path = ?path, "Loading event log");

        match fs::read(&path) {
            Ok(data) => {
                info!(bytes = data.len(), "Read event log file");
                match EventLog::deserialize(&data) {
                    Ok(loaded) => {
                        info!(count = loaded.total_count(), "Loaded events from disk");
                        *event_log = loaded;
                    }
                    Err(e) => {
                        error!(?e, "Failed to deserialize event log");
                    }
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                info!("No existing event log found, starting fresh");
            }
            Err(e) => {
                error!(?e, path = ?path, "Failed to read event log");
            }
        }
    }

    pub fn save(&mut self, data: Vec<u8>) {
        let path = PathBuf::from(WASI_EVENT_LOG_PATH);
        info!(bytes = data.len(), path = ?path, "Saving event log");

        match fs::write(&path, &data) {
            Ok(_) => {
                debug!(bytes = data.len(), "Event log saved successfully");
            }
            Err(e) => {
                error!(?e, path = ?path, "Failed to write event log");
            }
        }
    }
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

pub fn extract_events_for_llm(event_log: &mut EventLog) -> Option<(Vec<String>, u32)> {
    let unconsumed: Vec<LogEntry> = event_log.unconsumed().cloned().collect();
    if unconsumed.is_empty() {
        return None;
    }

    let count = unconsumed.len() as u32;
    event_log.consume(count as usize);

    let events: Vec<String> = unconsumed.iter().map(|e| e.event.to_string()).collect();

    Some((events, count))
}
