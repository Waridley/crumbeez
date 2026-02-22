use std::collections::VecDeque;

use serde::{Deserialize, Serialize};

use crate::KeystrokeEvent;

const EVENT_LOG_CAPACITY: usize = 10000;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogEntry {
    pub event: KeystrokeEvent,
    pub timestamp_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct LogHeader {
    version: u32,
    consumed_count: u64,
}

#[derive(Debug)]
pub struct EventLog {
    events: VecDeque<LogEntry>,
    consumed_count: usize,
}

impl Default for EventLog {
    fn default() -> Self {
        Self::new()
    }
}

impl EventLog {
    pub fn new() -> Self {
        Self {
            events: VecDeque::with_capacity(EVENT_LOG_CAPACITY),
            consumed_count: 0,
        }
    }

    pub fn append(&mut self, event: KeystrokeEvent, timestamp_ms: u64) {
        if self.events.len() >= EVENT_LOG_CAPACITY {
            if self.consumed_count > 0 {
                let to_remove = self.consumed_count.min(self.events.len());
                for _ in 0..to_remove {
                    self.events.pop_front();
                }
                self.consumed_count = 0;
            } else {
                self.events.pop_front();
            }
        }
        self.events.push_back(LogEntry {
            event,
            timestamp_ms,
        });
    }

    pub fn unconsumed(&self) -> impl Iterator<Item = &LogEntry> {
        self.events.iter().skip(self.consumed_count)
    }

    pub fn unconsumed_count(&self) -> usize {
        self.events.len().saturating_sub(self.consumed_count)
    }

    pub fn total_count(&self) -> usize {
        self.events.len()
    }

    pub fn consume(&mut self, count: usize) {
        self.consumed_count = (self.consumed_count + count).min(self.events.len());
    }

    pub fn compact(&mut self) {
        if self.consumed_count > 0 {
            let to_remove = self.consumed_count.min(self.events.len());
            for _ in 0..to_remove {
                self.events.pop_front();
            }
            self.consumed_count = 0;
        }
    }

    pub fn serialize(&self) -> Result<Vec<u8>, EventLogError> {
        let mut buf = Vec::new();

        let header = LogHeader {
            version: 1,
            consumed_count: self.consumed_count as u64,
        };
        rmp_serde::encode::write(&mut buf, &header)
            .map_err(|e| EventLogError::Serialization(e.to_string()))?;

        for entry in &self.events {
            rmp_serde::encode::write(&mut buf, entry)
                .map_err(|e| EventLogError::Serialization(e.to_string()))?;
        }

        Ok(buf)
    }

    pub fn deserialize(data: &[u8]) -> Result<Self, EventLogError> {
        let mut cursor = std::io::Cursor::new(data);

        let header: LogHeader = rmp_serde::decode::from_read(&mut cursor)
            .map_err(|e| EventLogError::Deserialization(e.to_string()))?;

        if header.version != 1 {
            return Err(EventLogError::InvalidFormat(format!(
                "unsupported version: {}",
                header.version
            )));
        }

        let mut events = VecDeque::new();
        loop {
            match rmp_serde::decode::from_read::<_, LogEntry>(&mut cursor) {
                Ok(entry) => events.push_back(entry),
                Err(e) if e.to_string().contains("unexpected EOF") => break,
                Err(e) => return Err(EventLogError::Deserialization(e.to_string())),
            }
        }

        let consumed_count = (header.consumed_count as usize).min(events.len());

        Ok(Self {
            events,
            consumed_count,
        })
    }
}

#[derive(Debug)]
pub struct Summary {
    pub events_consumed: usize,
    pub event_types: std::collections::HashMap<String, usize>,
}

impl Summary {
    pub fn from_events(entries: impl Iterator<Item = LogEntry>) -> Self {
        let mut events_consumed = 0;
        let mut event_types = std::collections::HashMap::new();

        for entry in entries {
            events_consumed += 1;
            let type_name = match entry.event {
                KeystrokeEvent::TextTyped(_) => "TextTyped",
                KeystrokeEvent::Shortcut(_) => "Shortcut",
                KeystrokeEvent::Navigation(_) => "Navigation",
                KeystrokeEvent::EditControl(_) => "EditControl",
                KeystrokeEvent::Escape => "Escape",
                KeystrokeEvent::FunctionKey(_) => "FunctionKey",
                KeystrokeEvent::SystemKey(_) => "SystemKey",
                KeystrokeEvent::PaneFocused(_) => "PaneFocused",
            };
            *event_types.entry(type_name.to_string()).or_insert(0) += 1;
        }

        Summary {
            events_consumed,
            event_types,
        }
    }
}

#[derive(Debug)]
pub enum EventLogError {
    InvalidFormat(String),
    Serialization(String),
    Deserialization(String),
}

impl std::fmt::Display for EventLogError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidFormat(msg) => write!(f, "Invalid format: {}", msg),
            Self::Serialization(msg) => write!(f, "Serialization error: {}", msg),
            Self::Deserialization(msg) => write!(f, "Deserialization error: {}", msg),
        }
    }
}

impl std::error::Error for EventLogError {}
