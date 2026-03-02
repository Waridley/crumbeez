use std::fs;
use std::io::Write;
use std::path::PathBuf;

use chrono::Utc;
use tracing::{debug, info};

pub struct SummaryIO {
    /// WASI-internal path to the summaries directory (e.g. `/host/.crumbeez/summaries`).
    summaries_dir: Option<PathBuf>,
    current_session_file: Option<PathBuf>,
}

impl Default for SummaryIO {
    fn default() -> Self {
        Self::new()
    }
}

impl SummaryIO {
    pub fn new() -> Self {
        Self {
            summaries_dir: None,
            current_session_file: None,
        }
    }

    /// Set the summaries directory using a WASI-internal path.
    ///
    /// The path must be accessible via `std::fs` inside the WASI sandbox —
    /// i.e. rooted under `/host` (project CWD) or `/data` (plugin cache).
    pub fn set_summaries_dir(&mut self, path: PathBuf) {
        info!(path = ?path, "Summaries directory set");
        self.summaries_dir = Some(path);
    }

    pub fn save_summary_text(&mut self, summary: String) {
        let Some(summaries_dir) = &self.summaries_dir else {
            debug!("No summaries directory set for save");
            return;
        };

        if let Err(e) = fs::create_dir_all(summaries_dir) {
            debug!(?e, path = ?summaries_dir, "Failed to create summaries directory");
            return;
        }

        let now = Utc::now();

        if self.current_session_file.is_none() {
            let filename = format!("{}.md", now.format("%Y%m%d_%H%M%S"));
            let filepath = summaries_dir.join(&filename);
            self.current_session_file = Some(filepath.clone());
        }

        let filepath = self.current_session_file.clone().unwrap();

        let timestamp = now.format("%H:%M:%S");
        let section = format!("\n## {}\n\n{}", timestamp, summary);

        match fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&filepath)
        {
            Ok(mut file) => {
                if let Err(e) = file.write_all(section.as_bytes()) {
                    debug!(?e, path = ?filepath, "Failed to append summary");
                } else {
                    info!(path = ?filepath, "Summary appended successfully");
                }
            }
            Err(e) => {
                debug!(?e, path = ?filepath, "Failed to open file for appending");
            }
        }
    }
}
